//! Patterns and lanes: who plays what, and for how long.
//!
//! A pattern is any number of steps, not necessarily sixteen. That is
//! deliberate: a loop whose length does not divide the bar drifts against it
//! and only lines up again every few bars, which is most of what makes
//! hypnotic techno hypnotic. The Python studio calls it polymeter and gets it
//! from `grid()`; here it is just the length of a slice.

use dsp::inst::{bass, glass, hat, kick, strings};
use dsp::space::Position;

use crate::automation::{Flight, Macro, Play, Transition};
use crate::mix::Gate;

/// One step of a pattern. Velocity 0 is a rest.
#[derive(Clone, Copy, Debug, Default)]
pub struct Step {
    pub velocity: f32,
    /// Semitones above the lane's root, for pitched lanes.
    pub offset: i8,
}

impl Step {
    pub const REST: Self = Self {
        velocity: 0.0,
        offset: 0,
    };

    #[must_use]
    pub fn new(velocity: f32, offset: i8) -> Self {
        Self { velocity, offset }
    }
}

/// A looping sequence of steps. Built once, when a set is loaded.
#[derive(Clone, Debug)]
pub struct Pattern {
    steps: Vec<Step>,
}

impl Pattern {
    /// From a drum grid, the notation the Python studio uses: `X` accent,
    /// `x` hit, `o` ghost, `.` rest. Spaces and bar lines are ignored.
    ///
    /// # Panics
    /// If the grid contains a character that is not one of those.
    #[must_use]
    pub fn grid(pattern: &str) -> Self {
        let steps = pattern
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '|')
            .map(|c| match c {
                'X' => Step::new(1.0, 0),
                'x' => Step::new(0.7, 0),
                'o' => Step::new(0.4, 0),
                '.' => Step::REST,
                other => panic!("{other:?} is not a grid character (X x o .)"),
            })
            .collect();
        Self { steps }
    }

    #[must_use]
    pub fn steps(steps: Vec<Step>) -> Self {
        Self { steps }
    }

    /// The step at this position of the running transport, looping at the
    /// pattern's own length.
    #[must_use]
    pub fn at(&self, step: u64) -> Step {
        if self.steps.is_empty() {
            return Step::REST;
        }
        self.steps[(step % self.steps.len() as u64) as usize]
    }

    /// Change one step. Allocation-free: the slot already exists, which is why
    /// an editor sends one step rather than a whole clip.
    pub fn set(&mut self, index: usize, step: Step) {
        if let Some(slot) = self.steps.get_mut(index) {
            *slot = step;
        }
    }

    /// Every step, for an interface that wants to draw the loop.
    #[must_use]
    pub fn all(&self) -> &[Step] {
        &self.steps
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// How long a note lasts, and the distinction matters musically.
///
/// A drum's ring is a physical fact: a kick drum does not decay faster because
/// the record is faster, so its length is in **seconds**. A note that should
/// last "a sixteenth" is in **steps**, and follows the tempo. Getting this
/// wrong is what cut the Python beat sketch's kick to 0.12 s and made it feel
/// wimpy — it had been given a one-step length.
#[derive(Clone, Copy, Debug)]
pub enum Length {
    Seconds(f32),
    Steps(f32),
}

impl Length {
    #[must_use]
    pub fn seconds(self, samples_per_step: f64, sr: f32) -> f32 {
        match self {
            Self::Seconds(s) => s,
            Self::Steps(n) => n * (samples_per_step / f64::from(sr)) as f32,
        }
    }
}

impl Voicing {
    /// Which instrument this is, for an interface to label and for a preset to
    /// be filed under.
    #[must_use]
    pub fn instrument(self) -> &'static str {
        match self {
            Self::Kick(_) => "kick",
            Self::Hat(_) => "hat",
            Self::Bass(_) => "bass",
            Self::Glass(_) => "glass",
            Self::Strings(_) => "strings",
        }
    }
}

/// Which instrument a lane plays, and how it is set up.
#[derive(Clone, Copy, Debug)]
pub enum Voicing {
    Kick(kick::Params),
    Hat(hat::Params),
    Bass(bass::Params),
    Glass(glass::Params),
    Strings(strings::Params),
}

/// A lane: one instrument, one pattern, one fader.
#[derive(Clone, Debug)]
pub struct Lane {
    pub name: &'static str,
    /// Which clip this lane belongs to. Lanes of one clip are edited together
    /// and drawn as one row on the timeline: "kick" is a lane of "beat".
    pub clip: &'static str,
    pub voicing: Voicing,
    pub pattern: Pattern,
    /// Linear level, relative to the other lanes.
    pub gain: f32,
    pub length: Length,
    /// MIDI note the pattern's offsets are relative to.
    pub root: f32,
    /// Where it sits around the head.
    pub home: Home,
    /// How much of this lane goes to the shared reverb, 0 for dry.
    pub send: f32,
    /// A rhythmic chop on this lane's level, if any.
    pub gate: Option<Gate>,
    /// When this lane plays, in bars. Empty means always — which is what a
    /// looping set for jamming wants, and a score fills in.
    pub spans: Vec<Play>,
    /// A multiplier on the velocity of notes started from now on. Macros ride
    /// this; nothing else touches it.
    pub velocity_scale: f32,
    /// Whether the kick ducks this lane (the sidechain pump).
    pub ducked: bool,
    pub muted: bool,
}

