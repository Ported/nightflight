//! The engine as a server, and the interface as a browser tab.
//!
//! Two things over one port: a handful of static files, and a WebSocket carrying
//! commands one way and telemetry the other. There is no HTTP framework and no
//! async runtime, because there is exactly one client and nothing here has to
//! scale — a thread per connection is the whole of it.
//!
//! **Why a separate process rather than an embedded webview.** The audio thread
//! has a deadline measured in milliseconds. Keeping the interface in its own
//! process means nothing it does — a slow layout, a garbage collection, a
//! devtools pause, a crash — can starve or take down the thread making the
//! sound. It also means the page reloads without restarting the engine, which is
//! most of the reason for choosing the web at all: the loop is a save and a
//! refresh rather than a rebuild and a relaunch.
//!
//! Note what is *not* shared with the audio thread. `Link` holds this side of
//! both ring buffers and nothing else; the audio thread owns the other ends
//! inside its callback. So putting the `Link` behind a mutex is free — the audio
//! thread can never contend on it, and could not be made to wait even by a
//! connection thread holding it forever.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use engine::Engine;
use engine::seq::{Set, Step};
use engine::telemetry::{self, Command, Description, LaneState, Telemetry};
use host::Link;
use serde::Serialize;
use tungstenite::{Message, accept};

/// Where the browser goes.
const ADDRESS: &str = "127.0.0.1:8730";

/// The piece, and the one thing allowed to change it.
///
/// The engine plays; this holds. Every authoring edit is applied here *and*
/// forwarded to the engine, so what is heard and what would be written down can
/// never disagree — and a save is then only a file write.
///
/// A performance command is not an edit. Muting a lane, moving a fader, taking a
/// macro off its curve: all of that reaches the engine and none of it touches the
/// document, which is the same distinction a mixing desk makes between playing
/// and authoring.
/// What the engine is playing.
///
/// Every editor in this tool wants the same thing — to hear what it is editing
/// and nothing else — so "what is playing" is a small closed set rather than a
/// flag. Opening any editor swaps the engine for one built from just that
/// material; closing it puts the piece back where it was.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    /// The piece as written.
    Piece,
    /// One clip, looping.
    Clip(String),
    /// One lane of the piece, looping — how a patch is heard while it is being
    /// edited inside a clip.
    Lane(usize),
    /// A saved patch with nothing in the piece playing it, on a plain test line.
    Patch(String),
}

impl Stage {
    /// A word and a name for the page, so it can say what it is hearing.
    fn kind(&self) -> &'static str {
        match self {
            Self::Piece => "piece",
            Self::Clip(_) => "clip",
            Self::Lane(_) => "lane",
            Self::Patch(_) => "patch",
        }
    }

    fn label(&self, document: &Set) -> String {
        match self {
            Self::Piece => String::new(),
            Self::Clip(name) | Self::Patch(name) => name.clone(),
            Self::Lane(index) => document
                .lanes
                .get(*index)
                .map_or_else(String::new, |lane| lane.name.clone()),
        }
    }
}

struct Session {
    link: Link,
    document: Set,
    name: String,
    /// Whether the document has changed since it was last written.
    dirty: bool,
    /// What the engine is playing.
    stage: Stage,
    /// For each of the engine's lanes, which of the document's it came from.
    /// While the piece is playing this is the identity; while a clip is being
    /// auditioned it is that clip's lanes.
    ///
    /// The point of holding it here is that *the wire stays in document
    /// coordinates*. The page says "lane 7" and means the seventh lane of the
    /// piece whether or not it is the seventh thing sounding; every translation
    /// happens on this side, once, in the two places below.
    on_stage: Vec<usize>,
    /// Bars the staged material runs for.
    stage_bars: f32,
    /// The bar whatever is staged has reached — the piece or an audition alike.
    /// Kept apart from `resume_at` because they answer different questions: this
    /// one is for rebuilding what is playing without it jumping, and that one is
    /// for going back to the piece after an editor borrowed the engine.
    stage_bar: f32,
    /// The bar the piece had reached when an audition took over, so leaving the
    /// editor puts you back where you were rather than at the beginning.
    resume_at: f32,
}

