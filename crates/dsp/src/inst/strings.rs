//! String machine: the pad of *Oxygène*, an Eminent 310's strings through a phaser.
//!
//! **What a pad is.** A sustained chordal cushion that swells rather than
//! strikes. Everything else in this project announces itself — a kick, a hat, a
//! plucked note. A pad's job is to already be there.
//!
//! **Why three saws instead of one.** A single sawtooth is thin and obviously a
//! machine. Take three copies, detune them slightly, and give each a slow
//! independent pitch wobble a third of a cycle out of step with the others, and
//! they drift in and out of phase with each other forever. That is the
//! **ensemble** effect, and it is the whole trick of the string machines of the
//! 1970s — the Solina, the Eminent 310 that Jarre recorded *Oxygène* on. The ear
//! hears not three detuned oscillators but a section of players who cannot
//! quite agree, which is what a real string section is.
//!
//! Then a lowpass to take the buzz off, which can **breathe** slowly open and
//! shut, and optionally the phaser for the swirl.
//!
//! Every modulation — the wobble, the breath, the sweep — runs on the *track's*
//! clock rather than each note's, so all the notes of a chord move together, as
//! one instrument would.

use serde::{Deserialize, Serialize};

use crate::env::Sustain;
use crate::filter::{Mode, Svf};
use crate::osc::{Phasor, saw};
use crate::phaser::Phaser;
use crate::{Voice, hz};

/// The most saws one note will ever use.
const MAX_COPIES: usize = 5;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    /// Saws per note in the ensemble.
    pub copies: usize,
    /// Cents between the copies at rest.
    pub detune: f32,
    /// Cents each copy wobbles: the shimmer.
    pub depth: f32,
    /// Hz: lower is darker, further away.
    pub cutoff: f32,
    /// 0 still, 1 the cutoff sweeps down to a third and back.
    pub breathe: f32,
    /// Hz of that slow breath.
    pub breathe_rate: f32,
    /// Seconds to swell in.
    pub attack: f32,
    /// Seconds to fade after the note ends.
    pub release: f32,
    /// 0 off, 1 full swirl.
    pub phaser: f32,
    /// Hz: 0.12 is one sweep up and down every eight seconds.
    pub phaser_rate: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            copies: 3,
            detune: 7.0,
            depth: 12.0,
            cutoff: 2600.0,
            breathe: 0.0,
            breathe_rate: 0.07,
            attack: 1.2,
            release: 2.5,
            phaser: 0.0,
            phaser_rate: 0.12,
        }
    }
}

/// The dark dream, chosen by ear from four candidates. Darker, breathing slowly,
/// longer swells, a slower swirl.
#[must_use]
pub fn dark() -> Params {
    Params {
        cutoff: 1100.0,
        breathe: 0.8,
        phaser: 0.7,
        phaser_rate: 0.07,
        attack: 2.5,
        release: 4.0,
        ..Params::default()
    }
}

fn cents(amount: f32) -> f32 {
    (amount / 1200.0).exp2()
}

/// The pitch wobble of one copy in a string-machine ensemble.
///
/// Each copy drifts on a slow cycle and a fast one, a third of a turn out of
/// step with the others. Pass the track's clock.
fn ensemble(t: f32, depth: f32, voice: usize, voices: usize) -> f32 {
    let phase = voice as f32 / voices as f32;
    let tau = std::f32::consts::TAU;
    let wobble = 0.7 * (tau * (0.6 * t + phase)).sin() + 0.3 * (tau * (5.5 * t + phase)).sin();
    cents(depth * wobble)
}

pub struct Strings {
    sr: f32,
    p: Params,
    freq: f32,
    velocity: f32,
    copies: usize,
    phases: [Phasor; MAX_COPIES],
    spreads: [f32; MAX_COPIES],
    filter: Svf,
    phaser: Phaser,
    envelope: Sustain,
    /// Seconds since the transport started: every modulation rides on this.
    track_time: f32,
}

impl Strings {
    #[must_use]
    pub fn new(
        sr: f32,
        pitch: f32,
        velocity: f32,
        length: f32,
        track_time: f32,
        p: Params,
    ) -> Self {
        let copies = p.copies.clamp(1, MAX_COPIES);
        Self {
            sr,
            p,
            freq: hz(pitch),
            velocity,
            copies,
            // Each copy starts at a different point in its cycle, so they never
            // line up into one louder saw.
            phases: std::array::from_fn(|k| Phasor::new(k as f32 / copies as f32)),
            spreads: std::array::from_fn(|k| {
                cents(p.detune * (k as f32 - (copies as f32 - 1.0) / 2.0))
            }),
            filter: Svf::new(sr, p.cutoff, 0.1),
            phaser: Phaser::default(),
            envelope: Sustain::new(sr, length, p.attack, p.release),
            track_time,
        }
    }
}

impl Voice for Strings {
    fn add(&mut self, out: &mut [f32]) {
        // The swirl is pointed once per block: at 0.07 Hz it moves
        // imperceptibly in a millisecond.
        if self.p.phaser > 0.0 {
            self.phaser
                .sweep(self.sr, self.track_time, self.p.phaser_rate);
        }
        let mix = 0.5 * self.p.phaser;

        for s in out.iter_mut() {
            let mut x = 0.0;
            for k in 0..self.copies {
                let wobble = ensemble(self.track_time, self.p.depth, k, self.copies);
                let freq = self.freq * self.spreads[k] * wobble;
                x += saw(self.phases[k].tick(freq, self.sr), freq / self.sr);
            }
            x /= self.copies as f32;

            if self.p.breathe > 0.0 {
                let breath = 0.5
                    - 0.5 * (std::f32::consts::TAU * self.p.breathe_rate * self.track_time).cos();
                self.filter.set(
                    self.sr,
                    self.p.cutoff * (1.0 - self.p.breathe * 0.67 * breath),
                    0.1,
                );
            }
            x = self.filter.tick(x, Mode::Low);

            if mix > 0.0 {
                x = self.phaser.tick(x, mix);
            }

            *s += x * self.envelope.tick() * self.velocity;
            self.track_time += 1.0 / self.sr;
        }
    }

    fn finished(&self) -> bool {
        self.envelope.finished()
    }
}
