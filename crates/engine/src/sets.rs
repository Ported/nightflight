//! The music, as code.
//!
//! Rolling: four on the
//! floor, hats chasing each other on the sixteenths, and a bass rolling on the
//! three off-beats between kicks. Wata Igarashi's shape — the kick owns the
//! beat, the bass fills the gaps, and almost nothing is a melody.
//!
//! The room: a 3.5 s tail, damped so it darkens, with the hats sending into it
//! and the kick and bass staying dry. Reverberant sub-bass is mud, and the
//! Python's Rolling has no reverb at all — this is the one deliberate departure,
//! because a hypnotic techno hat wants a little air around it, and because a
//! reverb nothing feeds is a reverb nobody can hear.
//!
//! Sends are small for a reason. Tails overlap: a hat on every sixteenth into a
//! 3.5 s tail piles about twenty of them on top of each other, so a send that
//! looks modest arrives much louder than it reads. The Python learned this the
//! hard way — a send of 0.6 made a landing helicopter 15 dB wetter than dry.
//!
//! Space: kick and bass stay in the centre, unprocessed. The two hats orbit
//! the head 1.5 m out, one lap every four bars, half a lap apart so they chase
//! each other, rising and dipping on a six-bar cycle that drifts against the
//! four-bar laps.
//!
//! It is Rust rather than a data file on purpose, for now. A file would need a
//! format, a writer, a schema and a drift test before a single note played; a
//! `Vec<Lane>` needs none of that, and the structs are already `serde`-shaped
//! for the day loading one is worth it.

use dsp::inst::{bass, glass, hat, kick, strings};

use crate::automation::{Curve, Flight, Macro, Mapping, Play, Target, Transition};
use crate::mix::Gate;
use crate::seq::{Home, Lane, Length, Pattern, ReverbSettings, Set, Step, Voicing};

/// Everything here is written at this tempo.
pub const BPM: f32 = 126.0;

/// G1, about 49 Hz: Rolling's root, and the kick is tuned to it.
pub const ROOT: f32 = 31.0;

/// Accents down the sixteenths: strong on the beat, weakest on the second
/// sixteenth, medium on the off-beat. Without this a hat line is a machine gun.
const HAT_ACCENTS: [f32; 4] = [0.55, 0.35, 0.5, 0.45];

/// The hats' circle: 1.5 m out, a lap every four bars.
fn orbit(phase: f32) -> Home {
    Home::Orbit {
        radius: 1.5,
        bars_per_lap: 4.0,
        phase,
        elevation: 0.0,
        height: 0.4,
        height_bars: 6.0,
    }
}

/// Semitones for the rolling bass, over a two-bar phrase. Mostly the root: the
/// pull comes from the one octave jump and the one minor third.
fn rolling_bass() -> Pattern {
    let mut steps = vec![Step::REST; 32];
    for (i, step) in steps.iter_mut().enumerate() {
        if i % 4 == 0 {
            continue; // the kick's own sixteenth stays empty
        }
        let offset = match i {
            22 => 12, // the octave, once every two bars
            30 => 3,  // the minor third, the only hint of a key
            _ => 0,
        };
        // The off-beat sixteenth is the accented one: the kick just ducked the
        // one before it, so this is where the bass is heard.
        *step = Step::new(if i % 4 == 2 { 1.0 } else { 0.8 }, offset);
    }
    Pattern::steps(steps)
}

/// Closed hats on every sixteenth except the off-beat, where the open hat goes.
fn closed_hats() -> Pattern {
    let steps = (0..16)
        .map(|i| {
            if i % 4 == 2 {
                Step::REST
            } else {
                Step::new(HAT_ACCENTS[i % 4], 0)
            }
        })
        .collect();
    Pattern::steps(steps)
}

