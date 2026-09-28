//! Mixing: the sidechain duck.

/// Duck one part's level every time another part plays — almost always the
/// kick ducking everything else.
///
/// Why it exists: the kick and the bass both want the bottom of the spectrum,
/// and two loud things in the same octave sound like mud rather than like two
/// things. Turning the bass down for a fraction of a second on each kick clears
/// room for it. The side effect became the point: the swell back up is the
/// **pump** you hear in most dance music, and how fast it swells sets the
/// groove. French house lives on an obvious one; this is set subtle, the way
/// Wata Igarashi uses it.
#[derive(Clone, Copy, Debug)]
pub struct Duck {
    /// The level ducked to, linear.
    floor: f32,
    attack: f32,
    release: f32,
    gain: f32,
    ducking: bool,
}

impl Duck {
    #[must_use]
    pub fn new(sr: f32, depth_db: f32, attack: f32, release: f32) -> Self {
        Self {
            floor: 10.0f32.powf(-depth_db / 20.0),
            // One-pole slew coefficients: reach 63% of the way in `attack`
            // seconds. Nothing steps, so nothing clicks.
            attack: 1.0 - (-1.0 / (attack.max(1e-5) * sr)).exp(),
            release: 1.0 - (-1.0 / (release.max(1e-5) * sr)).exp(),
            gain: 1.0,
            ducking: false,
        }
    }

    pub fn trigger(&mut self) {
        self.ducking = true;
    }

    pub fn tick(&mut self) -> f32 {
        if self.ducking {
            self.gain += (self.floor - self.gain) * self.attack;
            if self.gain - self.floor < 0.002 {
                self.ducking = false;
            }
        } else {
            self.gain += (1.0 - self.gain) * self.release;
        }
        self.gain
    }
}

/// A trance gate: chop a part's level in a rhythm, like a hand on a fader.
///
/// Not re-triggered notes — the pad keeps playing underneath and only its
/// *volume* is cut, so the swell and the swirl carry on through the holes. That
/// is what makes a gated pad sound like one instrument being interrupted rather
/// than a stab repeated.
///
/// `depth` is how hard it chops: 0 leaves the sound alone, 1 closes it fully
/// between hits. Creeping that from 0 to 1 over many bars is a build all by
/// itself, which is what the intro uses it for.
///
/// It is a pure function of the transport position — no state to get out of
/// step, and it stays locked to the grid when the tempo changes.
#[derive(Clone, Debug)]
pub struct Gate {
    /// One entry per cell of the pattern: true is open.
    open: Vec<bool>,
    /// Steps per cell. 1.0 is a sixteenth, the usual grid.
    steps_per_cell: f32,
    /// Share of an open cell that stays open.
    length: f32,
    attack: f32,
    release: f32,
    /// 0 no chop, 1 full chop.
    pub depth: f32,
}

impl Gate {
    /// From a drum grid: anything but `.` is an open step.
    #[must_use]
    pub fn new(pattern: &str, length: f32, depth: f32) -> Self {
        Self {
            open: pattern
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '|')
                .map(|c| c != '.')
                .collect(),
            steps_per_cell: 1.0,
            length,
            // Short ramps, or the chop itself clicks.
            attack: 0.003,
            release: 0.02,
            depth,
        }
    }

    /// The gain at this transport position.
    #[must_use]
    pub fn gain(&self, sample: u64, samples_per_step: f64, sr: f32) -> f32 {
        if self.open.is_empty() || self.depth <= 0.0 {
            return 1.0;
        }
        let cell_samples = samples_per_step * f64::from(self.steps_per_cell);
        let cell = (sample as f64 / cell_samples).floor();
        let within = (sample as f64 - cell * cell_samples) as f32 / sr;
        let cell_seconds = (cell_samples / f64::from(sr)) as f32;

        let index = (cell as i64).rem_euclid(self.open.len() as i64) as usize;
        let shape = if self.open[index] {
            let open_for = self.length * cell_seconds;
            let rise = (within / self.attack).clamp(0.0, 1.0);
            let fall = ((open_for - within) / self.release).clamp(0.0, 1.0);
            rise.min(fall)
        } else {
            0.0
        };
        1.0 - self.depth * (1.0 - shape)
    }
}
