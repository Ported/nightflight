//! The Small Stone swirl: notches that sweep slowly up and down the spectrum.
//!
//! An **all-pass** filter is a strange object: it passes every frequency at full
//! level and changes only their *phase*, delaying each one by a different
//! amount. On its own it is almost inaudible. Mix it back with the dry signal
//! and the frequencies that come out half a cycle late cancel, leaving deep
//! notches; the ones that come back in step reinforce. Sweeping the all-pass
//! corner moves those notches up and down the spectrum, and the ear hears a
//! swirl — the sound of Jarre's *Oxygène*, an Eminent 310 organ through an
//! Electro-Harmonix Small Stone.
//!
//! Six stages, because each one contributes a notch and six is what the pedal
//! had. The sweep follows the **track's** clock rather than the note's, so every
//! note of a chord swirls together; and because the whole thing is linear,
//! phasing each note separately gives exactly the same result as phasing the
//! finished chord.

/// One first-order all-pass section.
#[derive(Clone, Copy, Debug, Default)]
struct AllPass {
    previous_in: f32,
    previous_out: f32,
}

impl AllPass {
    fn tick(&mut self, x: f32, a: f32) -> f32 {
        let y = a * x + self.previous_in - a * self.previous_out;
        self.previous_in = x;
        self.previous_out = y;
        y
    }
}

const STAGES: usize = 6;
/// The corner sweeps between these, on a log scale.
const LOW: f32 = 250.0;
const HIGH: f32 = 3500.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Phaser {
    stages: [AllPass; STAGES],
    coefficient: f32,
}

impl Phaser {
    /// Point the notches where they should be at track time `t`.
    ///
    /// Called once per block rather than per sample: at 0.07 Hz the sweep moves
    /// imperceptibly in a millisecond, and this saves six tangents a sample.
    pub fn sweep(&mut self, sr: f32, t: f32, rate: f32) {
        let phase = 0.5 - 0.5 * (std::f32::consts::TAU * rate * t).cos();
        let corner = LOW * (HIGH / LOW).powf(phase);
        let w = (std::f32::consts::PI * corner / sr).tan();
        self.coefficient = (w - 1.0) / (w + 1.0);
    }

    /// `mix` of 0 is dry, 0.5 is the deepest notches, 1 is all-pass only — which
    /// sounds like almost nothing at all, since nothing has cancelled.
    pub fn tick(&mut self, x: f32, mix: f32) -> f32 {
        let mut wet = x;
        for stage in &mut self.stages {
            wet = stage.tick(wet, self.coefficient);
        }
        (1.0 - mix) * x + mix * wet
    }
}