#[must_use]
/// The four lanes of the groove. Both `rolling` and the arrival use these; the
/// score is the only difference between a loop to jam on and a piece.
fn groove_lanes() -> Vec<Lane> {
    vec![
        Lane {
            name: "kick".into(),
            clip: "beat".into(),
            voicing: Voicing::Kick(kick::Params::default()),
            patch: None,
            pattern: Pattern::grid("X...X...X...X..."),
            gain: 1.0,
            // Seconds, not steps: a drum's ring is a physical fact.
            length: Length::Seconds(0.4),
            root: ROOT,
            home: Home::Centre,
            send: 0.0,
            gate: None,
            spans: Vec::new(),
            velocity_scale: 1.0,
            ducked: false,
            muted: false,
        },
        Lane {
            name: "closed hat".into(),
            clip: "beat".into(),
            voicing: Voicing::Hat(hat::Params::default()),
            patch: None,
            pattern: closed_hats(),
            gain: 0.6,
            length: Length::Seconds(0.06),
            root: 0.0,
            home: orbit(0.0),
            send: 0.15,
            gate: None,
            spans: Vec::new(),
            velocity_scale: 1.0,
            ducked: false,
            muted: false,
        },
        Lane {
            name: "open hat".into(),
            clip: "beat".into(),
            voicing: Voicing::Hat(hat::Params {
                decay: 0.3,
                ..hat::Params::default()
            }),
            // The disco "tss" between the kicks.
            patch: None,
            pattern: Pattern::grid("..x...x...x...x."),
            gain: 0.6,
            // One step: the next closed hat cuts it off, which is exactly what
            // a foot closing the pedal does.
            length: Length::Steps(1.0),
            root: 0.0,
            // Half a lap behind the closed hat: they pass each other twice a lap.
            home: orbit(0.5),
            send: 0.2,
            gate: None,
            spans: Vec::new(),
            velocity_scale: 1.0,
            ducked: false,
            muted: false,
        },
        Lane {
            name: "bass".into(),
            clip: "bass".into(),
            voicing: Voicing::Bass(bass::Params::default()),
            patch: None,
            pattern: rolling_bass(),
            gain: 0.8,
            length: Length::Steps(0.8),
            root: ROOT,
            home: Home::Centre,
            send: 0.0,
            gate: None,
            spans: Vec::new(),
            velocity_scale: 1.0,
            ducked: true,
            muted: false,
        },
    ]
}

#[must_use]
pub fn rolling() -> Set {
    Set {
        bpm: BPM,
        lanes: groove_lanes(),
        macros: Vec::new(),
        // Rolling's own length in the Python: forty bars, four-bar blocks.
        length_bars: 40.0,
        reverb: Some(ReverbSettings {
            rt60: 3.5,
            damping: 0.6,
            predelay: 0.03,
            lowcut: 100.0,
        }),
    }
}

// ── Prelude ─────────────────────────────────────────────────────────────────

/// Bach's Prelude in C major (BWV 846, 1722), transposed to G and played on
/// glass, as the hour's intro.
///
/// Bach built every bar from one five-note chord played through the same figure
/// — notes 1 2 3 4 5 3 4 5, twice a bar, with the lowest two held. He wrote an
/// arpeggiator two hundred and fifty years early, which is why the chords here
/// are data and the figure is code.
///
/// What changes from Bach: FM glass instead of a harpsichord, left to ring for
/// seconds like a sustain pedal held down; a seven-second reverb that every
/// voice sends most of itself into; his sixteenths become eighths at 126 BPM,
/// so one of his bars lasts two of ours; down a fourth into G, the key of
/// Rolling; and each chord voice is its own source on a slowly turning ring,
/// lower voices lower, so the figure spirals around the listener.
///
/// Only the lowest three voices play. The upper two carry the melody and come
/// in later in the piece.
///
/// The chords are from memory and worth checking against a score — IMSLP has it.
/// One Bach bar per row, lowest note first, as MIDI numbers already transposed.
const CHORDS: [[i8; 5]; 11] = [
    [55, 59, 62, 67, 71], // G  B  D  G  B — the tonic
    [55, 57, 64, 69, 72],
    [54, 57, 62, 69, 72],
    [55, 59, 62, 67, 71],
    [55, 59, 64, 71, 76],
    [55, 57, 61, 64, 69],
    [54, 57, 62, 69, 74],
    [54, 55, 59, 62, 67],
    [52, 55, 59, 62, 67],
    [45, 52, 57, 61, 67],
    [50, 54, 57, 62, 66], // ends on the dominant: it pulls home and never gets there
];

/// Which chord note plays on each eighth, twice per Bach bar.
const FIGURE: [usize; 8] = [0, 1, 2, 3, 4, 2, 3, 4];

