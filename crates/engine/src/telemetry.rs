//! What crosses between the audio thread and the window.
//!
//! Two lock-free queues and two plain `Copy` structs, and that is the whole
//! contract. The window never reads the engine's state and the engine never
//! waits for the window: a hand on a fader becomes a `Command` in a ring buffer,
//! and everything the window draws arrives as a `Telemetry` frame about sixty
//! times a second. Anything that needed a lock between them would eventually
//! make the audio thread wait for a redraw, which is a dropout.
//!
//! Both are fixed-size on purpose — no `Vec`, no `String`, nothing to allocate
//! or free on the audio thread. Parts and macros are addressed by index, and the
//! window keeps its own copy of the names, read once before the engine was
//! handed over.

use serde::{Deserialize, Serialize};

/// More lanes than any set has. Costs 80 bytes each in a telemetry frame.
pub const MAX_PARTS: usize = 16;
/// More macros than a hand can hold anyway.
pub const MAX_MACROS: usize = 8;

/// One lane, as the window sees it.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct LaneState {
    /// Peak of this lane's own contribution since the last frame.
    pub level: f32,
    /// Where it is, in metres. All zeros for a lane that is not placed.
    pub position: [f32; 3],
    /// Whether it is placed at all.
    pub placed: bool,
    /// Whether any of its voices are sounding.
    pub sounding: bool,
    pub muted: bool,
    /// Its fader, as the engine currently has it — macros move this too.
    pub gain: f32,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Telemetry {
    pub bar: f32,
    pub bpm: f32,
    pub voices: u16,
    /// Voices that could not be started because the pool was full.
    pub dropped: u32,
    /// Master peak since the last frame.
    pub peak: f32,
    pub lanes: [LaneState; MAX_PARTS],
    pub lane_count: u8,
    pub macros: [f32; MAX_MACROS],
    pub macro_count: u8,
    /// Share of the callback's deadline used, 0 to 1. Filled in by whoever owns
    /// the audio device, since only it knows how long it had.
    pub load: f32,
    /// Xruns reported by the audio device: the audible kind of failure.
    pub xruns: u32,
    /// Whether the transport is running.
    pub playing: bool,
    /// The bars being looped between, if any.
    pub loop_from: f32,
    pub loop_to: f32,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            bar: 0.0,
            bpm: 0.0,
            voices: 0,
            dropped: 0,
            peak: 0.0,
            lanes: [LaneState::default(); MAX_PARTS],
            lane_count: 0,
            macros: [0.0; MAX_MACROS],
            macro_count: 0,
            load: 0.0,
            xruns: 0,
            playing: true,
            loop_from: 0.0,
            loop_to: 0.0,
        }
    }
}

/// A hand on a control.
///
/// Tagged on the wire, so the browser sends `{"t":"mute","index":0,"muted":true}`
/// and it arrives as this enum with no hand-written parsing in between. The JSON
/// is deliberately the same shape as the Rust, so a message in devtools reads as
/// the thing it does.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Command {
    /// `None` gives the macro back to its curve.
    Macro {
        index: u8,
        value: Option<f32>,
    },
    Mute {
        index: u8,
        muted: bool,
    },
    Level {
        index: u8,
        gain: f32,
    },
    // These are struct variants rather than newtypes because serde's internally
    // tagged representation cannot carry a bare primitive: `Bpm(f32)` compiles
    // and then fails at run time on the first message. Named fields also read
    // better on the wire.
    Bpm {
        value: f32,
    },
    Master {
        value: f32,
    },
    /// Jump the transport to this bar.
    Seek {
        bar: f32,
    },
    /// Run the transport, or stop it.
    Playing {
        value: bool,
    },
    /// Loop between two bars while editing, or stop looping.
    Loop {
        from: f32,
        to: f32,
        on: bool,
    },
    /// Change one of a lane's patch parameters, by its index in the spec.
    SetParam {
        lane: u8,
        param: u8,
        value: f32,
    },
    /// Change one step of a lane. An editor sends these one at a time as cells
    /// are clicked, which is both what an interface naturally produces and the
    /// only shape that needs no allocation on the audio thread: the slot
    /// already exists.
    /// How long every note of a lane rings.
    ///
    /// A lane property rather than a per-note one, which is the model's real
    /// limit: Bach's bass holds a half bar and his top voice an eighth, and
    /// that is two lanes, not two note lengths. Changing it needs no rebuild —
    /// the engine reads it when a note starts.
    SetLength {
        lane: u8,
        length: crate::seq::Length,
    },
    SetStep {
        lane: u8,
        step: u16,
        /// 0 is a rest.
        velocity: f32,
        /// Semitones from the lane's root.
        offset: i8,
    },
}

// ── Describing a set, so an interface can draw it ────────────────────────────

/// Everything about a set that does not change while it plays.
///
/// Sent once, when an interface connects. The window needs the names to label
/// anything, and a timeline needs the patterns and the spans and the curves —
/// which is the gap telemetry alone cannot close: telemetry says where `energy`
/// *is*, and only this says where it is going.
#[derive(Clone, Debug, Serialize)]
pub struct Description {
    pub bpm: f32,
    pub length_bars: f32,
    pub lanes: Vec<LaneDescription>,
    pub macros: Vec<MacroDescription>,
}

#[derive(Clone, Debug, Serialize)]
pub struct LaneDescription {
    pub name: String,
    /// The clip this lane belongs to.
    pub clip: String,
    /// Which instrument plays it: kick, hat, bass, glass, strings.
    pub instrument: &'static str,
    /// The saved patch it plays, if it plays a saved one.
    pub patch: Option<String>,
    /// Whether its steps carry notes worth drawing on a keyboard.
    pub pitched: bool,
    /// How long each note rings: steps of the grid, or seconds.
    pub length: crate::seq::Length,
    pub gain: f32,
    pub send: f32,
    pub muted: bool,
    /// Whether it is placed in space at all.
    pub placed: bool,
    /// Whether the kick ducks it.
    pub ducked: bool,
    /// The loop, as velocity and semitone offset per step. Velocity 0 is a rest.
    pub steps: Vec<(f32, i8)>,
    /// The lane's root note, which the offsets are relative to.
    pub root: f32,
    /// When it plays. Empty means always, which is what a set for jamming wants.
    pub spans: Vec<SpanDescription>,
    /// Whether it has a gate, and how hard it is currently chopping.
    pub gate_depth: Option<f32>,
    /// What this lane's patch can be asked to change: name, range, unit, scale.
    pub params: Vec<dsp::params::ParamSpec>,
    /// Where each of those currently sits, in the same order.
    pub values: Vec<f32>,
}

/// A span, flattened to what a timeline needs to draw it.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SpanDescription {
    pub start: f32,
    pub end: f32,
    /// Where the lane first makes a sound — earlier than `start` if it flies in.
    pub first_bar: f32,
    /// "cut", "fade" or "fly".
    pub enter: &'static str,
    pub leave: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct MacroDescription {
    pub name: String,
    /// Whether a curve is driving it rather than a hand.
    pub automated: bool,
    /// The curve's keyframes, as (bar, value). Empty if it has none.
    pub curve: Vec<(f32, f32)>,
}
