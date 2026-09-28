//! A reverb built from recirculating delays, not from a recording.
//!
//! What a reverb is: after the first few reflections, a room's answer to a sound
//! is so dense that it is statistically just noise dying away — faster in the
//! highs, which air and soft furnishings soak up, than in the lows. The Python
//! studio makes exactly that, as an explicit impulse response: octave bands of
//! noise, each decaying at its own rate, then convolved with the send bus.
//!
//! That cannot be done live. At RT60 7 s the response is 370,000 samples per
//! ear, so convolving it directly is some 35 billion multiply-adds a second,
//! and doing it with big FFTs — which is what makes it cheap offline — needs a
//! chunk of input buffered before it can start. The audio callback has 2.7 ms.
//!
//! So this generates the same statistics instead of replaying a recording of
//! them: a **feedback delay network**. Eight delay lines of mutually prime
//! length recirculate through an orthogonal mixing matrix, each losing a little
//! on every pass and a little more in the highs than the lows. Mutually prime
//! matters: lines whose lengths share factors reinforce each other at the
//! common period and the tail develops an audible pitch.
//!
//! It is not a compromise here, because the Python's impulse response is
//! *synthetic* too — there is no real cathedral being faithfully reproduced, so
//! there is nothing to be unfaithful to. Two things get better: the decay can
//! be changed while the tail is still ringing, which a fixed response cannot
//! do, and there is no overlap-add bookkeeping across render windows, because a
//! recirculating network never stopped.

use crate::filter::{Mode, OnePole, Svf};

/// Eight lines, 35 to 85 ms, all prime.
const LINES: [usize; 8] = [1693, 2017, 2411, 2789, 3169, 3511, 3793, 4091];
/// Four short allpasses that smear the input before it enters the network.
const DIFFUSERS: [usize; 4] = [113, 197, 331, 439];
const DIFFUSION: f32 = 0.65;
/// Where the damping curve is anchored: below this, decay is full length.
const DAMPING_PIVOT: f32 = 500.0;
/// The top of the range the damping curve is fitted over.
const DAMPING_TOP: f32 = 16_000.0;
/// Corner frequencies of the three damping shelves, two octaves apart.
///
/// The target asks for a loss that grows by a *constant number of dB per
/// octave*: RT60 falls as 1/(1 + damping x octaves), so the per-pass loss in dB
/// rises linearly in log frequency, over five octaves from 500 Hz to 16 kHz.
///
/// No single filter of low order does that. A plain lowpass is flat and then
/// falls at 6 dB an octave, and fitting one to a *small* loss at a *low*
/// frequency puts its corner so high that by 8 kHz it has taken out far too
/// much — measured, that gave 0.5 s at 8 kHz where 2 s was asked for. A
/// **shelf** levels off instead: past its corner it settles at a fixed loss. So
/// three shelves, each taking a third of the total in dB, tile the range and
/// approximate the straight line without overshooting past it.
const DAMPING_CORNERS: [f32; 4] = [1_500.0, 4_000.0, 9_000.0, 20_000.0];
const DAMPING_POLES: usize = DAMPING_CORNERS.len();

/// An integer delay of exactly `len` samples.
struct Ring {
    buffer: Vec<f32>,
    index: usize,
}

impl Ring {
    fn new(len: usize) -> Self {
        Self {
            buffer: vec![0.0; len.max(1)],
            index: 0,
        }
    }

    fn peek(&self) -> f32 {
        self.buffer[self.index]
    }

    fn push(&mut self, x: f32) {
        self.buffer[self.index] = x;
        self.index += 1;
        if self.index == self.buffer.len() {
            self.index = 0;
        }
    }
}

/// A Schroeder allpass: it delays every frequency by a different amount but
/// passes them all at the same level. Four in series turn a click into a dense
/// burst, which is what makes the start of the tail smooth rather than a
/// handful of discrete slaps. This is what replaces the Python response's 50 ms
/// fade-in.
struct Allpass {
    ring: Ring,
    gain: f32,
}

