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
use engine::seq::Set;

mod watch;
use engine::telemetry::{self, Command, Description, LaneState, Telemetry};
use host::Link;
use serde::Serialize;
use tungstenite::{Message, accept};

/// Where the browser goes.
const ADDRESS: &str = "127.0.0.1:8730";

/// The piece, and the one thing allowed to change it.
///
/// The engine plays; this holds a copy of what it was given.
///
/// Read-only, now that authoring lives in `nf`. The document is here so the
/// server can build an audition from one clip, and so telemetry can be put
/// back into document order — not so anything can change it.
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
    /// Bumped whenever something changed that a page cannot have asked for: a
    /// file was written, a piece reloaded. Each connection remembers the
    /// number it last drew and sends itself a fresh hello when it moves.
    ///
    /// A counter rather than a message queue because there can be more than
    /// one page, they can connect at any time, and none of them needs to know
    /// *what* changed — only that what they are showing is old.
    generation: u64,
    /// Why the last reload did not happen, for the page to show. A tab file
    /// with a typo in it is the normal case here, not an exceptional one.
    trouble: Option<String>,
}

impl Session {
    /// Send one command to the engine.
    ///
    /// **Nothing here writes a document.** Everything the page can send is a
    /// performance: a fader moved, a lane muted, a macro taken off its curve,
    /// a patch parameter turned while it plays. None of it is written down,
    /// and reloading the piece puts it all back — which is the same thing a
    /// mixing desk means by the distinction, and the reason moving a fader for
    /// an hour leaves a piece unmodified.
    ///
    /// Authoring lives in `nf` and in the files it writes. The page used to do
    /// both and the cost was a mutable document, a dirty flag, a save path and
    /// eight message types; deleting the editors deleted all of it.
    fn apply(&mut self, command: Command) {
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

    /// Read the current piece again and carry on from where it was.
    ///
    /// The difference from `load` is the bar: a different piece has no
    /// sensible place to resume, and the same piece edited under you has
    /// exactly one — the one you are on. Whatever was staged stays staged, so
    /// editing a clip while auditioning it keeps auditioning it.
    ///
    /// # Errors
    /// If the file no longer reads. Nothing is swapped in that case: a typo
    /// must not stop the music.
    fn reload(&mut self) -> Result<(), String> {
        let name = self.name.clone();
        let (_, mut set) = engine::document::find(&name).map_err(|err| err.to_string())?;
        engine::library::resolve(&mut set);
        let previous = std::mem::replace(&mut self.document, set);
        let stage = self.stage.clone();
        let at = self.stage_bar;
        match self.stage_at(&stage, Some(at)) {
            Ok(()) => {
                self.trouble = None;
                Ok(())
            }
            Err(why) => {
                self.document = previous;
                Err(why)
            }
        }
    }

    /// Put a different piece on.
    ///
    /// Everything about the session changes — the lanes, the macros, the
    /// clips, the room, the tempo — so the page is sent a fresh hello and
    /// rebuilds from scratch. That is not laziness: a piece change is exactly
    /// the event the hello was written for, and inventing a narrower message
    /// would be inventing a second way to say the same thing.
    ///
    /// It starts at the top. There is no sensible bar to resume at when the
    /// piece has a different number of them, and whether it was playing
    /// carries over because that is a property of the transport, not the
    /// music.
    ///
    /// # Errors
    /// If there is no piece of that name, or it cannot be read. The engine
    /// keeps playing what it had — a typo should not stop the music.
    fn load(&mut self, what: &str) -> Result<(), String> {
        let (name, mut set) = engine::document::find(what).map_err(|err| err.to_string())?;
        for missing in engine::library::resolve(&mut set) {
            eprintln!("patch not found: {missing}");
        }
        self.document = set;
        self.name = name;
        self.resume_at = 0.0;
        self.stage_bar = 0.0;
        self.stage_at(&Stage::Piece, Some(0.0))
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

    fn hello(&self) -> String {
        serde_json::to_string(&Outgoing::Hello {
            set: &self.name,
            device: &self.link.device,
            buffer_frames: self.link.buffer_frames,
            stage: Staged {
                kind: self.stage.kind(),
                name: self.stage.label(&self.document),
                bars: if self.stage == Stage::Piece {
                    self.document.length_bars
                } else {
                    self.stage_bars
                },
            },
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
        /// What the engine is playing right now.
        ///
        /// A page that reloads must not assume the piece is on: the engine
        /// outlives the page — which is most of the reason the interface is a
        /// separate process — so it may well be looping a clip an earlier page
        /// opened. Without this the transport reads against the wrong length and
        /// the first tab click does nothing, because the page thinks it is
        /// already where it is being asked to go.
        stage: Staged,
        #[serde(flatten)]
        description: &'a Description,
    },
    /// What the engine is playing now.
    Stage(Staged),
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
    let (set_name, set) = match engine::document::find(&set_name) {
        Ok(found) => found,
        Err(err) => {
            eprintln!("{err}");
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
        stage: Stage::Piece,
        generation: 0,
        trouble: None,
        stage_bars: 0.0,
        stage_bar: 0.0,
        resume_at: 0.0,
        link,
    }));
    // From here a tab file being written is a thing that happens to the music.
    watch::spawn(Arc::clone(&session));

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

    // What this connection has drawn. A page that connects mid-session starts
    // level with whatever has already happened.
    let mut drawn = session.lock().expect("no panics hold this").generation;

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
        // Something changed on disk: the piece reloaded, or the library did.
        // A hello is the message that means "here is what this is", and it is
        // the right one whether a lane appeared or a whole piece was rewritten.
        {
            let session = session.lock().expect("no panics hold this");
            if session.generation != drawn {
                drawn = session.generation;
                reply.push(session.hello());
                if let Some(why) = &session.trouble {
                    reply.push(complaint_text(why));
                }
            }
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

/// Something went wrong, said in a sentence, without printing it again —
/// the watcher has already said it once and repeats it to every page that
/// connects.
fn complaint_text(why: &str) -> String {
    serde_json::to_string(&Outgoing::Complaint {
        why: why.to_string(),
    })
    .expect("a complaint serialises")
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
        // A different piece. Everything about the session changes, so the
        // answer is a fresh hello — the message that already means "here is
        // what this is".
        Some("load") => {
            let Some(name) = value.get("piece").and_then(serde_json::Value::as_str) else {
                return vec![complaint("load needs a piece")];
            };
            match session.load(name) {
                Ok(()) => {
                    println!("playing {name}");
                    vec![session.hello()]
                }
                Err(why) => vec![complaint(&why)],
            }
        }
        Some("library") => one(serde_json::to_string(&Outgoing::Library {
            index: &engine::library::index(),
        })),
        // Everything else is a performance command: it reaches the engine and
        // nothing is written down, so there is nothing to reply.
        _ => match serde_json::from_value::<Command>(value) {
            Ok(command) => {
                session.apply(command);
                Vec::new()
            }
            Err(err) => {
                eprintln!("could not read {text:?}: {err}");
                Vec::new()
            }
        },
    }
}
