//! Two-operator FM, DX7-style: a sine whose phase is wobbled by another sine.
//!
//! **What FM synthesis is.** Everything else here is *subtractive*: start with a
//! rich waveform and filter things away. This is the opposite. Take one plain
//! sine — the **carrier** — and use a second sine, the **modulator**, to push
//! its phase back and forth. Do that slowly and you hear vibrato. Do it at an
//! audible rate and something else happens entirely: the wobble stops being
//! heard as movement and becomes *timbre*. The result is a whole family of new
//! partials at `carrier ± n × modulator`, from two sine waves and a multiply.
//!
//! John Chowning found this at Stanford in 1967 while trying to program
//! vibrato, and it became the Yamaha DX7 in 1983 — the sound of that decade's
//! records, and the reason electric pianos, bells and glassy bass appear on
//! everything from 1984 onwards.
//!
//! Three controls decide what you get:
//!
//! * **ratio**, the modulator's frequency as a multiple of the carrier's. A
//!   whole number puts every new partial on a harmonic of the carrier, so the
//!   result has a clear pitch. A ratio like 1.41 puts them between the
//!   harmonics, and the ear stops hearing a note and starts hearing metal — the
//!   same reason the 808's six unrelated squares sound like a cymbal.
//! * **index**, how hard the modulator pushes. Zero is a bare sine; the higher
//!   it goes the more partials appear and the brighter the sound.
//! * **index_decay**, and this is what makes it *glass*. A struck object is
//!   brightest at the instant it is hit and dulls as it rings. Letting the index
//!   fall away does exactly that: a bright "tink" melting into an almost pure
//!   tone. Subtractive synthesis can imitate it with a filter; FM gets it from
//!   the physics of the spectrum itself.
//!
//! Two carriers a few cents apart beat slowly against each other, which the ear
//! reads as shimmer rather than as two notes.

use serde::{Deserialize, Serialize};

use crate::env::{Amp, Decay};
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::osc::{Phasor, sine};
use crate::{Voice, hz};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// Modulator frequency as a multiple of the carrier's. Whole numbers stay
    /// in tune; anything else goes metallic.
    pub ratio: f32,
    /// How hard the modulator pushes at the strike: the brightness of the tink.
    pub index: f32,
    /// Seconds for that brightness to melt away.
    pub index_decay: f32,
    /// Cents between the two carriers: a slow shimmer as they beat.
    pub detune: f32,
    pub attack: f32,
    /// Seconds, amplitude.
    pub decay: f32,
    /// Seconds after the note ends, so arpeggios ring into each other.
    pub release: f32,
    /// Level of a 3 ms high tick at the start: the mallet touching the glass.
    pub strike: f32,
    /// Cents of slow pitch drift, like tape running unevenly.
    pub wow_depth: f32,
    /// Hz of that drift.
    pub wow_rate: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            ratio: 3.0,
            index: 2.2,
            index_decay: 0.12,
            detune: 6.0,
            attack: 0.004,
            decay: 1.6,
            release: 0.6,
            strike: 0.12,
            wow_depth: 0.0,
            wow_rate: 0.25,
        }
    }
}

/// Frequency ratio of a detune in cents.
fn cents(amount: f32) -> f32 {
    (amount / 1200.0).exp2()
}

pub struct Glass {
    sr: f32,
    p: Params,
    base: f32,
    velocity: f32,
    /// Two carriers, detuned either side, and the modulator.
    low: Phasor,
    high: Phasor,
    modulator: Phasor,
    spread: f32,
    index: Decay,
    amp: Amp,
    /// Seconds since the *track* started, not the note: the wow has to bend
    /// every note together, as one warped tape would.
    track_time: f32,
    strike: Decay,
    strike_fade: f32,
    tick_filter: Svf,
    noise: Noise,
}

impl Glass {
    #[must_use]
    pub fn new(
        sr: f32,
        pitch: f32,
        velocity: f32,
        length: f32,
        track_time: f32,
        p: Params,
    ) -> Self {
        Self {
            sr,
            p,
            base: hz(pitch),
            velocity,
            low: Phasor::default(),
            high: Phasor::default(),
            modulator: Phasor::default(),
            spread: cents(p.detune / 2.0),
            index: Decay::new(sr, p.index_decay),
            amp: Amp::new(sr, length, p.attack, 0.0, p.decay, p.release),
            track_time,
            strike: Decay::new(sr, 0.003),
            strike_fade: 0.0,
            // The tone alone is nearly pure, and a pure tone is hard to place:
            // the cues for front, back and height all live above about 4 kHz.
            // This tick gives the ear an onset to locate, as a real mallet would.
            tick_filter: Svf::new(sr, 4000.0, 0.0),
            noise: Noise::new(0x61A5),
        }
    }
}

impl Voice for Glass {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            // Wow runs on the track clock, so all the notes bend as one.
            let wow = if self.p.wow_depth > 0.0 {
                cents(
                    self.p.wow_depth
                        * (std::f32::consts::TAU * self.p.wow_rate * self.track_time).sin(),
                )
            } else {
                1.0
            };
            let freq = self.base * wow;

            // Phase modulation: the modulator is added to the carriers' phase.
            let index = self.p.index * self.index.tick();
            let wobble = index * sine(self.modulator.tick(freq * self.p.ratio, self.sr));
            let low = self.low.tick(freq / self.spread, self.sr);
            let high = self.high.tick(freq * self.spread, self.sr);
            let mut y = 0.5
                * (sine_with_offset(low, wobble) + sine_with_offset(high, wobble))
                * self.amp.tick();

            if self.p.strike > 0.0 {
                // A 3 ms burst above 4 kHz, itself faded in over half a
                // millisecond so the tick does not start with a step.
                let tick = self.tick_filter.tick(self.noise.tick(), Mode::High);
                // The fade is read before it is advanced, so the very first
                // sample of a note is exactly zero. A tick that begins at
                // one twenty-fourth of full level is only -59 dBFS, but "every
                // note starts on a ramp" is either true or it is not.
                let fade = self.strike_fade;
                self.strike_fade = (self.strike_fade + 1.0 / (0.0005 * self.sr)).min(1.0);
                y += self.p.strike * tick * self.strike.tick() * fade;
            }

            *s += y * self.velocity;
            self.track_time += 1.0 / self.sr;
        }
    }

    fn finished(&self) -> bool {
        self.amp.finished()
    }
}

/// A sine at `phase` turns, with `offset` radians added.
fn sine_with_offset(phase: f32, offset: f32) -> f32 {
    (std::f32::consts::TAU * phase + offset).sin()
}
