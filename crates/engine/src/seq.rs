//! Patterns and lanes: who plays what, and for how long.
//!
//! A pattern is any number of steps, not necessarily sixteen. That is
//! deliberate: a loop whose length does not divide the bar drifts against it
//! and only lines up again every few bars, which is most of what makes
//! hypnotic techno hypnotic. The Python studio calls it polymeter and gets it
//! from `grid()`; here it is just the length of a slice.

use dsp::inst::{bass, glass, hat, kick, strings};
use dsp::params::{ParamSpec, Parameters};
use dsp::space::Position;
use serde::{Deserialize, Serialize};

use crate::automation::{Flight, Macro, Play, Transition};
use crate::mix::Gate;
use crate::telemetry::{Description, LaneDescription, MacroDescription, SpanDescription};

/// Sixteenths. Every pattern in this tool is read on a sixteenth grid, so the
/// number appears here once rather than as a 16 in four files.
pub const STEPS_PER_BAR: usize = 16;

/// One step of a pattern. Velocity 0 is a rest.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Step {
    pub velocity: f32,
    /// Semitones above the lane's root, for pitched lanes.
    pub offset: i8,
}

impl Step {
    pub const REST: Self = Self {
        velocity: 0.0,
        offset: 0,
    };

    #[must_use]
    pub fn new(velocity: f32, offset: i8) -> Self {
        Self { velocity, offset }
    }
}

/// A looping sequence of steps. Built once, when a set is loaded.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    steps: Vec<Step>,
}

impl Pattern {
    /// From a drum grid, the notation the Python studio uses: `X` accent,
    /// `x` hit, `o` ghost, `.` rest. Spaces and bar lines are ignored.
    ///
    /// # Panics
    /// If the grid contains a character that is not one of those.
    #[must_use]
    pub fn grid(pattern: &str) -> Self {
        let steps = pattern
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '|')
            .map(|c| match c {
                'X' => Step::new(1.0, 0),
                'x' => Step::new(0.7, 0),
                'o' => Step::new(0.4, 0),
                '.' => Step::REST,
                other => panic!("{other:?} is not a grid character (X x o .)"),
            })
            .collect();
        Self { steps }
    }

    #[must_use]
    pub fn steps(steps: Vec<Step>) -> Self {
        Self { steps }
    }

    /// The step at this position of the running transport, looping at the
    /// pattern's own length.
    #[must_use]
    pub fn at(&self, step: u64) -> Step {
        if self.steps.is_empty() {
            return Step::REST;
        }
        self.steps[(step % self.steps.len() as u64) as usize]
    }

    /// Change one step. Allocation-free: the slot already exists, which is why
    /// an editor sends one step rather than a whole clip.
    pub fn set(&mut self, index: usize, step: Step) {
        if let Some(slot) = self.steps.get_mut(index) {
            *slot = step;
        }
    }

    /// Grow or trim the loop, in steps.
    ///
    /// Growing fills with rests rather than repeating what is there: a bar you
    /// just added should be empty, so you can hear what you put in it. Trimming
    /// drops the tail, and the steps that went are gone — which is why this is a
    /// structural edit the document records rather than a view setting.
    pub fn resize(&mut self, steps: usize) {
        self.steps.resize(steps, Step::REST);
    }

    /// Every step, for an interface that wants to draw the loop.
    #[must_use]
    pub fn all(&self) -> &[Step] {
        &self.steps
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// How long a note lasts, and the distinction matters musically.
///
/// A drum's ring is a physical fact: a kick drum does not decay faster because
/// the record is faster, so its length is in **seconds**. A note that should
/// last "a sixteenth" is in **steps**, and follows the tempo. Getting this
/// wrong is what cut the Python beat sketch's kick to 0.12 s and made it feel
/// wimpy — it had been given a one-step length.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Length {
    Seconds(f32),
    Steps(f32),
}

