//! Drum-machine kick: a sine that drops fast in pitch, plus a click.
//!
//! Three layers, the way producers stack kicks:
//!
//! * **the sub** — the pitch settles on the note (tuned to the key), and `hold`
//!   keeps it at full level a moment before the decay: felt more than heard;
//! * **the punch** — the pitch sweep. `sweep`/`pitch_decay` is the slow "doom";
//!   `punch`/`punch_decay` adds a second, very fast drop from hundreds of Hz in
//!   a few milliseconds, and that is what hits the chest;
//! * **the click** — a millisecond of highpassed noise plus a short 1-4 kHz
//!   knock: the ear's cue for exactly when it hit.
//!
//! `drive` saturates the lot. The defaults are the "punch" preset from the
//! Python studio it was ported from, chosen by ear from four candidates.

use crate::env::{Amp, Decay};
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::osc::{Phasor, sine};
use crate::parameters;
use crate::shape::saturate;
use crate::{Voice, hz};

parameters! {
    /// The kick's three layers and what drives them.
    pub struct Params {
        /// Starts this many times above the note's pitch, then falls to it.
        sweep: lin 1.0..=16.0 = 5.0, "x";
        /// Seconds: how fast that fall happens. The "doom".
        pitch_decay: log 0.005..=0.3 = 0.03, "s";
        /// Extra fast drop from this many times the pitch on top. 6 starts near 350 Hz on G1.
        punch: lin 0.0..=20.0 = 6.0, "x";
        /// Seconds: how fast the punch falls.
        punch_decay: log 0.001..=0.05 = 0.004, "s";
        /// Seconds: body amplitude decay.
        decay: log 0.05..=2.0 = 0.32, "s";
        /// Seconds at full level before the decay starts.
        hold: lin 0.0..=0.3 = 0.04, "s";
        /// Level of the beater transient.
        click: lin 0.0..=1.0 = 0.35, "";
        /// Seconds.
        click_decay: log 0.0005..=0.05 = 0.002, "s";
        /// Level of a short 1-4 kHz knock over the click.
        knock: lin 0.0..=1.0 = 0.3, "";
        /// Saturation amount.
        drive: lin 0.5..=8.0 = 3.0, "";
    }
}

pub struct Kick {
    sr: f32,
    p: Params,
    base: f32,
    velocity: f32,
    phase: Phasor,
    sweep: Decay,
    punch: Decay,
    click: Decay,
    knock: Decay,
    highpass: Svf,
    bandpass: Svf,
    noise: Noise,
    amp: Amp,
}

impl Kick {
    /// A kick at `pitch` (MIDI), ringing for `length` seconds before a 20 ms fade.
    #[must_use]
    pub fn new(sr: f32, pitch: f32, velocity: f32, length: f32, p: Params) -> Self {
        Self {
            sr,
            p,
            base: hz(pitch),
            velocity,
            phase: Phasor::default(),
            sweep: Decay::new(sr, p.pitch_decay),
            punch: Decay::new(sr, p.punch_decay),
            click: Decay::new(sr, p.click_decay),
            knock: Decay::new(sr, 0.003),
            // The click is noise above 1.5 kHz; the knock a band around 2 kHz.
            highpass: Svf::new(sr, 1500.0, 0.0),
            bandpass: Svf::new(sr, 2000.0, 0.4),
            // The same noise every hit, like a machine: a drum machine's click
            // is a fixed circuit, not a new sound each time.
            noise: Noise::new(0x5EED),
            amp: Amp::new(sr, length, 0.0005, p.hold, p.decay, 0.02),
        }
    }
}

impl Voice for Kick {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            // Pitch: the note, plus the slow sweep, plus the fast punch.
            let freq = self.base
                * (1.0
                    + (self.p.sweep - 1.0) * self.sweep.tick()
                    + self.p.punch * self.punch.tick());
            let mut y = sine(self.phase.tick(freq, self.sr));

            let n = self.noise.tick();
            y += self.p.click * self.highpass.tick(n, Mode::High) * self.click.tick();
            if self.p.knock > 0.0 {
                y += self.p.knock * self.bandpass.tick(n, Mode::Band) * self.knock.tick();
            }

            *s += saturate(y, self.p.drive) * self.amp.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.amp.finished()
    }
}