/// The three voices' root; offsets in the pattern are relative to it.
const PRELUDE_ROOT: f32 = 55.0;
const PRELUDE_RADIUS: f32 = 2.0;
/// One lap every eight bars. The Python accelerates this to one lap a bar
/// across the build, like a rotor spinning up; that needs macro curves, which
/// are the next thing to build.
const PRELUDE_LAP_BARS: f32 = 8.0;

/// Bach's figure for one voice, as a pattern over all eleven bars.
///
/// One Bach bar is sixteen eighths — thirty-two of our sixteenth-note steps —
/// and the figure runs through it twice.
fn bach_voice(voice: usize, pedal_bars: usize) -> Pattern {
    // The pedal holds the last chord: tension with no release. Bach's eleven
    // bars end on the dominant and these keep sitting on it.
    let bars: Vec<[i8; 5]> = CHORDS
        .iter()
        .copied()
        .chain(std::iter::repeat_n(CHORDS[CHORDS.len() - 1], pedal_bars))
        .collect();
    let mut steps = vec![Step::REST; bars.len() * 32];
    for (bar, chord) in bars.iter().enumerate() {
        for half in 0..2 {
            for (k, &plays) in FIGURE.iter().enumerate() {
                if plays != voice {
                    continue;
                }
                let step = bar * 32 + half * 16 + k * 2;
                // The first note of each figure leans slightly harder.
                let velocity = if k == 0 { 0.77 } else { 0.7 };
                steps[step] = Step::new(velocity, chord[voice] - PRELUDE_ROOT as i8);
            }
        }
    }
    Pattern::steps(steps)
}

/// Half asleep: a softer strike that melts more slowly, notes ringing for
/// seconds so they overlap, and the whole thing drifting in pitch like tape.
fn dreaming() -> glass::Params {
    glass::Params {
        index: 1.3,
        index_decay: 0.25,
        decay: 4.0,
        release: 3.0,
        strike: 0.07,
        wow_depth: 8.0,
        wow_rate: 0.2,
        ..glass::Params::default()
    }
}

#[must_use]
pub fn prelude() -> Set {
    // Bach holds the bass through the half bar and the second voice nearly as
    // long; everything above is a single eighth.
    let lengths = [Length::Steps(16.0), Length::Steps(14.0), Length::Steps(2.0)];
    let names = ["voice 1", "voice 2", "voice 3"];
    let lanes = (0..3)
        .map(|voice| Lane {
            name: names[voice].into(),
            clip: "prelude".into(),
            voicing: Voicing::Glass(dreaming()),
            patch: None,
            pattern: bach_voice(voice, 0),
            // Measured, not chosen: at 1.2/1.0 the mix peaked at +3.5 dBFS.
            // Three voices ringing for seconds each, all sending most of
            // themselves into a seven-second room, add up to far more than the
            // sum of their notes. The Python gets away with those numbers
            // because it peak-normalises the finished track; there is no master
            // stage here yet, so the set carries its own balance.
            gain: if voice == 0 { 0.55 } else { 0.46 },
            length: lengths[voice],
            root: PRELUDE_ROOT,
            home: Home::Orbit {
                radius: PRELUDE_RADIUS,
                bars_per_lap: PRELUDE_LAP_BARS,
                // Evenly spaced, the lowest voice starting behind the listener.
                phase: 0.5 + voice as f32 / 3.0,
                elevation: -0.3 + 0.3 * voice as f32,
                height: 0.0,
                height_bars: 1.0,
            },
            // Nearly all of it goes to the room. At 2 m the send falls to
            // 1/sqrt(2) = 0.71 on its own, so this is as wet as it looks.
            send: 1.0,
            gate: None,
            spans: Vec::new(),
            velocity_scale: 1.0,
            ducked: false,
            muted: false,
        })
        .collect();

    Set {
        bpm: BPM,
        lanes,
        macros: Vec::new(),
        // Bach's eleven bars, two of ours each.
        length_bars: (CHORDS.len() * 2) as f32,
        reverb: Some(ReverbSettings {
            rt60: 7.0,
            // The Python asks 0.6 here; our shelves tilt more gently, so a
            // higher number gives the same darkening.
            damping: 1.0,
            predelay: 0.04,
            lowcut: 100.0,
        }),
    }
}

