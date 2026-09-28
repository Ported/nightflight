//! Envelopes. Nothing here starts or stops without a ramp, so nothing clicks.

/// An exponential fall from 1.0, with time constant `tau` seconds (37% left
/// after tau). `tick` returns the current value, then decays.
///
/// Held as a multiply rather than an `exp()` per sample: the same curve, a few
/// nanoseconds instead of twenty.
#[derive(Clone, Copy, Debug)]
pub struct Decay {
    level: f32,
    k: f32,
}

impl Decay {
    #[must_use]
    pub fn new(sr: f32, tau: f32) -> Self {
        Self {
            level: 1.0,
            k: (-1.0 / (tau.max(1e-6) * sr)).exp(),
        }
    }

    pub fn tick(&mut self) -> f32 {
        let now = self.level;
        self.level *= self.k;
        now
    }

    #[must_use]
    pub fn level(&self) -> f32 {
        self.level
    }
}

/// A note's amplitude: linear attack, full level for `hold`, exponential
/// `decay` while the note is held, then a raised-cosine release once `length`
/// is up.
#[derive(Clone, Copy, Debug)]
pub struct Amp {
    t: u32,
    attack: u32,
    hold: u32,
    gate: u32,
    release: u32,
    decay: Decay,
}

impl Amp {
    #[must_use]
    pub fn new(sr: f32, length: f32, attack: f32, hold: f32, decay: f32, release: f32) -> Self {
        Self {
            t: 0,
            attack: (attack * sr).max(1.0) as u32,
            hold: (hold * sr) as u32,
            gate: (length * sr) as u32,
            release: (release * sr).max(1.0) as u32,
            decay: Decay::new(sr, decay),
        }
    }

    pub fn tick(&mut self) -> f32 {
        let mut g = if self.t < self.hold {
            1.0
        } else {
            self.decay.tick()
        };
        if self.t < self.attack {
            g *= self.t as f32 / self.attack as f32;
        }
        if self.t >= self.gate {
            let x = (self.t - self.gate) as f32 / self.release as f32;
            g *= 0.5 * (1.0 + (std::f32::consts::PI * x.min(1.0)).cos());
        }
        self.t += 1;
        g
    }

    #[must_use]
    pub fn finished(&self) -> bool {
        self.t >= self.gate + self.release
    }
}

/// 0 to 1 over `rise` seconds on a raised cosine, then held: a slow fade-in.
#[derive(Clone, Copy, Debug)]
pub struct Swell {
    t: u32,
    rise: u32,
}

impl Swell {
    #[must_use]
    pub fn new(sr: f32, rise: f32) -> Self {
        Self {
            t: 0,
            rise: (rise * sr).max(1.0) as u32,
        }
    }

    pub fn tick(&mut self) -> f32 {
        let x = (self.t as f32 / self.rise as f32).min(1.0);
        self.t = self.t.saturating_add(1);
        0.5 - 0.5 * (std::f32::consts::PI * x).cos()
    }
}

/// A sustaining envelope: swell in, hold, fade out. No attack transient at all,
/// just arrival — which is what separates a pad from every other instrument
/// here. A struck sound needs to tell you *when* it happened; a pad needs you
/// not to notice it starting.
#[derive(Clone, Copy, Debug)]
pub struct Sustain {
    t: u32,
    attack: u32,
    gate: u32,
    release: u32,
}

impl Sustain {
    #[must_use]
    pub fn new(sr: f32, length: f32, attack: f32, release: f32) -> Self {
        Self {
            t: 0,
            attack: (attack * sr).max(1.0) as u32,
            gate: (length * sr) as u32,
            release: (release * sr).max(1.0) as u32,
        }
    }

    pub fn tick(&mut self) -> f32 {
        let rise = (self.t as f32 / self.attack as f32).min(1.0);
        let mut g = 0.5 - 0.5 * (std::f32::consts::PI * rise).cos();
        if self.t >= self.gate {
            let fall = ((self.t - self.gate) as f32 / self.release as f32).min(1.0);
            g *= 0.5 + 0.5 * (std::f32::consts::PI * fall).cos();
        }
        self.t += 1;
        g
    }

    #[must_use]
    pub fn finished(&self) -> bool {
        self.t >= self.gate + self.release
    }
}