impl Length {
    #[must_use]
    pub fn seconds(self, samples_per_step: f64, sr: f32) -> f32 {
        match self {
            Self::Seconds(s) => s,
            Self::Steps(n) => n * (samples_per_step / f64::from(sr)) as f32,
        }
    }
}

impl Voicing {
    /// What this patch's parameters are: name, range, unit and scale. An
    /// interface draws faders from this without knowing one instrument from
    /// another.
    #[must_use]
    pub fn spec(self) -> &'static [ParamSpec] {
        match self {
            Self::Kick(_) => kick::Params::SPEC,
            Self::Hat(_) => hat::Params::SPEC,
            Self::Bass(_) => bass::Params::SPEC,
            Self::Glass(_) => glass::Params::SPEC,
            Self::Strings(_) => strings::Params::SPEC,
        }
    }

    /// Every parameter's current value, in the order of the spec.
    #[must_use]
    pub fn values(self) -> Vec<f32> {
        (0..self.spec().len()).map(|i| self.param(i)).collect()
    }

    #[must_use]
    pub fn param(self, index: usize) -> f32 {
        match self {
            Self::Kick(p) => p.get(index),
            Self::Hat(p) => p.get(index),
            Self::Bass(p) => p.get(index),
            Self::Glass(p) => p.get(index),
            Self::Strings(p) => p.get(index),
        }
    }

    /// Change one parameter. Clamped to its declared range by the setter, so
    /// nothing an interface sends can make an instrument misbehave.
    pub fn set_param(&mut self, index: usize, value: f32) {
        match self {
            Self::Kick(p) => p.set(index, value),
            Self::Hat(p) => p.set(index, value),
            Self::Bass(p) => p.set(index, value),
            Self::Glass(p) => p.set(index, value),
            Self::Strings(p) => p.set(index, value),
        }
    }

    /// Every instrument there is, for an interface offering to make a new patch.
    pub const INSTRUMENTS: &'static [&'static str] =
        &["kick", "hat", "bass", "glass", "strings"];

    /// A new patch of a named instrument, at its defaults.
    #[must_use]
    pub fn fresh(instrument: &str) -> Option<Self> {
        Some(match instrument {
            "kick" => Self::Kick(kick::Params::default()),
            "hat" => Self::Hat(hat::Params::default()),
            "bass" => Self::Bass(bass::Params::default()),
            "glass" => Self::Glass(glass::Params::default()),
            "strings" => Self::Strings(strings::Params::default()),
            _ => return None,
        })
    }

    /// Whether the notes are something you write, or something the instrument
    /// makes for itself.
    ///
    /// A kick has a pitch and it is not a note: it is a sweep from high to low
    /// that *is* the sound, and moving the whole thing up a tone is a patch
    /// edit, not a melody. A hat has no pitch worth naming. Everything else
    /// plays what the steps say, and wants a piano roll rather than a row of
    /// boxes.
    #[must_use]
    pub fn pitched(self) -> bool {
        match self {
            Self::Kick(_) | Self::Hat(_) => false,
            Self::Bass(_) | Self::Glass(_) | Self::Strings(_) => true,
        }
    }

    /// Which instrument this is, for an interface to label and for a patch to
    /// be filed under.
    #[must_use]
    pub fn instrument(self) -> &'static str {
        match self {
            Self::Kick(_) => "kick",
            Self::Hat(_) => "hat",
            Self::Bass(_) => "bass",
            Self::Glass(_) => "glass",
            Self::Strings(_) => "strings",
        }
    }
}

/// Which instrument a lane plays, and how it is set up.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "instrument", rename_all = "snake_case")]
pub enum Voicing {
    Kick(kick::Params),
    Hat(hat::Params),
    Bass(bass::Params),
    Glass(glass::Params),
    Strings(strings::Params),
}