// ── Pad ─────────────────────────────────────────────────────────────────────

/// Four chords in G minor, two bars each, with open unresolved colours — ninths
/// and a suspension — so nothing ever quite lands:
///
/// ```text
/// Gm9     G3 Bb3 D4 F4 A4    home, soft-edged: the 9th (A) floats above
/// Ebmaj9  Eb3 G3 Bb3 D4 F4   the warm step away, down a third
/// Cm9     C3 Eb3 G3 Bb3 D4   darker, deeper
/// D7sus4  D3 G3 A3 C4 F4     the pull back home, held open (G instead of F#)
/// ```
///
/// The patch chosen by ear from four candidates: the dark dream, with
/// each of the five chord voices its own source turning slowly around the head —
/// the envelopment stereo cannot give.
const PAD_CHORDS: [[i8; 5]; 4] = [
    [55, 58, 62, 65, 69],
    [51, 55, 58, 62, 65],
    [48, 51, 55, 58, 62],
    [50, 55, 57, 60, 65],
];

const PAD_ROOT: f32 = 48.0;
const PAD_VOICES: usize = 5;
/// Two bars a chord, so a voice's note is 32 steps long.
const PAD_STEPS_PER_CHORD: usize = 32;
/// The chop: syncopated sixteenths, 55% of each step left open.
const PAD_GATE: &str = "x.xx.xx.x.xx.x.x";

fn pad_voice(voice: usize) -> Pattern {
    let mut steps = vec![Step::REST; PAD_CHORDS.len() * PAD_STEPS_PER_CHORD];
    for (chord, notes) in PAD_CHORDS.iter().enumerate() {
        steps[chord * PAD_STEPS_PER_CHORD] = Step::new(0.8, notes[voice] - PAD_ROOT as i8);
    }
    Pattern::steps(steps)
}

#[must_use]
pub fn pad() -> Set {
    const NAMES: [&str; PAD_VOICES] = [
        "pad voice 1",
        "pad voice 2",
        "pad voice 3",
        "pad voice 4",
        "pad voice 5",
    ];
    let lanes = (0..PAD_VOICES)
        .map(|voice| Lane {
            name: NAMES[voice].into(),
            clip: "pad".into(),
            voicing: Voicing::Strings(strings::dark()),
            patch: None,
            pattern: pad_voice(voice),
            // Measured for a standalone listen: at 0.3 the five voices peaked
            // at -19.7 dBFS, far too quiet to judge. This puts the peak near
            // -4. It will want rebalancing again when the pad sits under the
            // glass rather than alone.
            gain: 1.8,
            length: Length::Steps(PAD_STEPS_PER_CHORD as f32),
            root: PAD_ROOT,
            home: Home::Orbit {
                radius: 2.5,
                bars_per_lap: 16.0,
                phase: voice as f32 / PAD_VOICES as f32,
                // Low notes low, high notes high.
                elevation: -0.3 + 0.2 * voice as f32,
                height: 0.0,
                height_bars: 1.0,
            },
            send: 0.5,
            // Off by default: the depth is what a build rides on, so it starts
            // at nothing and is turned up.
            gate: Some(Gate::new(PAD_GATE, 0.55, 0.0)),
            spans: Vec::new(),
            velocity_scale: 1.0,
            ducked: false,
            muted: false,
        })
        .collect();

    Set {
        bpm: BPM,
        lanes,
        macros: Vec::new(),
        // Four chords, two bars each.
        length_bars: (PAD_CHORDS.len() * 2) as f32,
        reverb: Some(ReverbSettings {
            rt60: 7.0,
            damping: 1.0,
            predelay: 0.04,
            lowcut: 100.0,
        }),
    }
}

// ── Intro ───────────────────────────────────────────────────────────────────

