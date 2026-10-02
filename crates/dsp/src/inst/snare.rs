//! Drum-machine snare: a tuned thump and the rattle of wires, each on its own
//! clock.
//!
//! The snare is the drum that answers the kick — the backbeat, beats 2 and 4 —
//! and it is two sounds happening at once:
//!
//! * **the drum** — a shell with a head whose vibration modes share no common
//!   fundamental. Two sines stand in for them, the 808's way: its pair sat near
//!   180 and 330 Hz, and 1.83x is not an octave or a fifth of anything, so the
//!   ear hears a thump with no note worth naming. A touch of pitch sweep puts
//!   the skin's tightening-on-impact into the attack.
//! * **the wires** — the *snares* that give the drum its name: coiled wires
//!   stretched against the bottom head, rattling against it for a moment after
//!   every hit. Highpassed noise is the classic stand-in; the 808 labels the
//!   knob for it "snappy". This layer is the crack that cuts through a mix.
//!
//! The two layers decay independently, and that pair of times is most of what
//! separates one snare from another: short drum and short wires is a tight
//! electro crack, long wires over a short drum is the 909's hiss, both long is
//! a marching drum in a hall. Listen for the moment the wires outlast the
//! thump — that tail is what "snappy" means.

use crate::env::{Decay, Sustain};
use crate::filter::{Mode, Svf};
use crate::noise::Noise;
use crate::osc::{Phasor, sine};
use crate::parameters;
use crate::shape::saturate;
use crate::{Voice, hz};

/// The head's two modes. The lower sits on the lane's pitch; the upper rides
/// at 1.83x, the 808's inharmonic ratio (330 over 180 Hz).
const MODES: [f32; 2] = [1.0, 1.83];

/// The upper mode is quieter: it colours the thump rather than competing.
const LEVELS: [f32; 2] = [1.0, 0.6];

parameters! {
    /// The drum and the wires, each with its own decay.
    pub struct Params {
        /// Starts this many times above the note's pitch, then falls to it.
        sweep: lin 1.0..=6.0 = 1.5, "x";
        /// Seconds: how fast that fall happens.
        pitch_decay: log 0.005..=0.1 = 0.02, "s";
        /// Seconds: the drum's decay. Short is tight; long is a floppy tom-like thud.
        decay: log 0.03..=0.6 = 0.12, "s";
        /// Level of the wires. The 808 calls this knob "snappy".
        snap: lin 0.0..=2.0 = 1.0, "";
        /// Seconds: how long the wires rattle on after the hit.
        snap_decay: log 0.02..=0.5 = 0.09, "s";
        /// Hz: the wires' noise is kept above this.
        snap_tone: log 400.0..=8000.0 = 1800.0, "Hz";
        /// Saturation amount.
        drive: lin 0.5..=8.0 = 1.5, "";
        /// Seconds: the fade when choked, that is, when the note ends.
        release: log 0.001..=0.05 = 0.005, "s";
    }
}

pub struct Snare {
    sr: f32,
    p: Params,
    base: f32,
    velocity: f32,
    phases: [Phasor; 2],
    sweep: Decay,
    body: Decay,
    snap: Decay,
    highpass: Svf,
    noise: Noise,
    /// The gate, not the shape: the layers above carry their own decays, and
    /// this only opens over half a millisecond and closes the note. The kick
    /// shares one envelope between its layers because its click is faster than
    /// its body by design; here the wires must be free to outlast the drum, so
    /// each layer gets its own clock and the gate stays out of the way.
    gate: Sustain,
}

impl Snare {
    /// A snare at `pitch` (MIDI): the lower head mode lands on that note.
    #[must_use]
    pub fn new(sr: f32, pitch: f32, velocity: f32, length: f32, p: Params) -> Self {
        Self {
            sr,
            p,
            base: hz(pitch),
            velocity,
            phases: std::array::from_fn(|_| Phasor::default()),
            sweep: Decay::new(sr, p.pitch_decay),
            body: Decay::new(sr, p.decay),
            snap: Decay::new(sr, p.snap_decay),
            highpass: Svf::new(sr, p.snap_tone, 0.0),
            // The same noise every hit, like a machine: a drum machine's wires
            // are a fixed circuit, not a new rattle each time.
            noise: Noise::new(0x57A2E),
            gate: Sustain::new(sr, length, 0.0005, p.release),
        }
    }
}

impl Voice for Snare {
    fn add(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            // Both modes bend together: it is one head tightening.
            let bend = 1.0 + (self.p.sweep - 1.0) * self.sweep.tick();
            let drum = self.body.tick();
            let mut y = 0.0;
            for ((phase, ratio), level) in self.phases.iter_mut().zip(MODES).zip(LEVELS) {
                y += level * sine(phase.tick(self.base * ratio * bend, self.sr)) * drum;
            }

            let n = self.highpass.tick(self.noise.tick(), Mode::High);
            y += self.p.snap * n * self.snap.tick();

            *s += saturate(y, self.p.drive) * self.gate.tick() * self.velocity;
        }
    }

    fn finished(&self) -> bool {
        self.gate.finished()
    }
}
