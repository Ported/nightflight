//! 808-style hi-hat: six detuned squares, filtered down to the top octave.
//!
//! Why metal sounds metallic: a struck string or a drum head vibrates in modes
//! that are whole-number multiples of a fundamental, and the ear hears that as
//! a pitch. A struck *plate* vibrates in modes at no simple ratio at all, and
//! the ear gives up on pitch and hears "metal". The TR-808 faked a cymbal with
//! six square-wave oscillators at deliberately unrelated frequencies, then
//! threw away everything below the top octave or so. That is all this is.
//!
//! Open or closed is only the decay — plus the note's length, since a note that
//! ends early is choked, exactly as a pedal closing on a ringing hat.

use serde::{Deserialize, Serialize};

use crate::env::Amp;
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::osc::{Phasor, saw};
use crate::{SR, Voice};

/// The 808's six hi-hat oscillators. The ratios between them are the point:
/// 205.3 to 800 is not an octave and a half of anything.
const OSCILLATORS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

/// Makeup gain, measured rather than chosen: the six squares summed and then
/// band-limited to 6-16 kHz leave very little behind, since only their high
/// harmonics survive. The Python version divides each hit by its own peak,
/// which a streaming voice cannot do — it would have to see the whole note
/// first — so the gain is fixed here and the lane's level does the balancing.
const MAKEUP: f32 = 6.5;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// Seconds: about 0.04 closed, about 0.3 open.
    pub decay: f32,
    /// Hz: centre of the band kept. The band runs 0.6x to 1.6x this.
    pub tone: f32,
    /// White noise mixed in for sizzle.
    pub noise: f32,
    /// Seconds: the fade when choked, i.e. when the note ends.
    pub release: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            decay: 0.04,
            tone: 10_000.0,
            noise: 0.3,
            release: 0.008,
        }
    }
}

pub struct Hat {
    phases: [Phasor; 6],
    noise: Noise,
    /// Two highpasses in series for a steeper skirt, then a lowpass: the band.
    high: [Svf; 2],
    low: Svf,
    amp: Amp,
    velocity: f32,
    noise_level: f32,
}

impl Hat {
    /// A hat ringing for `length` seconds. `seed` varies the oscillator phases:
    /// free-running oscillators meet each hit at a different point, so no two
    /// hits are quite identical, and passing the step number keeps it repeatable.
    #[must_use]
    pub fn new(sr: f32, velocity: f32, length: f32, seed: u32, p: Params) -> Self {
        let mut rng = Noise::new(seed);
        Self {
            phases: std::array::from_fn(|_| Phasor::new(rng.tick().abs())),
            noise: Noise::new(seed ^ 0xA5A5),
            high: [Svf::new(sr, 6000.0, 0.0), Svf::new(sr, 6000.0, 0.0)],
            low: Svf::new(sr, (p.tone * 1.6).min(0.45 * sr), 0.0),
            amp: Amp::new(sr, length, 0.0005, 0.0, p.decay, p.release),
            velocity,
            noise_level: p.noise,
        }
    }
}

impl Voice for Hat {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            // Six band-limited squares: a square is two saws half a cycle apart.
            let mut metal = 0.0;
            for (phase, freq) in self.phases.iter_mut().zip(OSCILLATORS) {
                let dt = freq / SR;
                let p = phase.tick(freq, SR);
                metal += saw(p, dt) - saw((p + 0.5).fract(), dt);
            }
            let mut y = metal / OSCILLATORS.len() as f32 + self.noise_level * self.noise.tick();
            y = self.low.tick(y, Mode::Low);
            for filter in &mut self.high {
                y = filter.tick(y, Mode::High);
            }
            *s += y * MAKEUP * self.amp.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.amp.finished()
    }
}
