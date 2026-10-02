//! Shaker: a hundred grains arriving almost together.
//!
//! Everything else in a kit is struck once. A shaker is hundreds of seeds
//! thrown against a shell, each a tiny click, landing over a few hundredths
//! of a second — so the sound has no edge. It *builds*: the envelope rises
//! over ten-odd milliseconds before it falls, and that soft-fronted "chick"
//! is what tells the ear "shaker" rather than "hat". Play them side by side
//! and listen to the fronts: the hat bites, the shaker breathes.
//!
//! Band-limited noise is already a crowd of clicks, so the whole instrument
//! is noise through a bandpass under an envelope with an attack. `attack` is
//! the length of the throw — short is a maraca's snap, long is a loose
//! egg-shaker. Each hit is seeded differently: seeds never land the same way
//! twice, and that un-machine-like shimmer is the point of a shaker over a
//! hat.

use crate::Voice;
use crate::env::Amp;
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::parameters;

parameters! {
    /// The throw and the shell.
    pub struct Params {
        /// Seconds: how long the grains take to all land. The soft front.
        attack: log 0.002..=0.05 = 0.012, "s";
        /// Seconds: the fall after they have.
        decay: log 0.02..=0.3 = 0.07, "s";
        /// Hz: the shell's colour. Higher is drier, smaller.
        tone: log 2000.0..=12000.0 = 5500.0, "Hz";
        /// How ringy the shell is: papery at 0, hollow toward 1.
        ring: lin 0.0..=0.9 = 0.3, "";
        /// Seconds: the fade when choked, that is, when the note ends.
        release: log 0.001..=0.05 = 0.008, "s";
    }
}

pub struct Shaker {
    band: Svf,
    noise: Noise,
    amp: Amp,
    velocity: f32,
}

impl Shaker {
    /// A shaker has no pitch: `tone` places it. `seed` varies the grains per
    /// hit — the step number, so no two shakes match and a render repeats.
    #[must_use]
    pub fn new(sr: f32, velocity: f32, length: f32, seed: u32, p: Params) -> Self {
        Self {
            band: Svf::new(sr, p.tone.min(0.4 * sr), p.ring),
            noise: Noise::new(seed | 1),
            amp: Amp::new(sr, length, p.attack, 0.0, p.decay, p.release),
            velocity,
        }
    }
}

impl Voice for Shaker {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            let y = self.band.tick(self.noise.tick(), Mode::Band);
            *s += 2.0 * y * self.amp.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.amp.finished()
    }
}