impl Session {
    /// Apply one command: always to the engine, and to the document too if it
    /// is an edit rather than a performance.
    fn apply(&mut self, command: Command) {
        if let Command::SetParam { lane, param, value } = command
            && let Some(lane) = self.document.lanes.get_mut(lane as usize)
        {
            lane.voicing.set_param(param as usize, value);
            self.dirty = true;
        }
        if let Command::SetLength { lane, length } = command
            && let Some(lane) = self.document.lanes.get_mut(lane as usize)
        {
            lane.length = length;
            self.dirty = true;
        }
        if let Command::SetStep {
            lane,
            step,
            velocity,
            offset,
        } = command
            && let Some(lane) = self.document.lanes.get_mut(lane as usize)
        {
            lane.pattern.set(step as usize, Step::new(velocity, offset));
            self.dirty = true;
        }
        // Performance and authoring alike reach the engine by *engine* index.
        // A lane that is not on stage simply does not: the document keeps the
        // edit, and the engine is not told about a lane it does not have.
        if let Some(command) = self.for_engine(command) {
            self.link.send(command);
        }
    }

    /// The same command in engine coordinates, or nothing if it refers to
    /// something the engine is not currently playing.
    fn for_engine(&self, command: Command) -> Option<Command> {
        let at = |document: usize| self.on_stage.iter().position(|&d| d == document);
        Some(match command {
            // Macros belong to a piece, and an audition has none.
            Command::Macro { .. } if self.stage != Stage::Piece => return None,
            Command::Mute { index, muted } => Command::Mute {
                index: u8::try_from(at(index as usize)?).ok()?,
                muted,
            },
            Command::Level { index, gain } => Command::Level {
                index: u8::try_from(at(index as usize)?).ok()?,
                gain,
            },
            Command::SetParam { lane, param, value } => Command::SetParam {
                lane: u8::try_from(at(lane as usize)?).ok()?,
                param,
                value,
            },
            Command::SetLength { lane, length } => Command::SetLength {
                lane: u8::try_from(at(lane as usize)?).ok()?,
                length,
            },
            Command::SetStep {
                lane,
                step,
                velocity,
                offset,
            } => Command::SetStep {
                lane: u8::try_from(at(lane as usize)?).ok()?,
                step,
                velocity,
                offset,
            },
            other => other,
        })
    }

    /// Put something on the stage.
    ///
    /// The engine for it is built here, off the audio thread, and swapped in
    /// over a ten-millisecond fade. Nothing is torn down in the callback: the
    /// engine it replaces comes back to be dropped on this side.
    fn stage(&mut self, wanted: &Stage) -> Result<(), String> {
        self.stage_at(wanted, None)
    }

    /// Rebuild what is playing, from a document that changed shape.
    ///
    /// Adding a lane or changing a loop's length cannot be a message to the
    /// engine: it has no slot to put a lane in and no room to grow a pattern
    /// into, and making one on the audio thread would mean allocating there. So
    /// the whole engine is built again here and swapped in — the same path an
    /// editor opening takes, which is why that path was worth building.
    ///
    /// It keeps its place: the transport picks up at the bar it had reached, so
    /// adding a hat while a loop runs does not restart the loop.
    fn restage(&mut self) {
        let wanted = self.stage.clone();
        let at = self.stage_bar;
        if let Err(why) = self.stage_at(&wanted, Some(at)) {
            eprintln!("could not rebuild the engine: {why}");
        }
    }

    fn stage_at(&mut self, wanted: &Stage, at: Option<f32>) -> Result<(), String> {
        let (set, on_stage, bar) = match wanted {
            Stage::Piece => (
                self.document.clone(),
                (0..self.document.lanes.len()).collect(),
                self.resume_at,
            ),
            Stage::Clip(name) => {
                let (set, on_stage) = engine::audition::clip(&self.document, name);
                if set.lanes.is_empty() {
                    return Err(format!("no lane belongs to clip {name:?}"));
                }
                (set, on_stage, 0.0)
            }
            Stage::Lane(index) => {
                let (set, on_stage) = engine::audition::lane(&self.document, *index)
                    .ok_or_else(|| format!("there is no lane {index}"))?;
                (set, on_stage, 0.0)
            }
            Stage::Patch(name) => {
                let patch = engine::library::load_patch(name)
                    .map_err(|err| format!("could not read patch {name:?}: {err}"))?;
                let set =
                    engine::audition::patch(patch.voicing, self.document.bpm, self.document.reverb);
                // Nothing in the document is sounding, so no lane of it is on
                // stage and every meter reads silent. Correct: none of them is
                // what you are hearing.
                (set, Vec::new(), 0.0)
            }
        };

        let loop_to = set.length_bars;
        // A rebuild resumes; an editor opening starts at the top. Wrapped to the
        // loop's length so a clip that just shrank does not resume past its end.
        let bar = at.map_or(bar, |at| if loop_to > 0.0 { at % loop_to } else { 0.0 });
        let mut next = Box::new(Engine::new(dsp::SR, set.bpm, set));
        next.start_at(bar);
        if *wanted != Stage::Piece {
            // The loop is what makes it an audition rather than a single pass.
            next.apply(Command::Loop {
                from: 0.0,
                to: loop_to,
                on: true,
            });
        }
        if self.link.load(next) {
            self.stage = wanted.clone();
            self.on_stage = on_stage;
            self.stage_bars = loop_to;
        }
        Ok(())
    }