impl Allpass {
    fn tick(&mut self, x: f32) -> f32 {
        let delayed = self.ring.peek();
        let v = x + self.gain * delayed;
        self.ring.push(v);
        delayed - self.gain * v
    }
}

/// One recirculating line: a delay, a loss, and a little more loss up high.
struct Line {
    ring: Ring,
    /// Loss per pass, chosen so the line alone would decay 60 dB in RT60.
    gain: f32,
    damping: [Shelf; DAMPING_POLES],
}

/// A first-order shelf: unity gain below its corner, `high` above it.
///
/// Built from a lowpass, which is all it takes: `high * x + (1 - high) *
/// lowpass(x)` is exactly 1 at DC, exactly `high` where the lowpass has died,
/// and 6 dB an octave in between.
#[derive(Clone, Copy, Debug)]
struct Shelf {
    lowpass: OnePole,
    corner: f32,
    high: f32,
}

impl Shelf {
    fn tick(&mut self, x: f32) -> f32 {
        self.high * x + (1.0 - self.high) * self.lowpass.tick(x, self.corner)
    }
}

pub struct Reverb {
    predelay: Ring,
    lowcut: Option<Svf>,
    diffusers: [Allpass; 4],
    lines: [Line; 8],
    output_gain: f32,
    sr: f32,
}

impl Reverb {
    /// `rt60` is the decay time in seconds at low frequencies; `damping` is how
    /// much faster the highs go (0 means not at all, 1 halves it every octave
    /// above 500 Hz); `predelay` is the silence before the tail begins, which
    /// is the ear's cue for how big the space is; `lowcut` keeps sub-bass out,
    /// because reverberant lows are just mud.
    #[must_use]
    pub fn new(sr: f32, rt60: f32, damping: f32, predelay: f32, lowcut: f32) -> Self {
        let rt60 = rt60.max(0.1);
        // The curve being aimed at, the same one the Python studio uses:
        // rt60 / (1 + damping * octaves above 500 Hz).
        let target_rt60 = |hz: f32| {
            let octaves = (hz / DAMPING_PIVOT).log2().max(0.0);
            (rt60 / (1.0 + damping * octaves)).max(0.05)
        };

        let lines = std::array::from_fn(|i| {
            let len = LINES[i];
            let loss = |seconds: f32| 10.0f32.powf(-3.0 * len as f32 / (seconds * sr));
            let at_dc = loss(rt60);

            // The whole extra loss wanted at the top of the range, shared
            // equally in dB between the shelves.
            let total = (loss(target_rt60(DAMPING_TOP)) / at_dc).clamp(1e-4, 1.0);
            let share = total.powf(1.0 / DAMPING_POLES as f32);

            Line {
                ring: Ring::new(len),
                gain: at_dc,
                damping: std::array::from_fn(|pole| Shelf {
                    lowpass: OnePole::default(),
                    corner: OnePole::coefficient(sr, DAMPING_CORNERS[pole]),
                    high: share,
                }),
            }
        });

        let mut reverb = Self {
            predelay: Ring::new(((predelay * sr) as usize).max(1)),
            lowcut: (lowcut > 0.0).then(|| Svf::new(sr, lowcut, 0.0)),
            diffusers: std::array::from_fn(|i| Allpass {
                ring: Ring::new(DIFFUSERS[i]),
                gain: DIFFUSION,
            }),
            lines,
            output_gain: 1.0,
            sr,
        };
        reverb.output_gain = 1.0 / reverb.measure_energy(rt60).max(1e-9);
        reverb
    }

