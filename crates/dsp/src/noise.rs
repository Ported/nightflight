//! White noise.
//!
//! An xorshift generator, not numpy's: nothing here needs to match the Python
//! renders sample for sample, and a drum machine's noise only has to be
//! uniform, cheap and free-running.

/// Uniform white noise in `[-1, 1)`.
#[derive(Clone, Copy, Debug)]
pub struct Noise {
    state: u32,
}

impl Noise {
    #[must_use]
    pub fn new(seed: u32) -> Self {
        // Any non-zero state; xorshift is stuck at zero.
        Self {
            state: seed | 0x9E37_79B9,
        }
    }

    pub fn tick(&mut self) -> f32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        // Top 24 bits, scaled: exactly representable in f32.
        ((self.state >> 8) as f32) * (2.0 / 16_777_216.0) - 1.0
    }
}

impl Default for Noise {
    fn default() -> Self {
        Self::new(1)
    }
}