    /// Give the engine every parameter of one lane, after its patch changed
    /// underneath it.
    ///
    /// One command per parameter rather than one per patch: the engine's whole
    /// authoring vocabulary is "set this parameter of this lane to this", and a
    /// handful of twelve-byte messages is nothing next to inventing a second way
    /// in that the audio thread would have to understand.
    fn push_voicing(&mut self, index: usize) {
        let Some(lane) = self.document.lanes.get(index) else {
            return;
        };
        let values = lane.voicing.values();
        for (param, value) in values.into_iter().enumerate() {
            let Ok(param) = u8::try_from(param) else {
                break;
            };
            let Ok(lane) = u8::try_from(index) else { break };
            if let Some(command) = self.for_engine(Command::SetParam { lane, param, value }) {
                self.link.send(command);
            }
        }
    }

    /// Write a lane's instrument to the library.
    ///
    /// Saving under the name the lane already carries updates **every** lane
    /// playing that patch, here and in the engine — that is what a shared patch
    /// is for. Saving under a new name repoints only this lane, which is how you
    /// fork a kick without disturbing the beat that had the old one.
    fn save_patch(&mut self, index: usize, name: &str) -> Result<String, String> {
        let lane = self
            .document
            .lanes
            .get(index)
            .ok_or_else(|| format!("there is no lane {index}"))?;
        let voicing = lane.voicing;
        let path = engine::library::save_patch(name, voicing).map_err(|err| err.to_string())?;

        // Everyone on this name takes the new values.
        let sharing: Vec<usize> = self
            .document
            .lanes
            .iter()
            .enumerate()
            .filter(|(i, lane)| *i != index && lane.patch.as_deref() == Some(name))
            .map(|(i, _)| i)
            .collect();
        for &other in &sharing {
            self.document.lanes[other].voicing = voicing;
            self.push_voicing(other);
        }
        if self.document.lanes[index].patch.as_deref() != Some(name) {
            self.document.lanes[index].patch = Some(name.to_string());
            // The piece now names a different patch, which is a change to the
            // piece — saving the patch does not save the piece.
            self.dirty = true;
        }
        Ok(path.display().to_string())
    }

    /// Point a lane at a saved patch: a different kick on this beat.
    fn use_patch(&mut self, index: usize, name: &str) -> Result<(), String> {
        let patch = engine::library::load_patch(name)
            .map_err(|err| format!("could not read patch {name:?}: {err}"))?;
        let lane = self
            .document
            .lanes
            .get_mut(index)
            .ok_or_else(|| format!("there is no lane {index}"))?;
        if patch.voicing.instrument() != lane.voicing.instrument() {
            return Err(format!(
                "{name:?} is a {} patch and {} plays {}",
                patch.voicing.instrument(),
                lane.name,
                lane.voicing.instrument()
            ));
        }
        lane.voicing = patch.voicing;
        lane.patch = Some(name.to_string());
        self.dirty = true;
        self.push_voicing(index);
        Ok(())
    }

    /// Write a clip's lanes to the library.
    ///
    /// Saving as new also moves the piece onto the new clip, because the thing
    /// you were editing is the thing you want to keep editing. The old clip is
    /// still in the library, unchanged, for whatever else uses it.
    fn save_clip(&mut self, clip: &str, name: &str) -> Result<String, String> {
        let lanes: Vec<_> = self
            .document
            .lanes
            .iter()
            .filter(|lane| lane.clip == clip)
            .cloned()
            .collect();
        if lanes.is_empty() {
            return Err(format!("no lane belongs to clip {clip:?}"));
        }
        let path = engine::library::save_clip(name, &lanes).map_err(|err| err.to_string())?;
        if name != clip {
            for lane in &mut self.document.lanes {
                if lane.clip == clip {
                    lane.clip = name.to_string();
                }
            }
            self.dirty = true;
            if self.stage == Stage::Clip(clip.to_string()) {
                self.stage = Stage::Clip(name.to_string());
            }
        }
        Ok(path.display().to_string())
    }