/// A lane: one instrument, one pattern, one fader.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lane {
    pub name: String,
    /// Which clip this lane belongs to. Lanes of one clip are edited together
    /// and drawn as one row on the timeline: "kick" is a lane of "beat".
    pub clip: String,
    pub voicing: Voicing,
    /// The name of the patch this lane plays, if it plays a saved one.
    ///
    /// The values are still here in `voicing` as well, so a piece file is whole
    /// on its own and plays with no library present. The library wins when both
    /// exist: on load a named patch is read from `patches/` and overwrites what
    /// the piece happens to remember, which is what makes changing a patch
    /// change every lane using it rather than one. Think of the copy in the
    /// piece as a cache, written every time the piece is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    pub pattern: Pattern,
    /// Linear level, relative to the other lanes.
    pub gain: f32,
    pub length: Length,
    /// MIDI note the pattern's offsets are relative to.
    pub root: f32,
    /// Where it sits around the head.
    pub home: Home,
    /// How much of this lane goes to the shared reverb, 0 for dry.
    pub send: f32,
    /// A rhythmic chop on this lane's level, if any.
    pub gate: Option<Gate>,
    /// When this lane plays, in bars. Empty means always — which is what a
    /// looping set for jamming wants, and a score fills in.
    pub spans: Vec<Play>,
    /// A multiplier on the velocity of notes started from now on. Macros ride
    /// this; nothing else touches it.
    pub velocity_scale: f32,
    /// Whether the kick ducks this lane (the sidechain pump).
    pub ducked: bool,
    pub muted: bool,
}

/// Where a lane sits, described rather than baked.
///
/// The Python scene stores a path as a list of keyframes, computed once by the
/// composition. Live, the description has to survive: you want to speed an
/// orbit up or throw a source somewhere while it plays, and you cannot do that
/// to a list of points. So a home is evaluated at the musical time it is needed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Home {
    /// Not placed: mixed to the centre, untouched. Right for kick and bass —
    /// low frequencies barely localise, and a head filter would only colour
    /// them.
    Centre,
    /// A circle at head height, one lap every `bars_per_lap`, clockwise seen
    /// from above (front, right, behind, left). `phase` is in laps, so two
    /// sources half a lap apart chase each other.
    ///
    /// The angle comes from an accumulated lap count, not from
    /// `bar / bars_per_lap`. That matters the moment the rate can change: with
    /// the angle computed from the bar, doubling the speed at bar 10 also
    /// doubles the accumulated angle, and the source teleports to the other side
    /// of the head. Integrating the rate instead means a change in speed is only
    /// ever a change in speed.
    Orbit {
        radius: f32,
        bars_per_lap: f32,
        phase: f32,
        /// Metres above or below the ears, held. Spreading several voices
        /// vertically keeps them from piling up in one place.
        elevation: f32,
        /// Metres of rise and fall on top of that, on its own cycle: a circle
        /// that also breathes vertically stops sounding like a machine.
        height: f32,
        height_bars: f32,
    },
}

impl Home {
    /// Laps per bar, for integrating the orbit's angle.
    #[must_use]
    pub fn laps_per_bar(self) -> f64 {
        match self {
            Self::Centre => 0.0,
            Self::Orbit { bars_per_lap, .. } => 1.0 / f64::from(bars_per_lap).max(0.01),
        }
    }

    /// Where the source is, given how many laps it has turned and what bar it
    /// is. `None` means it is not placed at all.
    #[must_use]
    pub fn at(self, laps: f64, bar: f64) -> Option<Position> {
        match self {
            Self::Centre => None,
            Self::Orbit {
                radius,
                phase,
                elevation,
                height,
                height_bars,
                ..
            } => {
                let angle = std::f64::consts::TAU * (laps + f64::from(phase));
                let rise = std::f64::consts::TAU * bar / f64::from(height_bars);
                Some(Position::new(
                    radius * angle.sin() as f32,
                    elevation + height * rise.sin() as f32,
                    -radius * angle.cos() as f32,
                ))
            }
        }
    }
}