/// The first twenty-six bars of Night Flight: the Prelude on glass with Bach's
/// chords held underneath on the pad, everything winding up towards a landing
/// that has not been built yet.
///
/// This is where curves and spans earn their place. Three macros run the whole
/// thing:
///
/// * **energy** — 0.10 at the start, 0.50 by the landing. It swells the pad from
///   far under the glass to meeting it, on a squared curve so most of the growth
///   happens late.
/// * **build** — 0 to 1 over the same bars. It leans on the glass: louder, and
///   brighter (more FM), and it spins the ring the voices sit on from one lap
///   every eight bars to one lap a bar, like a rotor starting.
/// * **space** — 0.80 vast down to 0.20 intimate. It walks the whole scene in:
///   the glass ring from 4 m to 2 m and the pad's from 5 m to 2.5 m. Distance
///   is not just volume — a far source is duller and much wetter, because its
///   direct sound falls as 1/r while its feed to the room falls only as
///   1/sqrt(r). So the piece starts as a wash a long way off and arrives.
/// * **chop** — nothing until bar 10, then the pad's gate creeping in until it
///   is chopping fully by bar 22, where the dominant pedal begins. The pad
///   drives the build before any drum does.
///
/// And then the arrival. The kick and both hats fly in from 40 m up and behind
/// over bars 10 to 26, on a descending spiral that circles the listener once and
/// a quarter, getting closer, louder and drier the whole way. One beat before
/// the landing everything drops out — the breath — and the glass and the reverb
/// ring through the hole. Then the groove is simply *there*, on the ground, and
/// the bass rolls in with it.
///
/// The beat's notes are emitted early by their own travel time so they *land* on
/// the beat rather than 116 ms after it, which is what 40 m costs. The flight is
/// also at 0.85 of the landed level, so the landing is a step up of a few dB
/// rather than a jolt.
const LANDING: f32 = 26.0;
/// Where the chop starts creeping in — in the hour, where the helicopter sets off.
const TAKEOFF: f32 = 10.0;
/// Where it is chopping fully: the dominant pedal.
const GATE_FULL: f32 = 22.0;
/// Bach's eleven bars plus two holding the dominant.
const PEDAL_BARS: usize = 2;
/// Bars of groove after the landing.
const AFTER: f32 = 8.0;
/// The pad takes the upper four notes of each chord; the glass has the lower three.
const INTRO_PAD_VOICES: usize = 4;