    /// Add a lane to a clip, playing a saved patch.
    ///
    /// This is how one drum gets a variation: a beat with two kicks is two
    /// lanes on two patches, alternating in the grid. A lane is one patch
    /// playing one line of steps, so a second sound is a second lane — and it
    /// arrives with its own level, placement, send and mute, which a per-step
    /// patch reference could never have.
    ///
    /// The new lane copies its shape from a sibling where it can: a second kick
    /// in the beat should sit where the first kick sits and last as long,
    /// because what you are changing is the sound, not the arrangement.
    fn add_lane(&mut self, clip: &str, patch: &str, name: Option<&str>) -> Result<(), String> {
        let patch = engine::library::load_patch(patch)
            .map_err(|err| format!("could not read patch {patch:?}: {err}"))?;

        let siblings: Vec<usize> = self
            .document
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.clip == clip)
            .map(|(index, _)| index)
            .collect();
        let Some(&last) = siblings.last() else {
            return Err(format!("no clip named {clip:?}"));
        };
        // Prefer a sibling playing the same instrument — its length and root
        // are the ones that will suit.
        let model = siblings
            .iter()
            .find(|&&i| self.document.lanes[i].voicing.instrument() == patch.voicing.instrument())
            .copied()
            .unwrap_or(last);

        let steps = siblings
            .iter()
            .map(|&i| self.document.lanes[i].pattern.all().len())
            .max()
            .unwrap_or(engine::seq::STEPS_PER_BAR);

        let mut lane = self.document.lanes[model].clone();
        lane.name = self.free_lane_name(name.unwrap_or(&patch.name));
        lane.clip = clip.to_string();
        lane.voicing = patch.voicing;
        lane.patch = Some(patch.name.clone());
        lane.pattern = engine::seq::Pattern::steps(vec![engine::seq::Step::REST; steps]);
        lane.muted = false;
        lane.velocity_scale = 1.0;

        self.document.lanes.insert(last + 1, lane);
        self.dirty = true;
        self.restage();
        Ok(())
    }

    /// A lane name not already in the document, because names are how macros
    /// find their targets and two "kick"s would be one macro's guess.
    fn free_lane_name(&self, wanted: &str) -> String {
        let taken: Vec<&str> = self
            .document
            .lanes
            .iter()
            .map(|lane| lane.name.as_str())
            .collect();
        if !taken.contains(&wanted) {
            return wanted.to_string();
        }
        (2..100)
            .map(|n| format!("{wanted} {n}"))
            .find(|name| !taken.contains(&name.as_str()))
            .unwrap_or_else(|| wanted.to_string())
    }

    /// Take a lane out of the document.
    ///
    /// The last lane of a clip is refused: a clip with no lanes is not an empty
    /// clip, it is a clip that has stopped existing, along with its tab and the
    /// timeline row you were looking at. Deleting a clip should be its own
    /// deliberate act, not what happens when you remove one drum too many.
    fn remove_lane(&mut self, index: usize) -> Result<(), String> {
        let lane = self
            .document
            .lanes
            .get(index)
            .ok_or_else(|| format!("there is no lane {index}"))?;
        let clip = lane.clip.clone();
        let name = lane.name.clone();
        if self
            .document
            .lanes
            .iter()
            .filter(|l| l.clip == clip)
            .count()
            <= 1
        {
            return Err(format!(
                "{name} is the only lane of {clip} — a clip needs one"
            ));
        }

        // A macro pointing at a lane that no longer exists is not an error the
        // engine will notice: wires are resolved by name at load and an unknown
        // name simply finds nothing. Saying so is still worth it.
        let wired: Vec<&str> = self
            .document
            .macros
            .iter()
            .filter(|m| m.mappings.iter().any(|mapping| mapping.lane == name))
            .map(|m| m.name.as_str())
            .collect();
        if !wired.is_empty() {
            println!("removing {name}, which {} mapped", wired.join(", "));
        }

        self.document.lanes.remove(index);
        self.dirty = true;
        self.restage();
        Ok(())
    }

    /// Change how long a clip runs, in bars.
    ///
    /// Every lane of the clip is resized together, which is what a "bars"
    /// control should mean. Lanes can still hold different lengths — a line that
    /// does not divide the bar drifts against it, and that is most of what makes
    /// hypnotic music hypnotic — but that is a per-lane edit, not this.
    fn set_clip_bars(&mut self, clip: &str, bars: f32) -> Result<(), String> {
        if !(0.0..=64.0).contains(&bars) || bars < 0.25 {
            return Err(format!("{bars} bars is outside a quarter bar to 64"));
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let steps = (bars * engine::seq::STEPS_PER_BAR as f32).round() as usize;
        if steps == 0 {
            return Err("that is no steps at all".to_string());
        }
        let mut found = false;
        for lane in &mut self.document.lanes {
            if lane.clip == clip {
                lane.pattern.resize(steps);
                found = true;
            }
        }
        if !found {
            return Err(format!("no clip named {clip:?}"));
        }
        self.dirty = true;
        self.restage();
        Ok(())
    }

    /// The engine's telemetry, put back into document order.
    ///
    /// A lane the audition is not playing reports as silent rather than as
    /// missing, so the page's meters stay where they are instead of shuffling
    /// along by one every time an editor opens.
    fn in_document_order(&self, frame: &Telemetry) -> Telemetry {
        let mut out = *frame;
        if self.stage == Stage::Piece {
            return out;
        }
        out.lanes = [LaneState::default(); telemetry::MAX_PARTS];
        for (engine_index, &document_index) in self.on_stage.iter().enumerate() {
            if let Some(slot) = out.lanes.get_mut(document_index) {
                *slot = frame.lanes[engine_index];
            }
        }
        out.lane_count = u8::try_from(self.document.lanes.len()).unwrap_or(u8::MAX);
        out
    }

    fn save(&mut self) -> std::io::Result<PathBuf> {
        let path = engine::document::directory().join(format!("{}.json", self.name));
        engine::document::save(&path, &self.name, &self.document)?;
        self.dirty = false;
        Ok(path)
    }

    fn stage_message(&self) -> String {
        serde_json::to_string(&Outgoing::Stage(Staged {
            kind: self.stage.kind(),
            name: self.stage.label(&self.document),
            bars: if self.stage == Stage::Piece {
                self.document.length_bars
            } else {
                self.stage_bars
            },
        }))
        .expect("a stage serialises")
    }

    /// The document as it now stands, for a page whose copy is stale.
    fn described_message(&self, saved: Option<String>) -> String {
        serde_json::to_string(&Outgoing::Described {
            dirty: self.dirty,
            saved,
            description: &self.document.describe(),
        })
        .expect("a description serialises")
    }

    /// The document's state after something was written, or changed.
    fn saved_message(&self, saved: Option<String>) -> String {
        serde_json::to_string(&Outgoing::Document {
            dirty: self.dirty,
            saved,
        })
        .expect("a document message serialises")
    }

    fn hello(&self) -> String {
        serde_json::to_string(&Outgoing::Hello {
            set: &self.name,
            device: &self.link.device,
            buffer_frames: self.link.buffer_frames,
            instruments: engine::seq::Voicing::INSTRUMENTS,
            stage: Staged {
                kind: self.stage.kind(),
                name: self.stage.label(&self.document),
                bars: if self.stage == Stage::Piece {
                    self.document.length_bars
                } else {
                    self.stage_bars
                },
            },
            dirty: self.dirty,
            description: &self.document.describe(),
        })
        .expect("a description serialises")
    }
}

