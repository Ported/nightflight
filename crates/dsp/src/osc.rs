//! Oscillators.
//!
//! Phase is kept wrapped to `[0, 1)` rather than accumulating, so f32's 24-bit
//! mantissa never runs out on a note held for minutes.

use std::f32::consts::TAU;

/// A phase accumulator. `tick` returns the phase *before* advancing, so the
/// first sample of a note is phase 0 (or wherever it was started).
#[derive(Clone, Copy, Debug, Default)]
pub struct Phasor {
    phase: f32,
}

impl Phasor {
    #[must_use]
    pub fn new(phase: f32) -> Self {
        Self { phase }
    }

    /// Advance by one sample at `freq` Hz, returning the phase for this sample.
    pub fn tick(&mut self, freq: f32, sr: f32) -> f32 {
        let now = self.phase;
        self.phase += freq / sr;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        now
    }
}

/// A sine at the given phase.
#[must_use]
pub fn sine(phase: f32) -> f32 {
    (TAU * phase).sin()
}

/// A band-limited sawtooth: the naive ramp with its jump smoothed (polyBLEP).
///
/// `dt` is the phase step, `freq / sr`. Without this correction the jump from
/// +1 to -1 is infinitely steep and folds energy back down the spectrum as an
/// out-of-tune whistle; the correction subtracts a small polynomial either side
/// of the jump, which is what a band-limited step actually looks like.
#[must_use]
pub fn saw(phase: f32, dt: f32) -> f32 {
    let mut out = 2.0 * phase - 1.0;
    if phase < dt {
        let t = phase / dt;
        out -= t + t - t * t - 1.0;
    } else if phase > 1.0 - dt {
        let t = (phase - 1.0) / dt;
        out -= t * t + t + t + 1.0;
    }
    out
}
