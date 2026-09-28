//! Curves, spans and macros: the conductor's side of the engine.
//!
//! Three ideas, and between them they turn a set of loops into a piece.
//!
//! * A **span** says when a lane plays, in bars, with a transition at each end.
//!   Parts know *how* to play; the score says *when*.
//! * A **curve** is a value against bars — straight lines between keyframes.
//! * A **macro** is one number wired to many parameters at once. Move `energy`
//!   and the bass filter opens, the pad swells, the kick hits harder and the
//!   swirl speeds up, each by its own amount and on its own shape. It is the
//!   conductor's gesture: one hand, many players.
//!
//! The important part is that a macro is the same object whether a curve is
//! driving it or a hand is. Automating the intro and performing it are the same
//! mechanism, which is what makes a captured performance into a score.

use dsp::space::{Position, SPEED_OF_SOUND};

use crate::seq::{Home, Voicing};

/// A value against bars, straight lines between `(bar, value)` keyframes.
/// Before the first and after the last it holds.
#[derive(Clone, Debug)]
pub struct Curve {
    points: Vec<(f32, f32)>,
}

impl Curve {
    /// # Panics
    /// If given no points.
    #[must_use]
    pub fn new(points: Vec<(f32, f32)>) -> Self {
        assert!(!points.is_empty(), "a curve needs at least one point");
        Self { points }
    }

    /// The keyframes, for an interface that wants to draw or edit the curve.
    #[must_use]
    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    #[must_use]
    pub fn at(&self, bar: f64) -> f32 {
        let bar = bar as f32;
        let first = self.points[0];
        if bar <= first.0 {
            return first.1;
        }
        for pair in self.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if bar <= b.0 {
                let span = (b.0 - a.0).max(1e-6);
                return a.1 + (b.1 - a.1) * (bar - a.0) / span;
            }
        }
        self.points[self.points.len() - 1].1
    }
}

/// The helicopter. A lane flies in from far away and lands on its home.
///
/// Entering, it plays the `bars` *before* its span starts, from an aircraft on a
/// descending, tightening spiral — so the beat is audibly approaching for half a
/// minute before it arrives. Which is a trick, not a sound: the ear has no
/// trouble believing a drum machine is a helicopter if it gets closer, louder,
/// brighter and drier the way one would.
#[derive(Clone, Copy, Debug)]
pub struct Flight {
    /// Bars of approach before the landing.
    pub bars: f32,
    /// Metres out, horizontally, at the far end.
    pub distance: f32,
    /// Metres up at the far end.
    pub height: f32,
    /// Degrees clockwise from ahead where it is when far away.
    pub angle: f32,
    /// Laps around the listener on the way in.
    pub circles: f32,
    /// Beats of silence immediately before the landing. Not silence, in fact —
    /// a drop-out, with the glass and the reverb tails ringing through it. The
    /// hole is what makes the landing land.
    pub breath_beats: f32,
    /// Level in the air, relative to the landed lane.
    pub gain: f32,
    /// How much of it goes to the room while flying.
    pub send: f32,
}

impl Default for Flight {
    fn default() -> Self {
        Self {
            bars: 16.0,
            distance: 40.0,
            height: 15.0,
            // Behind and to the left.
            angle: 225.0,
            circles: 1.25,
            breath_beats: 0.0,
            gain: 0.5,
            // Critical distance about 3 m: a wash far off, dry once it is here.
            send: 0.15,
        }
    }
}

impl Flight {
    /// Where the aircraft is relative to the lane's home, at progress `u`:
    /// 0 far away, 1 landed.
    ///
    /// It covers most of the distance early and slows to touch down, circling
    /// faster as it comes in — the squared and 1.5-power curves. A linear
    /// approach reads as a machine on rails.
    #[must_use]
    pub fn offset(self, u: f32) -> Position {
        let u = u.clamp(0.0, 1.0);
        let d = self.distance * (1.0 - u) * (1.0 - u);
        let h = self.height * (1.0 - u).powf(1.5);
        let angle = self.angle.to_radians() + std::f32::consts::TAU * self.circles * u.powf(1.5);
        Position::new(d * angle.sin(), h, -d * angle.cos())
    }

