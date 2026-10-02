//! Drum-machine clap: several hands that never quite agree.
//!
//! One pair of hands is a single noise burst, over in milliseconds, and it
//! sounds like nothing. What a record calls a clap is a *crowd* — many pairs
//! of hands missing each other by hundredths of a second. The 808 fakes the
//! crowd with one noise source and an envelope that fires three or four times
//! about ten milliseconds apart, then lets the last burst ring out as a tail:
//! the individual misses blur into one fat "clap", and the tail is the room
//! the crowd is standing in.
//!
//! The noise runs through a resonant bandpass near 1 kHz — the papery honk
//! that says "hands" rather than "snare". Listen for the thickness of the
//! attack: that is the bursts smearing, and `spread` is the knob that sets
//! how sloppy the crowd is. Too tight and it is a snare; too loose and you
//! hear the separate hits, which is a flam, not a clap.

use crate::env::{Decay, Sustain};
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::parameters;
use crate::shape::saturate;
use crate::{SR, Voice};

/// How fast each burst dies. Fixed: it only needs to be gone before the next
/// burst arrives, and 5 ms is gone.
const BURST_TAU: f32 = 0.005;

parameters! {
    /// The crowd and the room it stands in.
    pub struct Params {
        /// How many hands: envelope firings before the tail.
        bursts: lin 1.0..=5.0 = 3.0, "";
        /// Seconds between firings: how sloppy the crowd is.
        spread: log 0.005..=0.03 = 0.011, "s";
        /// Seconds: the tail after the last burst — the room.
        decay: log 0.05..=0.8 = 0.18, "s";
        /// Hz: the bandpass centre. Near 1 kHz says hands.
        tone: log 400.0..=4000.0 = 1100.0, "Hz";
        /// How ringy the band is: papery at 0, honky toward 1.
        ring: lin 0.0..=0.9 = 0.65, "";
        /// Saturation amount.
        drive: lin 0.5..=8.0 = 1.5, "";
        /// Seconds: the fade when choked, that is, when the note ends.
        release: log 0.001..=0.05 = 0.01, "s";
    }
}

pub struct Clap {
    p: Params,
    velocity: f32,
    /// Samples until the next envelope firing, and firings left.
    until_next: u32,
    left: u32,
    burst: Decay,
    tail: Decay,
    /// The tail is silent until the last burst fires it.
    tail_live: bool,
    band: Svf,
    noise: Noise,
    gate: Sustain,
}

impl Clap {
    /// A clap has no pitch: the lane's root is ignored, and `tone` places it.
    #[must_use]
    pub fn new(sr: f32, velocity: f32, length: f32, p: Params) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let left = p.bursts.round().max(1.0) as u32;
        Self {
            p,
            velocity,
            until_next: 0,
            left,
            burst: Decay::new(sr, BURST_TAU),
            tail: Decay::new(sr, p.decay),
            tail_live: false,
            band: Svf::new(sr, p.tone, p.ring),
            // The same noise every hit, like a machine: one crowd, hired for
            // the whole record.
            noise: Noise::new(0xC1A9),
            gate: Sustain::new(sr, length, 0.0005, p.release),
        }
    }
}

impl Voice for Clap {
    fn add(&mut self, out: &mut [f32]) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let spread = (self.p.spread * SR) as u32;
        for s in out.iter_mut() {
            if self.left > 0 {
                if self.until_next == 0 {
                    // Fire: the burst restarts from full, and the last one
                    // also lights the tail.
                    self.burst = Decay::new(SR, BURST_TAU);
                    self.left -= 1;
                    self.until_next = spread;
                    if self.left == 0 {
                        self.tail_live = true;
                    }
                } else {
                    self.until_next -= 1;
                }
            }
            let mut env = self.burst.tick();
            if self.tail_live {
                env += 0.9 * self.tail.tick();
            }
            let y = self.band.tick(self.noise.tick(), Mode::Band) * env;
            *s += saturate(y, self.p.drive) * self.gate.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.gate.finished()
    }
}
