//! Plucky rolling-techno bass: saw plus a sine sub, through a resonant lowpass
//! that an envelope snaps open and shut.
//!
//! This is subtractive synthesis in one line: start with a harmonically rich
//! waveform and take things away. The saw contains every harmonic; the filter
//! decides how many survive, and because the envelope opens it for only a few
//! tens of milliseconds, each note starts bright and immediately darkens. That
//! movement is the "pluck" — it is the filter, not the volume.
//!
//! The sine underneath carries weight that a filtered saw loses, and the
//! resonance (a boost right at the cutoff) is what makes the sweep audible as a
//! vowel rather than just a dimming.

use crate::env::{Amp, Decay};
use crate::filter::{Mode, Svf};
use crate::osc::{Phasor, saw, sine};
use crate::parameters;
use crate::shape::saturate;
use crate::{Voice, hz};

parameters! {
    /// A saw and a sub through a filter an envelope snaps open.
    pub struct Params {
        /// Hz: the filter's floor, where the sweep lands.
        cutoff: log 20.0..=8000.0 = 200.0, "Hz";
        /// The envelope opens the filter to cutoff times one plus this.
        env_amount: lin 0.0..=16.0 = 4.0, "";
        /// Seconds: how fast the filter shuts. Short is plucky.
        env_decay: log 0.005..=1.0 = 0.05, "s";
        /// 0 to just under 1. A boost at the cutoff itself.
        resonance: lin 0.0..=0.98 = 0.6, "";
        /// Share of a sine at the fundamental, mixed under the saw.
        sub: lin 0.0..=1.0 = 0.35, "";
        /// Saturation amount.
        drive: lin 0.5..=8.0 = 2.5, "";
        /// Seconds.
        attack: log 0.0005..=0.1 = 0.003, "s";
        /// Seconds: amplitude decay while the note is held.
        decay: log 0.01..=2.0 = 0.25, "s";
        /// Seconds.
        release: log 0.005..=1.0 = 0.03, "s";
    }
}

pub struct Bass {
    sr: f32,
    p: Params,
    freq: f32,
    velocity: f32,
    saw_phase: Phasor,
    sub_phase: Phasor,
    filter: Svf,
    sweep: Decay,
    amp: Amp,
}

impl Bass {
    #[must_use]
    pub fn new(sr: f32, pitch: f32, velocity: f32, length: f32, p: Params) -> Self {
        Self {
            sr,
            p,
            freq: hz(pitch),
            velocity,
            saw_phase: Phasor::default(),
            sub_phase: Phasor::default(),
            filter: Svf::new(sr, p.cutoff, p.resonance),
            sweep: Decay::new(sr, p.env_decay),
            amp: Amp::new(sr, length, p.attack, 0.0, p.decay, p.release),
        }
    }
}

impl Voice for Bass {
    fn add(&mut self, out: &mut [f32]) {
        let dt = self.freq / self.sr;
        for s in out.iter_mut() {
            let osc = (1.0 - self.p.sub) * saw(self.saw_phase.tick(self.freq, self.sr), dt)
                + self.p.sub * sine(self.sub_phase.tick(self.freq, self.sr));

            // The cutoff is recomputed every sample. That is a tangent per
            // sample, which sounds expensive and is not: the filter is
            // zero-delay-feedback precisely so its cutoff can move this fast
            // without stepping.
            let cutoff = self.p.cutoff * (1.0 + self.p.env_amount * self.sweep.tick());
            self.filter.set(self.sr, cutoff, self.p.resonance);

            let y = saturate(self.filter.tick(osc, Mode::Low), self.p.drive);
            *s += y * self.amp.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.amp.finished()
    }
}