/// What the engine is playing, as the page needs to know it.
#[derive(Serialize)]
struct Staged {
    /// "piece", "clip", "lane" or "patch".
    kind: &'static str,
    /// Which one, for the page to show. Empty for the piece.
    name: String,
    /// Bars it loops over, so the page can draw the right length.
    bars: f32,
}

/// What the server sends, tagged the same way commands are — so a message in
/// devtools reads as the thing it does.
#[derive(Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Outgoing<'a> {
    /// Once, on connect: what this is and everything about it that will not
    /// change while it plays.
    Hello {
        set: &'a str,
        device: &'a str,
        buffer_frames: u32,
        /// Every instrument a new patch can be made of.
        instruments: &'static [&'static str],
        /// What the engine is playing right now.
        ///
        /// A page that reloads must not assume the piece is on: the engine
        /// outlives the page — which is most of the reason the interface is a
        /// separate process — so it may well be looping a clip an earlier page
        /// opened. Without this the transport reads against the wrong length and
        /// the first tab click does nothing, because the page thinks it is
        /// already where it is being asked to go.
        stage: Staged,
        /// Whether the piece has unsaved changes.
        dirty: bool,
        #[serde(flatten)]
        description: &'a Description,
    },
    /// Sent whenever the document changes or is written down.
    Document { dirty: bool, saved: Option<String> },
    /// What the engine is playing now.
    Stage(Staged),
    /// The document again, after something changed that the page cannot work out
    /// for itself: a patch swapped in, a clip renamed. Saves re-sending the
    /// hello, which would also reset things the page is in the middle of.
    Described {
        dirty: bool,
        /// Where it was written, if this followed a save.
        saved: Option<String>,
        #[serde(flatten)]
        description: &'a Description,
    },
    /// Everything saved under a name, for the index.
    Library {
        #[serde(flatten)]
        index: &'a engine::library::Index,
    },
    /// Something the page asked for could not be done, in words meant to be
    /// read. Not an error code: there is one user and they are looking at it.
    Complaint { why: String },
    /// About sixty times a second. By reference: this enum exists for one line
    /// of serialisation and is never stored, and a telemetry frame is 448 bytes
    /// against the hello's handful of pointers.
    Telemetry(&'a Telemetry),
}