/// Where a lane sits, described rather than baked.
///
/// The Python scene stores a path as a list of keyframes, computed once by the
/// composition. Live, the description has to survive: you want to speed an
/// orbit up or throw a source somewhere while it plays, and you cannot do that
/// to a list of points. So a home is evaluated at the musical time it is needed.
#[derive(Clone, Copy, Debug)]
pub enum Home {
    /// Not placed: mixed to the centre, untouched. Right for kick and bass —
    /// low frequencies barely localise, and a head filter would only colour
    /// them.
    Centre,
    /// A circle at head height, one lap every `bars_per_lap`, clockwise seen
    /// from above (front, right, behind, left). `phase` is in laps, so two
    /// sources half a lap apart chase each other.
    ///
    /// The angle comes from an accumulated lap count, not from
    /// `bar / bars_per_lap`. That matters the moment the rate can change: with
    /// the angle computed from the bar, doubling the speed at bar 10 also
    /// doubles the accumulated angle, and the source teleports to the other side
    /// of the head. Integrating the rate instead means a change in speed is only
    /// ever a change in speed.
    Orbit {
        radius: f32,
        bars_per_lap: f32,
        phase: f32,
        /// Metres above or below the ears, held. Spreading several voices
        /// vertically keeps them from piling up in one place.
        elevation: f32,
        /// Metres of rise and fall on top of that, on its own cycle: a circle
        /// that also breathes vertically stops sounding like a machine.
        height: f32,
        height_bars: f32,
    },
}

impl Home {
    /// Laps per bar, for integrating the orbit's angle.
    #[must_use]
    pub fn laps_per_bar(self) -> f64 {
        match self {
            Self::Centre => 0.0,
            Self::Orbit { bars_per_lap, .. } => 1.0 / f64::from(bars_per_lap).max(0.01),
        }
    }

    /// Where the source is, given how many laps it has turned and what bar it
    /// is. `None` means it is not placed at all.
    #[must_use]
    pub fn at(self, laps: f64, bar: f64) -> Option<Position> {
        match self {
            Self::Centre => None,
            Self::Orbit {
                radius,
                phase,
                elevation,
                height,
                height_bars,
                ..
            } => {
                let angle = std::f64::consts::TAU * (laps + f64::from(phase));
                let rise = std::f64::consts::TAU * bar / f64::from(height_bars);
                Some(Position::new(
                    radius * angle.sin() as f32,
                    elevation + height * rise.sin() as f32,
                    -radius * angle.cos() as f32,
                ))
            }
        }
    }
}

/// The shared reverb's settings, as the scene JSON describes them.
#[derive(Clone, Copy, Debug)]
pub struct ReverbSettings {
    /// Seconds for the tail to fall 60 dB at low frequencies: bedroom ~0.5,
    /// cathedral 6-10.
    pub rt60: f32,
    /// How much faster the highs die. 0 leaves the tail as bright as the
    /// source; higher is warmer. Gentler per unit than the Python's damping —
    /// about 1.6 here for what 1.0 does there.
    pub damping: f32,
    /// Seconds of silence before the tail arrives: the ear's cue for how big
    /// the space is.
    pub predelay: f32,
    /// Hz. Nothing below this reaches the reverb, because reverberant sub-bass
    /// is only mud.
    pub lowcut: f32,
}

/// Everything the engine needs to play a piece.
pub struct Set {
    pub lanes: Vec<Lane>,
    pub reverb: Option<ReverbSettings>,
    /// The conductor's faders.
    pub macros: Vec<Macro>,
    /// How long the piece is, in bars. A looping set for jamming has no real
    /// end; this is then just how far the scrub bar reaches.
    pub length_bars: f32,
}

impl Lane {
    /// How loud the score says this lane is at `bar`, or `None` if it is not
    /// playing. A lane with no spans always plays.
    #[must_use]
    pub fn scored(&self, bar: f64) -> Option<f32> {
        if self.spans.is_empty() {
            return Some(1.0);
        }
        self.spans
            .iter()
            .filter_map(|s| s.level(bar))
            .reduce(f32::max)
    }

    /// If this lane is in the air right now, the flight and the bar it lands on.
    #[must_use]
    pub fn flying(&self, bar: f64) -> Option<(Flight, f32)> {
        self.spans.iter().find_map(|s| s.approach(bar))
    }

    /// Whether any of this lane's spans ever puts it in the air. Decided at
    /// load, because it settles whether the lane needs a delay line.
    #[must_use]
    pub fn ever_flies(&self) -> bool {
        self.spans
            .iter()
            .any(|s| matches!(s.enter, Transition::Fly(_)))
    }
}
