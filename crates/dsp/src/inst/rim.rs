//! Rimshot, clave, woodblock: a click and a ping — the shortest sounds a kit
//! makes.
//!
//! Hit the rim and the head of a snare with one stroke and almost nothing
//! rings: a millisecond of stick noise and one narrow resonance of the shell,
//! gone in a few hundredths of a second. A clave is the same idea in wood —
//! two sticks, one bright dry note near 2.5 kHz. A woodblock is the hollow
//! cousin, lower and a little longer. All three are a damped sine with a
//! click on the front, which is exactly what this is.
//!
//! These sounds carry a groove's *placement*: too quiet to crowd anything and
//! short enough to land exactly where they were written. Listen for where
//! they sit against the hat — a clave two sixteenths ahead of the backbeat is
//! most of what makes a pattern Latin.
//!
//! The lane's root tunes the ping; the patch sets how it is struck. `second`
//! adds the shell's upper mode at 2.4x — present on a rimshot, absent on a
//! clave, faint on a woodblock.

use crate::env::{Amp, Decay};
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::osc::{Phasor, sine};
use crate::parameters;
use crate::shape::saturate;
use crate::{Voice, hz};

/// The shell mode: 2.4x the ping, inharmonic the way a rim actually is.
const SECOND: f32 = 2.4;

parameters! {
    /// One ping, and how it is struck.
    pub struct Params {
        /// Seconds: how long the ping rings. All of these are short.
        decay: log 0.01..=0.15 = 0.04, "s";
        /// Level of the shell's upper mode at 2.4x the root.
        second: lin 0.0..=1.0 = 0.4, "";
        /// Level of the stick's millisecond of noise.
        click: lin 0.0..=1.0 = 0.4, "";
        /// Saturation amount.
        drive: lin 0.5..=8.0 = 1.5, "";
        /// Seconds: the fade when choked, that is, when the note ends.
        release: log 0.001..=0.05 = 0.003, "s";
    }
}

pub struct Rim {
    sr: f32,
    p: Params,
    base: f32,
    velocity: f32,
    phases: [Phasor; 2],
    click: Decay,
    highpass: Svf,
    noise: Noise,
    amp: Amp,
}

impl Rim {
    /// A rim at `pitch` (MIDI): the ping lands on that note.
    #[must_use]
    pub fn new(sr: f32, pitch: f32, velocity: f32, length: f32, p: Params) -> Self {
        Self {
            sr,
            p,
            base: hz(pitch),
            velocity,
            phases: std::array::from_fn(|_| Phasor::default()),
            click: Decay::new(sr, 0.001),
            highpass: Svf::new(sr, 3000.0, 0.0),
            // The same stick every hit, like a machine.
            noise: Noise::new(0x121113),
            amp: Amp::new(sr, length, 0.0005, 0.0, p.decay, p.release),
        }
    }
}

impl Voice for Rim {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            let mut y = sine(self.phases[0].tick(self.base, self.sr));
            if self.p.second > 0.0 {
                y += self.p.second * sine(self.phases[1].tick(self.base * SECOND, self.sr));
            }
            if self.p.click > 0.0 {
                y += self.p.click
                    * self.highpass.tick(self.noise.tick(), Mode::High)
                    * self.click.tick();
            }
            *s += saturate(y, self.p.drive) * self.amp.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.amp.finished()
    }
}