fn main() {
    let set_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "intro".to_string());

    // A piece on disk wins over the built-in of the same name. That ordering is
    // the point: once a piece has been written down, editing the file is editing
    // the music, and the Rust that first generated it is only its provenance.
    let path = engine::document::directory().join(format!("{set_name}.json"));
    let set = match engine::document::load(&path) {
        Ok(piece) => {
            println!("{} · {}", piece.name, path.display());
            piece.set
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            match engine::sets::by_name(&set_name) {
                Some(set) => {
                    println!("{set_name} · built in, nothing at {}", path.display());
                    set
                }
                None => {
                    eprintln!(
                        "no piece named {set_name:?}; have {:?}",
                        engine::sets::NAMES
                    );
                    std::process::exit(1);
                }
            }
        }
        Err(err) => {
            eprintln!("could not read {}: {err}", path.display());
            std::process::exit(1);
        }
    };

    // Named patches come from the library, overwriting whatever the piece
    // remembers of them: that is what makes a patch shared rather than copied.
    // A name with no file behind it is reported and left alone — a piece that
    // arrived without its library still plays.
    let mut set = set;
    for missing in engine::library::resolve(&mut set) {
        eprintln!("patch not found: {missing}");
    }

    // The engine is given a copy; the original stays here as the document.
    let document = set.clone();
    let link = match host::start(set) {
        Ok(link) => link,
        Err(err) => {
            eprintln!("could not open the audio device: {err}");
            std::process::exit(1);
        }
    };

    println!(
        "{set_name} · {} · {} frames",
        link.device, link.buffer_frames
    );

    let listener = match TcpListener::bind(ADDRESS) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("could not listen on {ADDRESS}: {err}");
            std::process::exit(1);
        }
    };
    println!("open http://{ADDRESS}");

    let session = Arc::new(Mutex::new(Session {
        on_stage: (0..document.lanes.len()).collect(),
        document,
        name: set_name,
        dirty: false,
        stage: Stage::Piece,
        stage_bars: 0.0,
        stage_bar: 0.0,
        resume_at: 0.0,
        link,
    }));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let session = Arc::clone(&session);
        thread::spawn(move || serve(&stream, &session));
    }
}

fn serve(stream: &TcpStream, session: &Mutex<Session>) {
    let mut peek = [0u8; 2048];
    let Ok(read) = stream.peek(&mut peek) else {
        return;
    };
    let head = String::from_utf8_lossy(&peek[..read]).to_lowercase();

    if head.contains("upgrade: websocket") {
        socket(stream, session);
    } else if let Err(err) = file(stream, &head) {
        eprintln!("could not serve a file: {err}");
    }
}

/// Serve one file. A four-line router, because there are three files.
///
/// Read from disk on every request, deliberately: editing the page needs no
/// rebuild and no restart, only a refresh.
fn file(mut stream: &TcpStream, head: &str) -> std::io::Result<()> {
    // Consume the request before answering it. Closing a socket that still has
    // unread bytes in its receive buffer sends a reset rather than a close, and
    // the client's pending read fails before it ever sees the response — which
    // looks exactly like the server crashing.
    let mut sink = [0u8; 2048];
    let mut seen = Vec::new();
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") && seen.len() < 16_384 {
        let read = stream.read(&mut sink)?;
        if read == 0 {
            break;
        }
        seen.extend_from_slice(&sink[..read]);
    }

    let requested = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let requested = if requested == "/" {
        "/index.html"
    } else {
        requested
    };

    // Only from `web/`, and only a bare file name: nothing to traverse with.
    let name = Path::new(requested).file_name().and_then(|n| n.to_str());
    let (status, body, kind) = match name.map(|n| (n, web_root().join(n))) {
        Some((name, file)) if file.is_file() => {
            let kind = match name.rsplit_once('.').map(|(_, extension)| extension) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                _ => "application/octet-stream",
            };
            ("200 OK", std::fs::read(file)?, kind)
        }
        _ => (
            "404 Not Found",
            b"not found".to_vec(),
            "text/plain; charset=utf-8",
        ),
    };

    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    stream.flush()?;
    // Both halves down, so the client sees a clean close and not a reset.
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// Where the page lives: beside the workspace, so `cargo run` finds it from
/// anywhere.
fn web_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("web"))
}

