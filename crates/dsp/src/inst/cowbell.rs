//! 808 cowbell: two squares a strange fifth apart, and nothing else.
//!
//! A real cowbell is a bent steel box, and a box that is wider at the mouth
//! than the top has two strong modes that land nowhere near a musical
//! interval. The 808 built exactly that: two square waves at 540 and 800 Hz —
//! a ratio of 1.48, almost a fifth but audibly wrong — through a bandpass
//! that keeps the honk and discards the buzz. The wrongness is the cowbell:
//! tune the pair to a true fifth and you get an organ, not a kitchen.
//!
//! Here the lower mode sits on the lane's root (the 808's is near C5) and
//! `ratio` sets how wrong the upper one is. The honk lives in the bandpass;
//! `clank` adds the first millisecond of stick-on-steel.

use crate::env::{Amp, Decay};
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::osc::{Phasor, saw};
use crate::parameters;
use crate::shape::saturate;
use crate::{SR, Voice, hz};

parameters! {
    /// The box and how hard it is hit.
    pub struct Params {
        /// The upper mode, as a multiple of the root. 1.48 is the 808's wrongness.
        ratio: lin 1.1..=2.0 = 1.48, "x";
        /// Seconds: how long the box rings.
        decay: log 0.05..=0.8 = 0.22, "s";
        /// Level of the stick's first millisecond.
        clank: lin 0.0..=1.0 = 0.25, "";
        /// Saturation amount.
        drive: lin 0.5..=8.0 = 2.0, "";
        /// Seconds: the fade when choked, that is, when the note ends.
        release: log 0.001..=0.05 = 0.005, "s";
    }
}

pub struct Cowbell {
    p: Params,
    base: f32,
    velocity: f32,
    phases: [Phasor; 2],
    band: Svf,
    clank: Decay,
    highpass: Svf,
    noise: Noise,
    amp: Amp,
}

impl Cowbell {
    /// A cowbell at `pitch` (MIDI): the lower mode lands on that note.
    #[must_use]
    pub fn new(sr: f32, pitch: f32, velocity: f32, length: f32, p: Params) -> Self {
        let base = hz(pitch);
        Self {
            p,
            base,
            velocity,
            phases: std::array::from_fn(|_| Phasor::default()),
            // The honk: a band sitting between the two modes, resonant enough
            // to sing and loose enough to let both through.
            band: Svf::new(sr, base * 1.2, 0.3),
            clank: Decay::new(sr, 0.001),
            highpass: Svf::new(sr, 2500.0, 0.0),
            // The same stick every hit, like a machine.
            noise: Noise::new(0xC0BE11),
            amp: Amp::new(sr, length, 0.0005, 0.0, p.decay, p.release),
        }
    }
}

impl Voice for Cowbell {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            let mut metal = 0.0;
            for (phase, ratio) in self.phases.iter_mut().zip([1.0, self.p.ratio]) {
                let freq = self.base * ratio;
                let dt = freq / SR;
                let ph = phase.tick(freq, SR);
                metal += saw(ph, dt) - saw((ph + 0.5).fract(), dt);
            }
            let mut y = self.band.tick(metal * 0.5, Mode::Band);
            if self.p.clank > 0.0 {
                y += self.p.clank
                    * self.highpass.tick(self.noise.tick(), Mode::High)
                    * self.clank.tick();
            }
            *s += saturate(y, self.p.drive) * self.amp.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.amp.finished()
    }
}
