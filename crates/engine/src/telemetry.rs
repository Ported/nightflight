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

/// More parts than any set has. Costs 80 bytes each in a telemetry frame.
pub const MAX_PARTS: usize = 16;
/// More macros than a hand can hold anyway.
pub const MAX_MACROS: usize = 8;

/// One part, as the window sees it.
#[derive(Clone, Copy, Debug, Default)]
pub struct PartState {
    /// Peak of this part's own contribution since the last frame.
    pub level: f32,
    /// Where it is, in metres. All zeros for a part that is not placed.
    pub position: [f32; 3],
    /// Whether it is placed at all.
    pub placed: bool,
    /// Whether any of its voices are sounding.
    pub sounding: bool,
    pub muted: bool,
    /// Its fader, as the engine currently has it — macros move this too.
    pub gain: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Telemetry {
    pub bar: f32,
    pub bpm: f32,
    pub voices: u16,
    /// Voices that could not be started because the pool was full.
    pub dropped: u32,
    /// Master peak since the last frame.
    pub peak: f32,
    pub parts: [PartState; MAX_PARTS],
    pub part_count: u8,
    pub macros: [f32; MAX_MACROS],
    pub macro_count: u8,
    /// Share of the callback's deadline used, 0 to 1. Filled in by whoever owns
    /// the audio device, since only it knows how long it had.
    pub load: f32,
    /// Xruns reported by the audio device: the audible kind of failure.
    pub xruns: u32,
    /// Whether the transport is running.
    pub playing: bool,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            bar: 0.0,
            bpm: 0.0,
            voices: 0,
            dropped: 0,
            peak: 0.0,
            parts: [PartState::default(); MAX_PARTS],
            part_count: 0,
            macros: [0.0; MAX_MACROS],
            macro_count: 0,
            load: 0.0,
            xruns: 0,
            playing: true,
        }
    }
}

/// A hand on a control.
#[derive(Clone, Copy, Debug)]
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
    Bpm(f32),
    Master(f32),
    /// Jump the transport to this bar.
    Seek(f32),
    /// Run the transport, or stop it.
    Playing(bool),
}