fn socket(stream: &TcpStream, session: &Mutex<Session>) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let Ok(mut socket) = accept(cloned) else {
        return;
    };

    // The hello is built now rather than at startup, so a page that connects
    // after an edit is shown the edit.
    let hello = session.lock().expect("no panics hold this").hello();
    if socket.send(Message::Text(hello.into())).is_err() {
        return;
    }

    // Non-blocking, so reading commands and writing telemetry can share one
    // thread. With one client and a frame every sixteen milliseconds there is
    // nothing to gain from two threads and a lock between them.
    if socket.get_ref().set_nonblocking(true).is_err() {
        return;
    }

    loop {
        let mut reply = Vec::new();
        match socket.read() {
            Ok(Message::Text(text)) => {
                reply = handle(&text, session);
            }
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        for message in reply {
            if socket.send(Message::Text(message.into())).is_err() {
                return;
            }
        }

        // Only the newest frame is of interest: an old one is of no use to a
        // meter, and a slow client simply sees fewer frames.
        let latest = {
            let mut session = session.lock().expect("no panics hold this");
            let mut latest = None;
            while let Ok(frame) = session.link.telemetry.pop() {
                latest = Some(frame);
            }
            // Where the piece had got to, kept only while the piece is what is
            // playing — an audition's bar count says nothing about the score.
            if let Some(frame) = latest {
                session.stage_bar = frame.bar;
                if session.stage == Stage::Piece {
                    session.resume_at = frame.bar;
                }
            }
            // Engines the audio thread has finished with are freed here, where a
            // deadline does not apply.
            session.link.collect();
            latest.map(|frame| session.in_document_order(&frame))
        };
        match latest {
            Some(frame) => match serde_json::to_string(&Outgoing::Telemetry(&frame)) {
                Ok(json) => {
                    if socket.send(Message::Text(json.into())).is_err() {
                        return;
                    }
                }
                Err(err) => eprintln!("could not write telemetry: {err}"),
            },
            // Nothing new: sleep rather than spin a core.
            None => thread::sleep(Duration::from_millis(4)),
        }
    }
}

/// Something went wrong, said in a sentence.
fn complaint(why: &str) -> String {
    eprintln!("{why}");
    serde_json::to_string(&Outgoing::Complaint {
        why: why.to_string(),
    })
    .expect("a complaint serialises")
}

/// One message from the page. Returns something to send back, if anything.
///
/// `save` is handled here rather than being a `Command`, because it is the
/// server's job and the engine has no business knowing the word.
/// A reply, or nothing if it could not be serialised — which would be a bug
/// here, not something the page can do anything about.
fn one(message: serde_json::Result<String>) -> Vec<String> {
    match message {
        Ok(message) => vec![message],
        Err(err) => {
            eprintln!("could not serialise a reply: {err}");
            Vec::new()
        }
    }
}