/// The shared reverb's settings, as the scene JSON describes them.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ReverbSettings {
    /// Seconds for the tail to fall 60 dB at low frequencies: bedroom ~0.5,
    /// cathedral 6-10.
    pub rt60: f32,
    /// How much faster the highs die. 0 leaves the tail as bright as the
    /// source; higher is warmer. Gentler per unit than the Python's damping —
    /// about 1.6 here for what 1.0 does there.
    pub damping: f32,
    /// Seconds of silence before the tail arrives: the ear's cue for how big
    /// the space is.
    pub predelay: f32,
    /// Hz. Nothing below this reaches the reverb, because reverberant sub-bass
    /// is only mud.
    pub lowcut: f32,
}

/// Everything the engine needs to play a piece — and, because every field of it
/// is data, the document itself. A piece on disk deserialises straight into this
/// and the engine plays it; there is no second set of types to drift.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Set {
    /// Beats a minute.
    pub bpm: f32,
    pub lanes: Vec<Lane>,
    pub reverb: Option<ReverbSettings>,
    /// The conductor's faders.
    pub macros: Vec<Macro>,
    /// How long the piece is, in bars. A looping set for jamming has no real
    /// end; this is then just how far the scrub bar reaches.
    pub length_bars: f32,
}

impl Lane {
    /// How loud the score says this lane is at `bar`, or `None` if it is not
    /// playing. A lane with no spans always plays.
    #[must_use]
    pub fn scored(&self, bar: f64) -> Option<f32> {
        if self.spans.is_empty() {
            return Some(1.0);
        }
        self.spans
            .iter()
            .filter_map(|s| s.level(bar))
            .reduce(f32::max)
    }

    /// If this lane is in the air right now, the flight and the bar it lands on.
    #[must_use]
    pub fn flying(&self, bar: f64) -> Option<(Flight, f32)> {
        self.spans.iter().find_map(|s| s.approach(bar))
    }

    /// Whether any of this lane's spans ever puts it in the air. Decided at
    /// load, because it settles whether the lane needs a delay line.
    #[must_use]
    pub fn ever_flies(&self) -> bool {
        self.spans
            .iter()
            .any(|s| matches!(s.enter, Transition::Fly(_)))
    }
}

impl Set {
    /// Everything about this piece that an interface needs in order to draw it.
    ///
    /// Telemetry says where a macro *is*; only this says where it is going,
    /// which is the difference between a fader and a timeline.
    #[must_use]
    pub fn describe(&self) -> Description {
        Description {
            bpm: self.bpm,
            length_bars: self.length_bars,
            lanes: self
                .lanes
                .iter()
                .map(|lane| LaneDescription {
                    name: lane.name.clone(),
                    clip: lane.clip.clone(),
                    instrument: lane.voicing.instrument(),
                    patch: lane.patch.clone(),
                    pitched: lane.voicing.pitched(),
                    length: lane.length,
                    gain: lane.gain,
                    send: lane.send,
                    muted: lane.muted,
                    // A lane is placed if it has somewhere to be, or ever flies
                    // to one.
                    placed: !matches!(lane.home, Home::Centre) || lane.ever_flies(),
                    ducked: lane.ducked,
                    steps: lane
                        .pattern
                        .all()
                        .iter()
                        .map(|step| (step.velocity, step.offset))
                        .collect(),
                    root: lane.root,
                    spans: lane
                        .spans
                        .iter()
                        .map(|play| SpanDescription {
                            start: play.start,
                            end: play.end,
                            first_bar: play.first_bar(),
                            enter: play.enter.name(),
                            leave: play.leave.name(),
                        })
                        .collect(),
                    gate_depth: lane.gate.as_ref().map(|gate| gate.depth),
                    params: lane.voicing.spec().to_vec(),
                    values: lane.voicing.values(),
                })
                .collect(),
            macros: self
                .macros
                .iter()
                .map(|m| MacroDescription {
                    name: m.name.clone(),
                    automated: m.manual.is_none(),
                    curve: m
                        .automation
                        .as_ref()
                        .map(|curve| curve.points().to_vec())
                        .unwrap_or_default(),
                })
                .collect(),
        }
    }
}
