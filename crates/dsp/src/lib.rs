//! Building blocks and instruments: f32, block-based, no I/O and no allocation.
//!
//! The difference from the Python studio this grew out of: numpy thinks in
//! whole arrays, so an instrument there is a function that sees a note's entire
//! future. Here an instrument is a small state machine, advanced one sample at
//! a time, which can be started in the middle of a block and never knows when
//! it will be asked for the next one. Same recipes, different shape.

pub mod env;
pub mod filter;
pub mod inst;
pub mod noise;
pub mod osc;
pub mod params;
pub mod phaser;
pub mod reverb;
pub mod shape;
pub mod smooth;
pub mod space;

/// Samples per second. Fixed: the app refuses a device that won't run at 48 kHz.
pub const SAMPLE_RATE: u32 = 48_000;
/// The same, where a float is wanted.
pub const SR: f32 = SAMPLE_RATE as f32;

/// MIDI note number to frequency. 69 is A4, 440 Hz; 31 is G1, about 49 Hz.
#[must_use]
pub fn hz(pitch: f32) -> f32 {
    440.0 * ((pitch - 69.0) / 12.0).exp2()
}

/// One voice of an instrument: a note, playing.
///
/// `add` mixes this voice's next `out.len()` samples *into* `out` and advances
/// it. The engine splits its blocks at note boundaries, so a voice always
/// starts at index 0 of the first slice it is handed — which is why there is no
/// start-offset argument here.
pub trait Voice {
    fn add(&mut self, out: &mut [f32]);
    /// True once the voice has rung out and its slot can be reused.
    fn finished(&self) -> bool;
}