fn handle(text: &str, session: &Mutex<Session>) -> Vec<String> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("could not read {text:?}: {err}");
            return Vec::new();
        }
    };

    let mut session = session.lock().expect("no panics hold this");
    match value.get("t").and_then(serde_json::Value::as_str) {
        Some("save") => {
            let saved = match session.save() {
                Ok(path) => {
                    println!("saved {}", path.display());
                    Some(path.display().to_string())
                }
                Err(err) => {
                    eprintln!("could not save: {err}");
                    None
                }
            };
            one(serde_json::to_string(&Outgoing::Document {
                dirty: session.dirty,
                saved,
            }))
        }
        // Opening an editor is not a command to the engine, it is a change of
        // what the engine *is*. The material is built into a small piece of its
        // own and swapped in; an empty audition puts the real one back.
        Some("audition") => {
            let text = |key: &str| {
                value
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            };
            let wanted = if let Some(name) = text("clip") {
                Stage::Clip(name)
            } else if let Some(name) = text("patch") {
                Stage::Patch(name)
            } else if let Some(index) = value.get("lane").and_then(serde_json::Value::as_u64) {
                Stage::Lane(index as usize)
            } else {
                Stage::Piece
            };
            match session.stage(&wanted) {
                Ok(()) => vec![session.stage_message()],
                Err(why) => vec![complaint(&why)],
            }
        }
        Some("library") => one(serde_json::to_string(&Outgoing::Library {
            index: &engine::library::index(),
        })),
        Some("save_patch") => {
            let Some(index) = value.get("lane").and_then(serde_json::Value::as_u64) else {
                return vec![complaint("save_patch needs a lane")];
            };
            let index = index as usize;
            // No name given means "save it where it came from".
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    session
                        .document
                        .lanes
                        .get(index)
                        .and_then(|lane| lane.patch.clone())
                });
            let Some(name) = name else {
                return vec![complaint(
                    "this lane has no patch name yet — save it as new",
                )];
            };
            match session.save_patch(index, &name) {
                Ok(path) => {
                    println!("saved patch {name} · {path}");
                    vec![session.described_message(Some(path))]
                }
                Err(why) => vec![complaint(&why)],
            }
        }
        Some("use_patch") => {
            let lane = value.get("lane").and_then(serde_json::Value::as_u64);
            let name = value.get("name").and_then(serde_json::Value::as_str);
            match (lane, name) {
                (Some(lane), Some(name)) => match session.use_patch(lane as usize, name) {
                    Ok(()) => vec![session.described_message(None)],
                    Err(why) => vec![complaint(&why)],
                },
                _ => vec![complaint("use_patch needs a lane and a name")],
            }
        }
        Some("save_clip") => {
            let Some(clip) = value.get("clip").and_then(serde_json::Value::as_str) else {
                return vec![complaint("save_clip needs a clip")];
            };
            let clip = clip.to_string();
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&clip)
                .to_string();
            match session.save_clip(&clip, &name) {
                Ok(path) => {
                    println!("saved clip {name} · {path}");
                    // Saving as new renamed the clip, which renamed the stage:
                    // the page needs both the new document and the new name of
                    // what it is listening to.
                    vec![
                        session.described_message(Some(path)),
                        session.stage_message(),
                    ]
                }
                Err(why) => vec![complaint(&why)],
            }
        }
        // Structural edits: the engine has no slot for a lane it was not built
        // with, so each of these rebuilds it and swaps it in where it was.
        Some("add_lane") => {
            let clip = value.get("clip").and_then(serde_json::Value::as_str);
            let patch = value.get("patch").and_then(serde_json::Value::as_str);
            let name = value.get("name").and_then(serde_json::Value::as_str);
            match (clip, patch) {
                (Some(clip), Some(patch)) => match session.add_lane(clip, patch, name) {
                    Ok(()) => vec![session.described_message(None)],
                    Err(why) => vec![complaint(&why)],
                },
                _ => vec![complaint("add_lane needs a clip and a patch")],
            }
        }
        Some("remove_lane") => match value.get("lane").and_then(serde_json::Value::as_u64) {
            Some(index) => match session.remove_lane(index as usize) {
                Ok(()) => vec![session.described_message(None)],
                Err(why) => vec![complaint(&why)],
            },
            None => vec![complaint("remove_lane needs a lane")],
        },
        Some("set_bars") => {
            let clip = value.get("clip").and_then(serde_json::Value::as_str);
            let bars = value.get("bars").and_then(serde_json::Value::as_f64);
            match (clip, bars) {
                #[allow(clippy::cast_possible_truncation)]
                (Some(clip), Some(bars)) => match session.set_clip_bars(clip, bars as f32) {
                    // The stage message too: the clip is a different length now,
                    // so the scrub bar and the loop are as well.
                    Ok(()) => vec![session.described_message(None), session.stage_message()],
                    Err(why) => vec![complaint(&why)],
                },
                _ => vec![complaint("set_bars needs a clip and a number of bars")],
            }
        }
        Some("new_patch") => {
            let instrument = value.get("instrument").and_then(serde_json::Value::as_str);
            let name = value.get("name").and_then(serde_json::Value::as_str);
            match (instrument, name) {
                (Some(instrument), Some(name)) => {
                    let Some(voicing) = engine::seq::Voicing::fresh(instrument) else {
                        return vec![complaint(&format!("there is no {instrument} instrument"))];
                    };
                    match engine::library::save_patch(name, voicing) {
                        Ok(path) => {
                            println!("new patch {name} · {}", path.display());
                            vec![session.saved_message(Some(path.display().to_string()))]
                        }
                        Err(err) => vec![complaint(&err.to_string())],
                    }
                }
                _ => vec![complaint("new_patch needs an instrument and a name")],
            }
        }
        _ => match serde_json::from_value::<Command>(value) {
            Ok(command) => {
                let was = session.dirty;
                session.apply(command);
                if session.dirty == was {
                    return Vec::new();
                }
                one(serde_json::to_string(&Outgoing::Document {
                    dirty: true,
                    saved: None,
                }))
            }
            Err(err) => {
                eprintln!("could not read {text:?}: {err}");
                Vec::new()
            }
        },
    }
}