#[must_use]
pub fn intro() -> Set {
    let glass_names = ["voice 1", "voice 2", "voice 3"];
    let lengths = [Length::Steps(16.0), Length::Steps(14.0), Length::Steps(2.0)];
    let mut lanes: Vec<Lane> = (0..3)
        .map(|voice| Lane {
            name: glass_names[voice].into(),
            clip: "prelude".into(),
            voicing: Voicing::Glass(dreaming()),
            patch: None,
            pattern: bach_voice(voice, PEDAL_BARS),
            gain: if voice == 0 { 0.3 } else { 0.25 },
            length: lengths[voice],
            root: PRELUDE_ROOT,
            home: Home::Orbit {
                radius: PRELUDE_RADIUS,
                bars_per_lap: PRELUDE_LAP_BARS,
                phase: 0.5 + voice as f32 / 3.0,
                elevation: -0.3 + 0.3 * voice as f32,
                height: 0.0,
                height_bars: 1.0,
            },
            send: 1.0,
            gate: None,
            spans: vec![Play::new(0.0, LANDING)],
            velocity_scale: 1.0,
            ducked: false,
            muted: false,
        })
        .collect();

    const PAD_NAMES: [&str; INTRO_PAD_VOICES] = ["pad 1", "pad 2", "pad 3", "pad 4"];
    lanes.extend((0..INTRO_PAD_VOICES).map(|voice| Lane {
        name: PAD_NAMES[voice].into(),
        clip: "pad".into(),
        voicing: Voicing::Strings(strings::dark()),
        // Bach's chords held, one per Bach bar: the harmony the glass
        // arpeggiates, so the pad is never a second idea.
        patch: None,
        pattern: held_chord(voice + 1, PEDAL_BARS),
        gain: 0.32,
        length: Length::Steps(32.0),
        root: PRELUDE_ROOT,
        home: Home::Orbit {
            radius: 2.5,
            bars_per_lap: 16.0,
            phase: voice as f32 / INTRO_PAD_VOICES as f32,
            elevation: -0.3 + 0.2 * voice as f32,
            height: 0.0,
            height_bars: 1.0,
        },
        send: 0.5,
        gate: Some(Gate::new(PAD_GATE, 0.55, 0.0)),
        spans: vec![Play {
            start: 0.0,
            end: Some(LANDING),
            enter: Transition::Fade { bars: 4.0 },
            leave: Transition::Cut,
        }],
        velocity_scale: 1.0,
        ducked: false,
        muted: false,
    }));

    // Every wire from the three faders. The pad's velocity goes to 20 because
    // its pattern is written at 0.8, and 0.8 x 20 x energy squared reproduces
    // the Python's 16 x energy squared.
    let mut energy = Vec::new();
    let mut build = Vec::new();
    let mut chop = Vec::new();
    let mut space = Vec::new();
    for name in PAD_NAMES {
        energy.push(Mapping::new(name, Target::Velocity, 0.0, 20.0).shaped(2.0));
        chop.push(Mapping::new(name, Target::GateDepth, 0.0, 1.0));
        // At space 0.2 this is 2.5 m, at 0.8 it is 5 m.
        space.push(Mapping::new(name, Target::Radius, 1.67, 5.83));
    }
    for name in glass_names {
        // 0.55 to 0.95 of absolute velocity, over a pattern written at 0.7.
        build.push(Mapping::new(name, Target::Velocity, 0.786, 1.357).shaped(1.5));
        build.push(Mapping::new(name, Target::GlassIndex, 0.9, 2.4).shaped(2.0));
        build.push(Mapping::new(name, Target::LapRate, PRELUDE_LAP_BARS, 1.0));
        // 2 m at space 0.2, 4 m at 0.8.
        space.push(Mapping::new(name, Target::Radius, 1.33, 4.67));
    }

    // The beat, flown in. The bass does not fly: it simply starts, one
    // sixteenth after the downbeat, the way Rolling has it.
    let arrive = Transition::Fly(Flight {
        bars: LANDING - TAKEOFF,
        breath_beats: 1.0,
        gain: 0.85,
        ..Flight::default()
    });
    for mut lane in groove_lanes() {
        lane.spans = vec![Play {
            start: LANDING,
            end: Some(LANDING + AFTER),
            enter: if lane.name == "bass" {
                Transition::Cut
            } else {
                arrive
            },
            leave: Transition::Cut,
        }];
        lanes.push(lane);
    }

    Set {
        bpm: BPM,
        lanes,
        macros: vec![
            Macro::new("energy", energy).automated(Curve::new(vec![(0.0, 0.10), (LANDING, 0.50)])),
            Macro::new("build", build).automated(Curve::new(vec![(0.0, 0.0), (LANDING, 1.0)])),
            Macro::new("space", space).automated(Curve::new(vec![(0.0, 0.80), (LANDING, 0.20)])),
            Macro::new("chop", chop).automated(Curve::new(vec![
                (0.0, 0.0),
                (TAKEOFF, 0.0),
                (GATE_FULL, 1.0),
            ])),
        ],
        // The arrival: the Prelude, the flight, the landing, eight bars of groove.
        length_bars: LANDING + AFTER,
        reverb: Some(ReverbSettings {
            rt60: 7.0,
            damping: 1.0,
            predelay: 0.04,
            lowcut: 100.0,
        }),
    }
}

/// One of Bach's chord notes, held for a whole Bach bar.
fn held_chord(voice: usize, pedal_bars: usize) -> Pattern {
    let bars: Vec<[i8; 5]> = CHORDS
        .iter()
        .copied()
        .chain(std::iter::repeat_n(CHORDS[CHORDS.len() - 1], pedal_bars))
        .collect();
    let mut steps = vec![Step::REST; bars.len() * 32];
    for (bar, chord) in bars.iter().enumerate() {
        steps[bar * 32] = Step::new(0.8, chord[voice] - PRELUDE_ROOT as i8);
    }
    Pattern::steps(steps)
}

/// Every set by name, for the command line and later the window.
#[must_use]
pub fn by_name(name: &str) -> Option<Set> {
    match name {
        "rolling" => Some(rolling()),
        "prelude" => Some(prelude()),
        "pad" => Some(pad()),
        "intro" => Some(intro()),
        _ => None,
    }
}

/// The names `by_name` accepts.
pub const NAMES: [&str; 4] = ["rolling", "prelude", "pad", "intro"];
