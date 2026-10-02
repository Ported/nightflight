//! 808-style cymbal: the hi-hat's six oscillators, grown up.
//!
//! The 808 has exactly one piece of metal in it — six square-wave oscillators
//! at unrelated frequencies — and every metallic voice it owns is that bank
//! heard through a different window. The hat keeps only the top octave and
//! dies in forty milliseconds. The cymbal keeps a much wider band and lets it
//! ring for seconds, and that is the whole difference between "tick" and
//! "crash".
//!
//! Two envelopes, because a cymbal is two events. The **strike** is the stick
//! meeting the bell: bright, fast, gone in a tenth of a second — on a ride
//! this is the "ping" you play time on. The **wash** is the plate itself
//! ringing: the long bloom that swallows a mix after a crash. A crash is
//! mostly wash; a ride is mostly strike. Same instrument, two patches.
//!
//! Choking a cymbal is grabbing it with a hand, which takes longer than a
//! pedal closing on a hat — so the choke `release` here defaults slower.

use crate::env::{Decay, Sustain};
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::osc::{Phasor, saw};
use crate::parameters;
use crate::{SR, Voice};

/// The same six frequencies as the hat, scaled down a little: a cymbal is a
/// bigger plate, and a bigger plate rings lower.
const OSCILLATORS: [f32; 6] = [164.2, 243.5, 295.7, 418.2, 432.0, 640.0];

/// Measured, not chosen: the bank band-limited and summed leaves little
/// behind, as with the hat. Set so the default crash peaks near full scale
/// at full velocity.
const MAKEUP: f32 = 4.0;

parameters! {
    /// The strike and the wash.
    pub struct Params {
        /// Seconds: how long the wash rings. 2.5 is a crash, 0.9 a ride.
        decay: log 0.2..=4.0 = 2.5, "s";
        /// Level of the strike — the ping a ride is played on.
        strike: lin 0.0..=2.0 = 0.5, "";
        /// Seconds: how fast the strike fades into the wash.
        strike_decay: log 0.01..=0.3 = 0.06, "s";
        /// Hz: where the kept band begins. Lower is darker, trashier metal.
        tone: log 2000.0..=12000.0 = 5000.0, "Hz";
        /// White noise mixed in: the wash's air.
        noise: lin 0.0..=1.0 = 0.5, "";
        /// Seconds: the choke — a hand grabbing the plate.
        release: log 0.005..=0.2 = 0.05, "s";
    }
}

pub struct Cymbal {
    phases: [Phasor; 6],
    noise: Noise,
    high: [Svf; 2],
    strike_band: Svf,
    wash: Decay,
    strike: Decay,
    gate: Sustain,
    velocity: f32,
    noise_level: f32,
    strike_level: f32,
}

impl Cymbal {
    /// A cymbal ringing for `length` seconds. `seed` varies the oscillator
    /// phases per hit, exactly as the hat does: free-running metal.
    #[must_use]
    pub fn new(sr: f32, velocity: f32, length: f32, seed: u32, p: Params) -> Self {
        let mut rng = Noise::new(seed);
        Self {
            phases: std::array::from_fn(|_| Phasor::new(rng.tick().abs())),
            noise: Noise::new(seed ^ 0xA5A5),
            high: [Svf::new(sr, p.tone, 0.0), Svf::new(sr, p.tone, 0.0)],
            // The strike is the band an octave above the wash's floor: the
            // bell of the cymbal, brighter than the body.
            strike_band: Svf::new(sr, (p.tone * 2.0).min(0.4 * sr), 0.3),
            wash: Decay::new(sr, p.decay),
            strike: Decay::new(sr, p.strike_decay),
            gate: Sustain::new(sr, length, 0.0005, p.release),
            velocity,
            noise_level: p.noise,
            strike_level: p.strike,
        }
    }
}

impl Voice for Cymbal {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            let mut metal = 0.0;
            for (phase, freq) in self.phases.iter_mut().zip(OSCILLATORS) {
                let dt = freq / SR;
                let p = phase.tick(freq, SR);
                metal += saw(p, dt) - saw((p + 0.5).fract(), dt);
            }
            let raw = metal / OSCILLATORS.len() as f32 + self.noise_level * self.noise.tick();
            let mut body = raw;
            for filter in &mut self.high {
                body = filter.tick(body, Mode::High);
            }
            let ping = self.strike_band.tick(raw, Mode::Band);
            let y = body * self.wash.tick() + self.strike_level * ping * self.strike.tick();
            *s += y * MAKEUP * self.gate.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.gate.finished()
    }
}
