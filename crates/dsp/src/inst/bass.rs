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
use crate::shape::saturate;
use crate::{Voice, hz};

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Hz: the filter's floor, where the sweep lands.
    pub cutoff: f32,
    /// The envelope opens the filter to `cutoff * (1 + env_amount)`.
    pub env_amount: f32,
    /// Seconds: how fast the filter shuts. Short is plucky.
    pub env_decay: f32,
    /// 0 to just under 1. A boost at the cutoff itself.
    pub resonance: f32,
    /// Share of a sine at the fundamental, mixed under the saw.
    pub sub: f32,
    pub drive: f32,
    pub attack: f32,
    /// Seconds: amplitude decay while the note is held.
    pub decay: f32,
    pub release: f32,
}

impl Default for Params {
    fn default() -> Self {
        // Rolling's settings: a touch more resonance and drive than the bare
        // instrument, and a fast filter envelope.
        Self {
            cutoff: 200.0,
            env_amount: 4.0,
            env_decay: 0.05,
            resonance: 0.6,
            sub: 0.35,
            drive: 2.5,
            attack: 0.003,
            decay: 0.25,
            release: 0.03,
        }
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
