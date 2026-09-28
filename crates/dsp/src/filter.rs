//! One filter, three shapes.
//!
//! A topology-preserving-transform state-variable filter, the same one the
//! Python studio uses. It is worth having exactly one: its cutoff can move
//! every single sample without the stepping noise ("zipper") that a naive
//! filter gives, and low, band and high all fall out of the same two
//! integrator states, so a sweeping lowpass and a fixed bandpass are the same
//! twelve lines of code.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Low,
    Band,
    High,
}

#[derive(Clone, Copy, Debug)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    /// A filter set to `cutoff` Hz with `res` resonance (0 none, just under 1
    /// self-oscillating).
    #[must_use]
    pub fn new(sr: f32, cutoff: f32, res: f32) -> Self {
        let mut f = Self {
            ic1: 0.0,
            ic2: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            k: 0.0,
        };
        f.set(sr, cutoff, res);
        f
    }

    /// Recompute coefficients. Call it once per block for a slow sweep, or
    /// every sample for a fast one; the state is untouched either way.
    pub fn set(&mut self, sr: f32, cutoff: f32, res: f32) {
        let c = cutoff.clamp(20.0, 0.45 * sr);
        let g = (std::f32::consts::PI * c / sr).tan();
        self.k = 2.0 - 2.0 * res.min(0.98);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    pub fn tick(&mut self, x: f32, mode: Mode) -> f32 {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match mode {
            Mode::Low => v2,
            Mode::Band => v1,
            Mode::High => x - self.k * v1 - v2,
        }
    }
}

/// A one-pole lowpass, for things that dull rather than resonate: air
/// absorption over distance, the head shadowing the far ear, a parameter being
/// smoothed. Its coefficient is passed in per sample so a moving cutoff can be
/// interpolated across a block without an `exp()` per sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct OnePole {
    state: f32,
}

impl OnePole {
    /// The coefficient for a cutoff of `hz`: 0 blocks everything, 1 passes all.
    #[must_use]
    pub fn coefficient(sr: f32, hz: f32) -> f32 {
        1.0 - (-std::f32::consts::TAU * hz.clamp(20.0, 0.49 * sr) / sr).exp()
    }

    pub fn tick(&mut self, x: f32, coefficient: f32) -> f32 {
        self.state += (x - self.state) * coefficient;
        self.state
    }
}
