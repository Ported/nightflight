//! Placing a sound around the head.
//!
//! Two cues do most of the work, and both come from having two ears a head's
//! width apart:
//!
//! * **ITD**, the interaural time difference. Sound arriving from the right
//!   reaches the right ear first, by up to about 0.8 ms. Below roughly 1.5 kHz
//!   the ear can compare the *phase* of the two arrivals, and this is the
//!   dominant cue for direction.
//! * **ILD**, the interaural level difference. The head is in the way, so the
//!   far ear hears less — and much less at high frequencies than at low ones,
//!   because long waves bend around an obstacle the size of a head and short
//!   ones do not. Above about 1.5 kHz this becomes the dominant cue.
//!
//! Plus distance: sound falls as 1/r, arrives later the further it has come
//! (which gives Doppler for free when it moves), and loses its highs to the air.
//!
//! **What this does not do yet:** the pinna. The folds of the outer ear filter
//! sound differently depending on where it comes from, and that is the only cue
//! for front-versus-back and for height. So with ITD and ILD alone, a source
//! ahead and the same source behind are identical, and a source overhead sounds
//! like one straight ahead. Fixing that means a measured head response (HRTF),
//! which is the next step in the plan.

use crate::filter::OnePole;

/// m/s.
pub const SPEED_OF_SOUND: f32 = 343.0;
/// Effective head radius, 10.7 cm — the value the Python studio fitted to the
/// KU100 dummy head's measured delays, to 0.03 ms rms.
pub const HEAD_RADIUS: f32 = 0.107;
/// 1/r would explode inside the head.
pub const MIN_DISTANCE: f32 = 0.25;
/// Unity gain at one metre.
pub const REFERENCE_DISTANCE: f32 = 1.0;
/// The air lowpass sits at 10 kHz at this distance, ~4 kHz at 40 m.
pub const AIR_DISTANCE: f32 = 10.0;
/// Longest delay the line can hold: 170 ms, about 58 m of travel.
const DELAY_LEN: usize = 8192;

/// Metres, listener at the origin, +x right, +y up, −z ahead. The same
/// convention as the scene JSON and the web page, so positions pass straight
/// through.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl std::ops::Add for Position {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Position {
    #[must_use]
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub fn distance(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z)
            .sqrt()
            .max(MIN_DISTANCE)
    }
}

/// Interaural time difference of a spherical head, seconds. Positive means the
/// source is to the right, so the **left** ear hears it later.
///
/// Woodworth's model: the extra distance to the far ear is the straight part
/// plus the arc around the head, which comes to `(a/c)(θ + sin θ)`. The Python
/// studio uses it rather than its own measurements because measured delays are
/// slightly noisy from one direction to the next, and a moving source turns
/// that noise into an audible pitch wobble.
#[must_use]
pub fn woodworth(lateral_sine: f32) -> f32 {
    let theta = lateral_sine.clamp(-1.0, 1.0).asin();
    HEAD_RADIUS / SPEED_OF_SOUND * (theta + theta.sin())
}

/// A delay line read at a fractional number of samples.
///
/// The interpolation is 4-point Lagrange rather than linear for one specific
/// reason: linear interpolation is itself a lowpass, losing several dB at the
/// top of the spectrum, and it varies with the fractional part — so a source
/// moving smoothly would shimmer. Hats live entirely in that top octave.
pub struct Delay {
    buffer: Vec<f32>,
    mask: usize,
    write: usize,
}

impl Delay {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: vec![0.0; DELAY_LEN],
            mask: DELAY_LEN - 1,
            write: 0,
        }
    }

    pub fn push(&mut self, x: f32) {
        self.buffer[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }

    /// The sample `delay` ago, interpolated. `delay` is clamped to leave room
    /// for the interpolator's neighbours at both ends.
    #[must_use]
    pub fn read(&self, delay: f32) -> f32 {
        let delay = delay.clamp(2.0, (DELAY_LEN - 4) as f32);
        let whole = delay.floor();
        // Position between the two middle taps, counted forwards in time.
        let mu = 1.0 - (delay - whole);
        let i = whole as usize;
        let at = |back: usize| self.buffer[(self.write + DELAY_LEN - 1 - back) & self.mask];
        let (a, b, c, d) = (at(i + 2), at(i + 1), at(i), at(i - 1));
        -mu * (mu - 1.0) * (mu - 2.0) / 6.0 * a + (mu + 1.0) * (mu - 1.0) * (mu - 2.0) / 2.0 * b
            - (mu + 1.0) * mu * (mu - 2.0) / 2.0 * c
            + (mu + 1.0) * mu * (mu - 1.0) / 6.0 * d
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything a position implies, ready to interpolate across a block.
#[derive(Clone, Copy, Debug)]
struct Target {
    delay: [f32; 2],
    gain: [f32; 2],
    air: f32,
    shadow: [f32; 2],
    /// Travel delay alone, with no interaural difference: what the room hears.
    send_delay: f32,
    /// The reverb feed falls as 1/sqrt(r) while the direct sound falls as 1/r,
    /// so a distant source is mostly reverb. That divergence is the strongest
    /// cue of distance there is.
    send_gain: f32,
}

/// Where a source is over one control period, and which slice of that period
/// this call covers.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub from: Position,
    pub to: Position,
    /// Samples into the control period that this call begins at.
    pub offset: usize,
    /// Length of the control period.
    pub period: usize,
}

/// Where a placed source's sound goes: two ears, and the room.
pub struct Ears<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
    /// The shared reverb bus.
    pub send: &'a mut [f32],
    /// How much of this source reaches it.
    pub send_level: f32,
}