    /// Run an impulse through and return the root energy of one ear, then reset.
    ///
    /// The Python normalises its response to unit energy so that a send of 1 is
    /// as loud as the dry signal. Doing the same here by measurement rather than
    /// by algebra costs a few milliseconds at load and holds whatever the
    /// parameters are. It is also why `rt60` cannot yet be changed while
    /// playing: the calibration would have to run again, and not on the audio
    /// thread.
    fn measure_energy(&mut self, rt60: f32) -> f32 {
        let n = ((rt60 * 1.2 + 0.1) * self.sr) as usize;
        let (mut left, mut right) = (0.0f64, 0.0f64);
        for i in 0..n {
            let (l, r) = self.tick(if i == 0 { 1.0 } else { 0.0 });
            left += f64::from(l) * f64::from(l);
            right += f64::from(r) * f64::from(r);
        }
        let energy = (0.5 * (left + right)).sqrt() as f32;
        self.reset();
        energy
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.ring.buffer.fill(0.0);
            for shelf in &mut line.damping {
                shelf.lowpass = OnePole::default();
            }
        }
        for diffuser in &mut self.diffusers {
            diffuser.ring.buffer.fill(0.0);
        }
        self.predelay.buffer.fill(0.0);
    }

    /// One sample in, two ears out. The output gain is applied by the caller's
    /// mix, so this is the raw wet signal.
    fn tick(&mut self, input: f32) -> (f32, f32) {
        // Predelay, then the lowcut, then diffusion.
        let delayed = self.predelay.peek();
        self.predelay.push(input);
        let mut x = match &mut self.lowcut {
            Some(filter) => filter.tick(delayed, Mode::High),
            None => delayed,
        };
        for diffuser in &mut self.diffusers {
            x = diffuser.tick(x);
        }

        // Read every line, then mix, then write back: the network is a single
        // simultaneous step, not a chain.
        let mut taps = [0.0f32; 8];
        for (tap, line) in taps.iter_mut().zip(self.lines.iter()) {
            *tap = line.ring.peek();
        }

        let mixed = hadamard(taps);
        // Alternating signs on the way in, so a single click does not arrive at
        // all eight lines as the same waveform.
        for (i, line) in self.lines.iter_mut().enumerate() {
            let injected = if i % 2 == 0 { x } else { -x } * INPUT_SCALE;
            let mut fed = mixed[i];
            for shelf in &mut line.damping {
                fed = shelf.tick(fed);
            }
            line.ring.push(fed * line.gain + injected);
        }
        // Two orthogonal rows of the same matrix: different combinations of the
        // same differently-delayed signals, so the ears decorrelate. The Python
        // gets this from using different noise in each ear; without that, it has
        // to be arranged.
        let mut left = 0.0;
        let mut right = 0.0;
        for i in 0..8 {
            left += LEFT_MIX[i] * taps[i];
            right += RIGHT_MIX[i] * taps[i];
        }
        (left * INPUT_SCALE, right * INPUT_SCALE)
    }

    /// Add the wet signal for `bus` into the two ears, scaled by `gain`.
    pub fn process(&mut self, bus: &[f32], left: &mut [f32], right: &mut [f32], gain: f32) {
        let scale = self.output_gain * gain;
        for (i, &x) in bus.iter().enumerate() {
            let (l, r) = self.tick(x);
            left[i] += l * scale;
            right[i] += r * scale;
        }
    }
}

/// 1/sqrt(8): what keeps the mixing matrix lossless.
const INPUT_SCALE: f32 = std::f32::consts::FRAC_1_SQRT_2 * 0.5;
const LEFT_MIX: [f32; 8] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
const RIGHT_MIX: [f32; 8] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];

/// An 8-point Walsh-Hadamard transform: every output is a sum of every input
/// with some signs, and the whole thing is its own inverse up to a scale. It is
/// the cheapest orthogonal mixing there is — 24 additions, no multiplies — and
/// orthogonal is what makes the network lose energy only where we intend it to.
fn hadamard(mut x: [f32; 8]) -> [f32; 8] {
    let mut step = 1;
    while step < 8 {
        let mut i = 0;
        while i < 8 {
            for j in i..i + step {
                let (a, b) = (x[j], x[j + step]);
                x[j] = a + b;
                x[j + step] = a - b;
            }
            i += step * 2;
        }
        step *= 2;
    }
    for v in &mut x {
        *v *= INPUT_SCALE;
    }
    x
}
