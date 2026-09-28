//! The transport: where we are in the bar.
//!
//! The position is an integer count of samples, never a float that creeps.
//! Where a step lands is derived from that count each time, so 126 BPM stays
//! 126 BPM after an hour.
//!
//! Tempo is **anchored**. A step's position is measured from the last tempo
//! change, not from the beginning of time — `origin_sample + round((step -
//! origin_step) * samples_per_step)`. Without the anchor, changing tempo
//! rescales the entire grid retroactively: halving it puts the next step
//! twelve seconds away and the music stops, doubling it puts the next step in
//! the past and the sequencer fires hundreds of catch-up steps inside one
//! callback. Anchoring makes a tempo change mean "from here on", which is what
//! a hand on a tempo knob means.
//!
//! The anchor also preserves the **phase** of the step in progress. A tempo
//! change lands in the middle of a sixteenth, and the part of it already played
//! cannot be unplayed; so the anchor is placed such that only the *remainder*
//! is stretched to the new tempo. Anchoring naively on the pending step instead
//! fires it at once, which shortens one step and puts an audible hiccup in the
//! beat exactly where a hand touched the knob.

/// A sixteenth is the grid everything sits on, as in the Python studio.
pub const STEPS_PER_BAR: u64 = 16;

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    sr: f64,
    bpm: f64,
    /// Samples played since the transport started.
    pub sample: u64,
    /// The next step to fire.
    pub step: u64,
    /// Where the current tempo started, in samples and in steps.
    origin_sample: u64,
    origin_step: u64,
    /// Bars elapsed before the current tempo took over, so the bar counter
    /// stays continuous across a tempo change.
    origin_bar: f64,
}

impl Clock {
    #[must_use]
    pub fn new(sr: f32, bpm: f32) -> Self {
        Self {
            sr: f64::from(sr),
            bpm: f64::from(bpm),
            sample: 0,
            step: 0,
            origin_sample: 0,
            origin_step: 0,
            origin_bar: 0.0,
        }
    }

    /// Samples in one sixteenth. Fractional on purpose: 126 BPM is 5714.28…
    /// samples, and rounding it per step would drift about a second an hour.
    #[must_use]
    pub fn samples_per_step(&self) -> f64 {
        60.0 / self.bpm / 4.0 * self.sr
    }

    /// Where a given step falls, measured from the anchor.
    #[must_use]
    pub fn step_sample(&self, step: u64) -> u64 {
        let steps = step as f64 - self.origin_step as f64;
        let offset = (steps * self.samples_per_step()).round();
        (self.origin_sample as f64 + offset).max(0.0) as u64
    }

    /// Jump the transport to `bar`.
    ///
    /// The sample count *is* the musical position — the gate's rhythm and the
    /// control grid are both derived from it — so seeking moves it rather than
    /// keeping it running. Everything is re-anchored at the new position, which
    /// also means the next step fires immediately instead of the sequencer
    /// trying to catch up through every step it skipped.
    pub fn seek(&mut self, bar: f64) {
        let bar = bar.max(0.0);
        let sample = (bar * self.samples_per_step() * STEPS_PER_BAR as f64).max(0.0) as u64;
        self.sample = sample;
        self.origin_sample = sample;
        self.origin_bar = bar;
        self.step = (bar * STEPS_PER_BAR as f64).floor().max(0.0) as u64;
        self.origin_step = self.step;
    }

    /// The sample a given bar falls on.
    #[must_use]
    pub fn sample_of_bar(&self, bar: f64) -> u64 {
        let bars = bar - self.origin_bar;
        let samples = bars * self.samples_per_step() * STEPS_PER_BAR as f64;
        (self.origin_sample as f64 + samples).max(0.0) as u64
    }

    /// Which step a sample falls in. Used by flights, which fire their notes
    /// ahead of the beat rather than on it.
    #[must_use]
    pub fn step_of(&self, sample: u64) -> u64 {
        let since = sample as f64 - self.origin_sample as f64;
        let steps = (since / self.samples_per_step()).floor();
        (self.origin_step as f64 + steps).max(0.0) as u64
    }

    /// The sample at which the next step falls.
    #[must_use]
    pub fn next_step_sample(&self) -> u64 {
        self.step_sample(self.step)
    }

    pub fn advance(&mut self, samples: usize) {
        self.sample += samples as u64;
    }

    /// Change tempo from here on, leaving the steps already played where they
    /// are and stretching only what is left of the step in progress.
    pub fn set_bpm(&mut self, bpm: f32) {
        let bpm = f64::from(bpm).clamp(20.0, 300.0);
        if (bpm - self.bpm).abs() < f64::EPSILON {
            return;
        }
        // How far into the current step we are, 0 to 1.
        let previous = self.step_sample(self.step.saturating_sub(1));
        let elapsed = (self.sample as f64 - previous as f64).max(0.0);
        let fraction = (elapsed / self.samples_per_step()).clamp(0.0, 1.0);
        let bar_now = self.bar();

        self.bpm = bpm;
        let spss = self.samples_per_step();
        // Anchor on the *previous* step, placed back by the fraction already
        // played at the new tempo, so the next step lands (1 - fraction) of a
        // new step away and the bar counter stays continuous.
        self.origin_step = self.step.saturating_sub(1);
        self.origin_sample = (self.sample as f64 - fraction * spss).max(0.0) as u64;
        self.origin_bar = bar_now - fraction / STEPS_PER_BAR as f64;
    }

    /// Position in bars, for the UI.
    #[must_use]
    pub fn bar(&self) -> f64 {
        self.bar_of(self.sample)
    }

    /// The bar any given sample falls on, past or future.
    #[must_use]
    pub fn bar_of(&self, sample: u64) -> f64 {
        let since = sample as f64 - self.origin_sample as f64;
        self.origin_bar + since / (self.samples_per_step() * STEPS_PER_BAR as f64)
    }

    #[must_use]
    pub fn bpm(&self) -> f32 {
        self.bpm as f32
    }
}