    /// Seconds a sound takes to reach the listener from `position`.
    #[must_use]
    pub fn travel(position: Position) -> f32 {
        position.distance() / SPEED_OF_SOUND
    }

    /// Progress through an approach that lands at `span_start`.
    #[must_use]
    pub fn progress(self, span_start: f32, bar: f64) -> f32 {
        ((bar as f32 - (span_start - self.bars)) / self.bars.max(1e-6)).clamp(0.0, 1.0)
    }
}

/// How a lane comes in or goes out at the edge of a span.
#[derive(Clone, Copy, Debug)]
pub enum Transition {
    /// On the bar.
    Cut,
    /// Over this many bars.
    Fade(f32),
    /// Flown in from far away.
    Fly(Flight),
}

/// When a lane plays, in bars.
#[derive(Clone, Copy, Debug)]
pub struct Play {
    pub start: f32,
    pub end: f32,
    pub enter: Transition,
    pub leave: Transition,
}

impl Transition {
    /// A word for an interface to draw with.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cut => "cut",
            Self::Fade(_) => "fade",
            Self::Fly(_) => "fly",
        }
    }
}

impl Play {
    #[must_use]
    pub fn new(start: f32, end: f32) -> Self {
        Self {
            start,
            end,
            enter: Transition::Cut,
            leave: Transition::Cut,
        }
    }

    #[must_use]
    pub fn fading(start: f32, end: f32, bars: f32) -> Self {
        Self {
            start,
            end,
            enter: Transition::Fade(bars),
            leave: Transition::Fade(bars),
        }
    }

    /// If an arriving flight covers `bar`, the flight and the bar it lands on.
    #[must_use]
    pub fn approach(&self, bar: f64) -> Option<(Flight, f32)> {
        let Transition::Fly(flight) = self.enter else {
            return None;
        };
        if bar >= f64::from(self.start - flight.bars) && bar < f64::from(self.start) {
            return Some((flight, self.start));
        }
        None
    }

    /// Where the approach begins, in bars — before the span itself.
    #[must_use]
    pub fn first_bar(&self) -> f32 {
        match self.enter {
            Transition::Fly(flight) => self.start - flight.bars,
            _ => self.start,
        }
    }

    /// How loud this span is at `bar`, or `None` if it is not running.
    ///
    /// During an approach this is 1.0: the flight carries its own gain, because
    /// a lane in the air is a different thing from the same lane fading in.
    #[must_use]
    pub fn level(&self, bar: f64) -> Option<f32> {
        if self.approach(bar).is_some() {
            return Some(1.0);
        }
        let bar = bar as f32;
        if bar < self.start || bar >= self.end {
            return None;
        }
        let rising = match self.enter {
            Transition::Cut | Transition::Fly(_) => 1.0,
            Transition::Fade(bars) => ((bar - self.start) / bars.max(1e-6)).clamp(0.0, 1.0),
        };
        let falling = match self.leave {
            Transition::Cut | Transition::Fly(_) => 1.0,
            Transition::Fade(bars) => ((self.end - bar) / bars.max(1e-6)).clamp(0.0, 1.0),
        };
        Some(rising.min(falling))
    }
}

/// What a macro can move. Each of these is also a knob the window will show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The lane's fader.
    Level,
    /// How much of it goes to the room.
    Send,
    /// How hard its gate chops.
    GateDepth,
    /// A multiplier on the velocity of notes it starts from now on.
    Velocity,
    /// Bars per lap of its orbit. Smaller is faster, so `from` > `to` speeds up.
    LapRate,
    /// How far out its orbit sits, in metres. This is the one that makes a
    /// `space` curve mean something: a source further away is quieter, duller
    /// and much wetter, because its direct sound falls as 1/r while its send to
    /// the room falls only as 1/sqrt(r). Bringing the scene closer over an
    /// intro is a build all by itself.
    Radius,
    /// FM depth: the brightness of a glass strike.
    GlassIndex,
    /// The pad's filter.
    StringsCutoff,
    /// The bass filter's floor, and how far its envelope opens it.
    BassCutoff,
    BassEnvAmount,
    /// The kick's second pitch drop, and its saturation. These two are the
    /// controls that separate one kick from another, so they are the ones worth
    /// riding across a piece rather than fixing per preset.
    KickPunch,
    KickDrive,
    /// How long a hat rings.
    HatDecay,
}

