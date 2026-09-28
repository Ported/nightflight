//! Parameter smoothing.
//!
//! Every control that a hand can move has to be smoothed before it reaches the
//! signal, or it steps: a gain that jumps from 0.8 to 0.3 between two samples
//! is a discontinuity, and a discontinuity is a click. A one-pole slew over
//! about 20 ms is inaudible as a delay and removes the step entirely.

/// A value that chases a target.
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    value: f32,
    target: f32,
    coefficient: f32,
}

impl Smoothed {
    /// `time` is the slew's time constant in seconds: 0.02 is the usual choice.
    #[must_use]
    pub fn new(sr: f32, time: f32, initial: f32) -> Self {
        Self {
            value: initial,
            target: initial,
            coefficient: 1.0 - (-1.0 / (time.max(1e-5) * sr)).exp(),
        }
    }

    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    pub fn tick(&mut self) -> f32 {
        self.value += (self.target - self.value) * self.coefficient;
        self.value
    }

    #[must_use]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// True once the value has effectively arrived, so a silent lane can be
    /// skipped without cutting it off mid-fade.
    #[must_use]
    pub fn settled_at_zero(&self) -> bool {
        self.target == 0.0 && self.value < 1e-4
    }
}