fn target(position: Position, sr: f32) -> Target {
    let r = position.distance();
    // How far to the side, -1 (hard left) to +1 (hard right).
    let lateral = (position.x / r).clamp(-1.0, 1.0);
    let lag = woodworth(lateral) * sr;
    let travel = r / SPEED_OF_SOUND * sr;
    let distance_gain = REFERENCE_DISTANCE / r;

    let mut t = Target {
        // The ear on the far side hears it later, by the whole ITD.
        delay: [travel + lag.max(0.0), travel + (-lag).max(0.0)],
        gain: [0.0; 2],
        air: OnePole::coefficient(sr, 20_000.0 / (1.0 + r / AIR_DISTANCE)),
        shadow: [0.0; 2],
        send_delay: travel,
        send_gain: (REFERENCE_DISTANCE / r).sqrt(),
    };
    for (ear, sign) in [(0, -1.0), (1, 1.0)] {
        // 1 when the source is directly off this ear, 0 when the head is
        // squarely in the way, 0.5 straight ahead or behind.
        let exposure = 0.5 + 0.5 * sign * lateral;
        // Broadband shadowing is mild: about -14 dB fully occluded.
        t.gain[ear] = distance_gain * (0.2 + 0.8 * exposure);
        // Frequency-dependent shadowing is not mild, and it is what makes the
        // difference between "louder on one side" and "over there": the far
        // ear loses its top end, from 20 kHz down to 2 kHz.
        t.shadow[ear] = OnePole::coefficient(sr, 2_000.0 * 10.0f32.powf(exposure));
    }
    t
}

/// Places one source's mono signal around the head.
///
/// One of these per *source*, not per voice: a source is the thing that moves,
/// and all its notes move with it.
pub struct Placer {
    delay: Delay,
    air: OnePole,
    shadow: [OnePole; 2],
}

impl Placer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            delay: Delay::new(),
            air: OnePole::default(),
            shadow: [OnePole::default(); 2],
        }
    }

    /// Add `mono`, placed on the path from `from` to `to`, into the two ears.
    ///
    /// Everything a position implies — two delays, two gains, three filter
    /// coefficients — is computed at each end of the *control period* and
    /// interpolated per sample in between. Nothing steps, so a source can cross
    /// the head at any speed without zippering, and because the delay follows
    /// the distance continuously, Doppler comes out for free.
    ///
    /// `motion` carries the positions at the two ends of the control period and
    /// how far into it this call begins. Taking the
    /// phase from absolute time rather than from the length of this particular
    /// buffer is what makes the result identical however the audio device
    /// chops time up — a control period split into ten pieces interpolates the
    /// same trajectory as one processed whole.
    /// Forget everything in flight. After the transport jumps, the delay line
    /// holds up to 170 ms of the place we just left.
    pub fn reset(&mut self) {
        self.delay = Delay::new();
        self.air = OnePole::default();
        self.shadow = [OnePole::default(); 2];
    }

    pub fn place(&mut self, mono: &[f32], motion: Motion, out: &mut Ears<'_>, sr: f32) {
        let start = target(motion.from, sr);
        let end = target(motion.to, sr);
        let scale = 1.0 / motion.period as f32;

        for (i, &x) in mono.iter().enumerate() {
            let f = (motion.offset + i) as f32 * scale;
            let lerp = |a: f32, b: f32| a + (b - a) * f;

            let dulled = self.air.tick(x, lerp(start.air, end.air));
            self.delay.push(dulled);

            for (ear, ear_out) in [(0usize, &mut *out.left), (1usize, &mut *out.right)] {
                let heard = self.delay.read(lerp(start.delay[ear], end.delay[ear]));
                let shadowed =
                    self.shadow[ear].tick(heard, lerp(start.shadow[ear], end.shadow[ear]));
                ear_out[i] += shadowed * lerp(start.gain[ear], end.gain[ear]);
            }

            if out.send_level > 0.0 {
                let to_room = self.delay.read(lerp(start.send_delay, end.send_delay));
                out.send[i] += to_room * lerp(start.send_gain, end.send_gain) * out.send_level;
            }
        }
    }
}

impl Default for Placer {
    fn default() -> Self {
        Self::new()
    }
}