/// One wire from a macro to a parameter.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    /// Which lane, by name.
    pub lane: &'static str,
    pub target: Target,
    /// Where the parameter sits at macro 0 and at macro 1.
    pub from: f32,
    pub to: f32,
    /// Shape. 1 is linear; above 1 starts slow and ends steep, which is what a
    /// build wants; below 1 the reverse.
    pub curve: f32,
}

impl Mapping {
    #[must_use]
    pub fn new(lane: &'static str, target: Target, from: f32, to: f32) -> Self {
        Self {
            lane,
            target,
            from,
            to,
            curve: 1.0,
        }
    }

    #[must_use]
    pub fn shaped(mut self, curve: f32) -> Self {
        self.curve = curve;
        self
    }

    #[must_use]
    pub fn value(&self, macro_value: f32) -> f32 {
        let shaped = macro_value.clamp(0.0, 1.0).powf(self.curve);
        self.from + (self.to - self.from) * shaped
    }
}

/// One number, many parameters.
#[derive(Clone, Debug)]
pub struct Macro {
    pub name: &'static str,
    pub mappings: Vec<Mapping>,
    /// A curve over bars, when the score is driving. A hand on the fader takes
    /// over: `manual` wins.
    pub automation: Option<Curve>,
    pub manual: Option<f32>,
}

impl Macro {
    #[must_use]
    pub fn new(name: &'static str, mappings: Vec<Mapping>) -> Self {
        Self {
            name,
            mappings,
            automation: None,
            manual: None,
        }
    }

    #[must_use]
    pub fn automated(mut self, curve: Curve) -> Self {
        self.automation = Some(curve);
        self
    }

    /// Where this macro sits at `bar`.
    #[must_use]
    pub fn at(&self, bar: f64) -> f32 {
        self.manual
            .or_else(|| self.automation.as_ref().map(|c| c.at(bar)))
            .unwrap_or(0.0)
    }
}

/// Write one mapped value into whatever it points at.
///
/// Instrument parameters are read when a voice is created, so changing one moves
/// the notes that start from now on and not the ones already ringing — which is
/// how the Python studio works too, where a filter sweep across a piece is a
/// per-note parameter. `Level`, `Send` and `GateDepth` belong to the lane rather
/// than the note, so they move continuously.
pub fn apply(lane: &mut crate::seq::Lane, target: Target, value: f32) {
    match target {
        Target::Level => lane.gain = value,
        Target::Send => lane.send = value.clamp(0.0, 1.0),
        Target::GateDepth => {
            if let Some(gate) = &mut lane.gate {
                gate.depth = value.clamp(0.0, 1.0);
            }
        }
        Target::Velocity => lane.velocity_scale = value.max(0.0),
        Target::LapRate => {
            if let Home::Orbit { bars_per_lap, .. } = &mut lane.home {
                *bars_per_lap = value.max(0.01);
            }
        }
        Target::Radius => {
            if let Home::Orbit { radius, .. } = &mut lane.home {
                *radius = value.max(0.25);
            }
        }
        Target::GlassIndex => {
            if let Voicing::Glass(p) = &mut lane.voicing {
                p.index = value.max(0.0);
            }
        }
        Target::StringsCutoff => {
            if let Voicing::Strings(p) = &mut lane.voicing {
                p.cutoff = value.max(20.0);
            }
        }
        Target::BassCutoff => {
            if let Voicing::Bass(p) = &mut lane.voicing {
                p.cutoff = value.max(20.0);
            }
        }
        Target::BassEnvAmount => {
            if let Voicing::Bass(p) = &mut lane.voicing {
                p.env_amount = value.max(0.0);
            }
        }
        Target::KickPunch => {
            if let Voicing::Kick(p) = &mut lane.voicing {
                p.punch = value.max(0.0);
            }
        }
        Target::KickDrive => {
            if let Voicing::Kick(p) = &mut lane.voicing {
                p.drive = value.max(0.1);
            }
        }
        Target::HatDecay => {
            if let Voicing::Hat(p) = &mut lane.voicing {
                p.decay = value.max(0.001);
            }
        }
    }
}
