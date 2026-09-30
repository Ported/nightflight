//! Pieces as text, so a person can read one.
//!
//! A piece on disk is JSON, which is fine for a program and hopeless for
//! anyone else: `intro.json` is 252 kB and holds eleven lanes, two hundred
//! sounding notes and ninety-seven parameters. Reading it costs about sixty-
//! three thousand tokens to learn things a page of tab would say in eight
//! hundred. That is not an inconvenience, it is the reason nobody edits a
//! piece they already have — it is cheaper to write a script that generates a
//! new one, which is how a tool stops being a tool.
//!
//! So: the same data, as a grid.
//!
//! ```text
//! piece barnluren
//! bpm 125.217
//! bars 6
//! room rt60=2.2 damping=0.65 predelay=0.02 lowcut=130
//!
//! lane kick  clip=drive patch=punch root=Bb1 gain=0.83 len=0.35s
//!   X . . . . . . . X . . . . . . . | X . . . X . . . X . . . X . . .
//!
//! lane bass  clip=drive patch=sub root=G1 gain=0.59 len=0.85 duck
//!   . . . . . . . . . . . . . . . . | G1 . . . . . . . G1 . . . . . . .
//! ```
//!
//! **One line per lane, bars divided by `|`.** Lanes stack, so a column is a
//! moment and you can read down it — which is the whole point of tab and the
//! thing a JSON array of step objects cannot do at any length.
//!
//! **A cell is one token, whitespace-separated.** Percussion writes velocity
//! as a digit — `.` for a rest, `1`-`9` for tenths, `X` for full — so a hat
//! line is a contour you can read: `5.3.4.5.3.4.5.3.`. A pitched lane writes
//! the note itself, `G1`, `Bb2`, because "the seventh semitone above the root"
//! is arithmetic and `D2` is a note.
//!
//! **Velocity is a dial with ten positions, not a float.** That is a deliberate
//! loss: the first draft kept full precision and every hat cell came out
//! `x@0.55`, which is unreadable and was also a confession — those numbers came
//! from a script, not from anyone's judgement. A twentieth of a velocity is
//! below hearing. Rounding to tenths costs nothing and buys a format you can
//! scan.
//!
//! Absolute note names rather than offsets or scale degrees: a lane's root can
//! change, and when it does every offset in the file silently means something
//! else. A note name cannot rot.

use std::fmt::Write as _;

use crate::automation::Play;
use crate::seq::{Home, Lane, Length, Pattern, ReverbSettings, Set, Step, Voicing, STEPS_PER_BAR};

/// Velocity where a bare note name carries none of its own.
const DEFAULT_VELOCITY: f32 = 0.8;

/// A velocity as one character, and back.
fn velocity_char(v: f32) -> char {
    if v >= 0.95 {
        'X'
    } else {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let tenths = (v * 10.0).round().clamp(0.0, 9.0) as u8;
        (b'0' + tenths) as char
    }
}

/// `X` is full, a digit is tenths, and the old words still read: `x` a hit,
/// `o` a ghost.
fn velocity_of(c: char) -> Option<f32> {
    Some(match c {
        'X' => 1.0,
        'x' => 0.7,
        'o' => 0.4,
        '.' => 0.0,
        d if d.is_ascii_digit() => f32::from(d as u8 - b'0') / 10.0,
        _ => return None,
    })
}

/// Flats, not sharps: this is dance music, and `Bb` is what people say.
const NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

/// MIDI number to a name with an octave. 0 is C-1, so 31 is G1.
#[must_use]
pub fn note_name(midi: f32) -> String {
    #[allow(clippy::cast_possible_truncation)]
    let n = midi.round() as i32;
    let octave = n.div_euclid(12) - 1;
    format!("{}{octave}", NAMES[n.rem_euclid(12) as usize])
}

/// A name back to a MIDI number. Accepts sharps as well as flats, because
/// someone will write `F#2` and being right about spelling is not worth an
/// error message.
#[must_use]
pub fn parse_note(text: &str) -> Option<f32> {
    let bytes = text.as_bytes();
    let letter = *bytes.first()?;
    let step = match letter.to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut at = 1;
    let mut accidental = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'b' => accidental -= 1,
            b'#' | b's' => accidental += 1,
            _ => break,
        }
        at += 1;
    }
    let octave: i32 = text[at..].parse().ok()?;
    Some(((octave + 1) * 12 + step + accidental) as f32)
}

// ── Writing ─────────────────────────────────────────────────────────────────

/// One step as a cell.
fn cell(step: Step, pitched: bool, root: f32, default: f32) -> String {
    if step.velocity <= 0.0 {
        return ".".to_string();
    }
    if !pitched {
        return velocity_char(step.velocity).to_string();
    }
    let note = note_name(root + f32::from(step.offset));
    if velocity_char(step.velocity) == velocity_char(default) {
        note
    } else {
        format!("{note}:{}", velocity_char(step.velocity))
    }
}

/// The velocity most of a lane's notes sit at, so the rest can be bare.
fn common_velocity(pattern: &Pattern) -> f32 {
    let mut counts: Vec<(char, usize)> = Vec::new();
    for step in pattern.all().iter().filter(|s| s.velocity > 0.0) {
        let c = velocity_char(step.velocity);
        match counts.iter_mut().find(|(k, _)| *k == c) {
            Some((_, n)) => *n += 1,
            None => counts.push((c, 1)),
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .and_then(|(c, _)| velocity_of(c))
        .unwrap_or(DEFAULT_VELOCITY)
}

fn length(l: Length) -> String {
    match l {
        Length::Seconds(s) => format!("{s}s"),
        Length::Steps(n) => format!("{n}"),
    }
}

fn home(h: Home) -> Option<String> {
    match h {
        Home::Centre => None,
        Home::Orbit {
            radius,
            bars_per_lap,
            phase,
            elevation,
            height,
            height_bars,
        } => Some(format!(
            "orbit={radius}/{bars_per_lap}/{phase}/{elevation}/{height}/{height_bars}"
        )),
    }
}

fn span(p: Play) -> String {
    format!("{}-{}", p.start, p.end)
}

/// A whole piece as tab.
///
/// Lane declarations first and compactly, then the music in **systems** — a
/// few bars at a time, every lane of a clip stacked so a column is a moment.
/// The first draft put a whole lane on one line and the prelude came out
/// fourteen hundred characters wide, which is the same unreadability as the
/// JSON wearing a different hat. Sheet music solved this centuries ago.
#[must_use]
pub fn write(name: &str, set: &Set) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "piece {name}");
    let _ = writeln!(out, "bpm {}", trim(set.bpm));
    let _ = writeln!(out, "bars {}", trim(set.length_bars));
    if let Some(r) = set.reverb {
        let _ = writeln!(
            out,
            "room rt60={} damping={} predelay={} lowcut={}",
            trim(r.rt60), trim(r.damping), trim(r.predelay), trim(r.lowcut)
        );
    }
    let _ = writeln!(out);

    let defaults: Vec<f32> = set.lanes.iter().map(|l| common_velocity(&l.pattern)).collect();

    for (lane, default) in set.lanes.iter().zip(&defaults) {
        let mut attrs = vec![
            format!("clip={}", word(&lane.clip)),
            format!("inst={}", lane.voicing.instrument()),
        ];
        if let Some(p) = &lane.patch {
            attrs.push(format!("patch={}", word(p)));
        }
        attrs.push(format!("root={}", note_name(lane.root)));
        attrs.push(format!("gain={}", trim(lane.gain)));
        attrs.push(format!("len={}", length(lane.length)));
        if lane.send > 0.0 {
            attrs.push(format!("send={}", trim(lane.send)));
        }
        if let Some(h) = home(lane.home) {
            attrs.push(h);
        }
        if lane.ducked {
            attrs.push("duck".to_string());
        }
        if lane.muted {
            attrs.push("mute".to_string());
        }
        if !lane.spans.is_empty() {
            let all: Vec<String> = lane.spans.iter().map(|p| span(*p)).collect();
            attrs.push(format!("plays={}", all.join(",")));
        }
        if lane.voicing.pitched() {
            attrs.push(format!("vel={}", velocity_char(*default)));
        }
        let _ = writeln!(out, "lane {} {}", word(&lane.name), attrs.join(" "));
    }

    // One grid per clip: lanes of a clip are written to be read together, and
    // they can only line up if they agree on how wide a step is.
    let mut clips: Vec<&str> = Vec::new();
    for lane in &set.lanes {
        if !clips.contains(&lane.clip.as_str()) {
            clips.push(&lane.clip);
        }
    }

    for clip in clips {
        let members: Vec<usize> = set
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.clip == clip)
            .map(|(i, _)| i)
            .collect();
        let cells: Vec<Vec<String>> = members
            .iter()
            .map(|&i| {
                let lane = &set.lanes[i];
                let pitched = lane.voicing.pitched();
                lane.pattern
                    .all()
                    .iter()
                    .map(|s| cell(*s, pitched, lane.root, defaults[i]))
                    .collect()
            })
            .collect();
        let width = cells
            .iter()
            .flatten()
            .map(String::len)
            .max()
            .unwrap_or(1);
        let label = members
            .iter()
            .map(|&i| word(&set.lanes[i].name).len())
            .max()
            .unwrap_or(4);
        let bars = cells.iter().map(Vec::len).max().unwrap_or(0).div_ceil(STEPS_PER_BAR);
        // Around a hundred characters of music to a line, whatever the cells
        // cost: four-character notes get a bar to a line, drum hits get four.
        let per_system = (100 / (STEPS_PER_BAR * (width + 1))).max(1);

        for first in (0..bars).step_by(per_system) {
            let last = (first + per_system).min(bars);
            let _ = writeln!(
                out,
                "\ngrid {} bars {}-{}",
                word(clip),
                first + 1,
                last
            );
            for (row, &i) in cells.iter().zip(&members) {
                let mut line = format!("  {:label$} ", word(&set.lanes[i].name));
                for step in first * STEPS_PER_BAR..last * STEPS_PER_BAR {
                    if step > first * STEPS_PER_BAR && step % STEPS_PER_BAR == 0 {
                        line.push_str("| ");
                    }
                    let c = row.get(step).map_or(".", String::as_str);
                    let _ = write!(line, "{c:width$} ");
                }
                let _ = writeln!(out, "{}", line.trim_end());
            }
        }
    }

    // Parameters last: they are the least interesting thing about a piece and
    // the longest, so they do not get to be the first thing you read. Only
    // what differs from the instrument's default, because a patch is a set of
    // departures and listing the rest is noise.
    for lane in &set.lanes {
        let values = lane.voicing.values();
        let pairs: Vec<String> = lane
            .voicing
            .spec()
            .iter()
            .zip(&values)
            .filter(|(s, v)| (**v - s.default).abs() > 1e-6)
            .map(|(s, v)| format!("{}={}", s.name, trim(*v)))
            .collect();
        if !pairs.is_empty() {
            let _ = writeln!(out, "\nparams {} {}", word(&lane.name), pairs.join(" "));
        }
    }
    out
}

/// A name with no spaces, so a line can be split on whitespace. Spaces become
/// underscores and come back as spaces: "open hat" and "open_hat" are the same
/// lane, and nobody has to think about quoting.
fn word(name: &str) -> String {
    name.replace(' ', "_")
}

fn unword(name: &str) -> String {
    name.replace('_', " ")
}

/// Shortest decimal that survives a round trip, so a tab is not full of
/// `0.6200000047683716`.
fn trim(v: f32) -> String {
    for places in 0..6 {
        let s = format!("{v:.places$}");
        if s.parse::<f32>() == Ok(v) {
            return s;
        }
    }
    format!("{v}")
}

// ── Reading ─────────────────────────────────────────────────────────────────

/// Split `key=value` pairs and bare flags off a line's tail.
fn attrs(words: &[&str]) -> Vec<(String, String)> {
    words
        .iter()
        .map(|w| match w.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => ((*w).to_string(), String::new()),
        })
        .collect()
}

fn get<'a>(a: &'a [(String, String)], key: &str) -> Option<&'a str> {
    a.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn flag(a: &[(String, String)], key: &str) -> bool {
    a.iter().any(|(k, _)| k == key)
}

fn number(text: &str, line: usize, what: &str) -> Result<f32, String> {
    text.parse::<f32>()
        .map_err(|_| format!("line {line}: {what} should be a number, not {text:?}"))
}

/// Parse a tab back into a piece.
///
/// # Errors
/// With a line number and what was expected. There is one reader and they are
/// looking at the file, so the message says where rather than what code failed.
pub fn read(text: &str) -> Result<(String, Set), String> {
    let mut name = String::from("untitled");
    let mut bpm = 126.0_f32;
    let mut bars = 0.0_f32;
    let mut reverb = None;
    let mut lanes: Vec<Lane> = Vec::new();
    // Steps land here first, sparsely, because a grid block says which bars it
    // covers and the blocks need not arrive in order.
    let mut steps: Vec<Vec<Step>> = Vec::new();
    let mut velocities: Vec<f32> = Vec::new();
    let mut clip_of_grid = String::new();
    let mut bar_of_grid = 0usize;

    for (n, raw) in text.lines().enumerate() {
        let line = n + 1;
        let body = raw.split('#').next().unwrap_or("").trim_end();
        if body.trim().is_empty() {
            continue;
        }
        let indented = body.starts_with(' ') || body.starts_with('\t');
        let words: Vec<&str> = body.split_whitespace().collect();

        // An indented line inside a grid block is a lane's steps.
        if indented && !clip_of_grid.is_empty() {
            let lane_name = unword(words[0]);
            let Some(index) = lanes.iter().position(|l| l.name == lane_name) else {
                return Err(format!("line {line}: no lane called {lane_name:?}"));
            };
            let pitched = lanes[index].voicing.pitched();
            let root = lanes[index].root;
            let mut at = bar_of_grid * STEPS_PER_BAR;
            for cell in &words[1..] {
                if *cell == "|" {
                    continue;
                }
                let step = read_cell(cell, pitched, root, velocities[index])
                    .ok_or_else(|| format!("line {line}: {cell:?} is not a step"))?;
                if steps[index].len() <= at {
                    steps[index].resize(at + 1, Step::REST);
                }
                steps[index][at] = step;
                at += 1;
            }
            continue;
        }

        match words[0] {
            "piece" => name = unword(words.get(1).copied().unwrap_or("untitled")),
            "bpm" => bpm = number(words.get(1).copied().unwrap_or(""), line, "bpm")?,
            "bars" => bars = number(words.get(1).copied().unwrap_or(""), line, "bars")?,
            "room" => {
                let a = attrs(&words[1..]);
                let pick = |k: &str, fallback: f32| -> Result<f32, String> {
                    get(&a, k).map_or(Ok(fallback), |v| number(v, line, k))
                };
                reverb = Some(ReverbSettings {
                    rt60: pick("rt60", 2.0)?,
                    damping: pick("damping", 0.6)?,
                    predelay: pick("predelay", 0.02)?,
                    lowcut: pick("lowcut", 100.0)?,
                });
            }
            "lane" => {
                let (lane, vel) = read_lane(&words, line)?;
                lanes.push(lane);
                steps.push(Vec::new());
                velocities.push(vel);
            }
            "grid" => {
                clip_of_grid = unword(words.get(1).copied().unwrap_or(""));
                // `bars 5-8` — only the first matters; the rest is for reading.
                bar_of_grid = words
                    .iter()
                    .position(|w| *w == "bars")
                    .and_then(|i| words.get(i + 1))
                    .and_then(|r| r.split('-').next())
                    .and_then(|s| s.parse::<usize>().ok())
                    .map_or(0, |b| b.saturating_sub(1));
            }
            "params" => {
                let lane_name = unword(words.get(1).copied().unwrap_or(""));
                let Some(lane) = lanes.iter_mut().find(|l| l.name == lane_name) else {
                    return Err(format!("line {line}: no lane called {lane_name:?}"));
                };
                for (key, value) in attrs(&words[2..]) {
                    let spec = lane.voicing.spec();
                    let Some(at) = spec.iter().position(|s| s.name == key) else {
                        return Err(format!(
                            "line {line}: {} has no parameter {key:?}; it has {:?}",
                            lane.voicing.instrument(),
                            spec.iter().map(|s| s.name).collect::<Vec<_>>()
                        ));
                    };
                    lane.voicing.set_param(at, number(&value, line, &key)?);
                }
            }
            other => return Err(format!("line {line}: {other:?} is not a thing a tab says")),
        }
    }

    for (lane, mut written) in lanes.iter_mut().zip(steps) {
        // Round up to a whole bar: a pattern that stops mid-bar would loop
        // early and drift against everything else.
        let whole = written.len().div_ceil(STEPS_PER_BAR) * STEPS_PER_BAR;
        written.resize(whole.max(STEPS_PER_BAR), Step::REST);
        lane.pattern = Pattern::steps(written);
    }
    if bars <= 0.0 {
        #[allow(clippy::cast_precision_loss)]
        let longest = lanes
            .iter()
            .map(|l| l.pattern.all().len())
            .max()
            .unwrap_or(STEPS_PER_BAR) as f32;
        bars = longest / STEPS_PER_BAR as f32;
    }

    Ok((
        name,
        Set {
            bpm,
            lanes,
            reverb,
            macros: Vec::new(),
            length_bars: bars,
        },
    ))
}

fn read_cell(cell: &str, pitched: bool, root: f32, default: f32) -> Option<Step> {
    if cell == "." {
        return Some(Step::REST);
    }
    if !pitched {
        let mut chars = cell.chars();
        let v = velocity_of(chars.next()?)?;
        return chars.next().is_none().then(|| Step::new(v, 0));
    }
    let (note, velocity) = match cell.split_once(':') {
        Some((n, v)) => (n, velocity_of(v.chars().next()?)?),
        None => (cell, default),
    };
    let midi = parse_note(note)?;
    #[allow(clippy::cast_possible_truncation)]
    Some(Step::new(velocity, (midi - root).round() as i8))
}

fn read_lane(words: &[&str], line: usize) -> Result<(Lane, f32), String> {
    let name = unword(words.get(1).copied().unwrap_or(""));
    let a = attrs(&words[2..]);
    let instrument = get(&a, "inst").unwrap_or("kick");
    let voicing = Voicing::fresh(instrument)
        .ok_or_else(|| format!("line {line}: there is no {instrument} instrument"))?;

    let root = match get(&a, "root") {
        Some(r) => parse_note(r).ok_or_else(|| format!("line {line}: {r:?} is not a note"))?,
        None => 0.0,
    };
    let length = match get(&a, "len") {
        Some(l) if l.ends_with('s') => Length::Seconds(number(&l[..l.len() - 1], line, "len")?),
        Some(l) => Length::Steps(number(l, line, "len")?),
        None => Length::Steps(1.0),
    };
    let home = match get(&a, "orbit") {
        Some(o) => {
            let n: Vec<f32> = o
                .split('/')
                .map(|p| p.parse().unwrap_or(0.0))
                .collect();
            Home::Orbit {
                radius: n.first().copied().unwrap_or(1.5),
                bars_per_lap: n.get(1).copied().unwrap_or(8.0),
                phase: n.get(2).copied().unwrap_or(0.0),
                elevation: n.get(3).copied().unwrap_or(0.0),
                height: n.get(4).copied().unwrap_or(0.0),
                height_bars: n.get(5).copied().unwrap_or(1.0),
            }
        }
        None => Home::Centre,
    };
    let spans = match get(&a, "plays") {
        Some(list) => list
            .split(',')
            .filter_map(|s| s.split_once('-'))
            .map(|(a, b)| Play::new(a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0)))
            .collect(),
        None => Vec::new(),
    };
    let velocity = get(&a, "vel")
        .and_then(|v| v.chars().next())
        .and_then(velocity_of)
        .unwrap_or(DEFAULT_VELOCITY);

    Ok((
        Lane {
            name,
            clip: unword(get(&a, "clip").unwrap_or("clip")),
            voicing,
            patch: get(&a, "patch").map(unword),
            pattern: Pattern::steps(Vec::new()),
            gain: get(&a, "gain").map_or(Ok(1.0), |g| number(g, line, "gain"))?,
            length,
            root,
            home,
            send: get(&a, "send").map_or(Ok(0.0), |s| number(s, line, "send"))?,
            gate: None,
            spans,
            velocity_scale: 1.0,
            ducked: flag(&a, "duck"),
            muted: flag(&a, "mute"),
        },
        velocity,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_survive_a_round_trip() {
        for midi in 0_i16..128 {
            let name = note_name(f32::from(midi));
            assert_eq!(
                parse_note(&name),
                Some(f32::from(midi)),
                "{midi} wrote as {name}"
            );
        }
        // Sharps are accepted even though flats are written: someone will type
        // F#2 and being right about spelling is not worth an error.
        assert_eq!(parse_note("F#2"), parse_note("Gb2"));
        assert_eq!(parse_note("G1"), Some(31.0));
        assert_eq!(note_name(31.0), "G1");
    }

    /// The real test: a piece written as tab and read back is the same piece.
    ///
    /// Velocity is the one thing allowed to move, and only to the nearest
    /// tenth — that loss is the format's whole argument, so it is asserted
    /// rather than tolerated.
    #[test]
    fn every_piece_survives_a_round_trip() {
        for name in crate::sets::NAMES {
            let before = crate::sets::by_name(name).expect("a built-in set");
            let text = write(name, &before);
            let (read_name, after) = read(&text)
                .unwrap_or_else(|why| panic!("{name} did not read back: {why}"));

            assert_eq!(read_name, *name);
            assert!((after.bpm - before.bpm).abs() < 1e-3, "{name}: bpm");
            assert_eq!(after.lanes.len(), before.lanes.len(), "{name}: lane count");

            for (a, b) in after.lanes.iter().zip(&before.lanes) {
                assert_eq!(a.name, b.name);
                assert_eq!(a.clip, b.clip);
                assert_eq!(a.voicing.instrument(), b.voicing.instrument());
                assert!((a.root - b.root).abs() < 1e-3, "{}: root", a.name);
                assert!((a.gain - b.gain).abs() < 1e-4, "{}: gain", a.name);
                assert_eq!(a.ducked, b.ducked, "{}: duck", a.name);
                assert_eq!(a.spans.len(), b.spans.len(), "{}: spans", a.name);
                assert_eq!(
                    format!("{:?}", a.voicing),
                    format!("{:?}", b.voicing),
                    "{}: parameters",
                    a.name
                );
                assert_eq!(
                    a.pattern.all().len(),
                    b.pattern.all().len(),
                    "{}: pattern length",
                    a.name
                );
                for (i, (x, y)) in a.pattern.all().iter().zip(b.pattern.all()).enumerate() {
                    assert_eq!(x.offset, y.offset, "{}: step {i} pitch", a.name);
                    // Half a position on a ten-position dial, plus float slop:
                    // 0.55 rounds to 0.6 and the difference is 0.0500000119.
                    assert!(
                        (x.velocity - y.velocity).abs() <= 0.0501,
                        "{}: step {i} velocity moved {} -> {}",
                        a.name,
                        y.velocity,
                        x.velocity
                    );
                    assert_eq!(
                        x.velocity > 0.0,
                        y.velocity > 0.0,
                        "{}: step {i} appeared or vanished",
                        a.name
                    );
                }
            }
        }
    }

    /// Written twice, it says the same thing: the rounding happens once, not
    /// a little more on every pass.
    #[test]
    fn writing_is_idempotent() {
        for name in crate::sets::NAMES {
            let set = crate::sets::by_name(name).expect("a built-in set");
            let once = write(name, &set);
            let (_, back) = read(&once).expect("reads");
            assert_eq!(once, write(name, &back), "{name} drifted on a second pass");
        }
    }

    #[test]
    fn a_tab_written_by_hand_plays() {
        let (name, set) = read(
            "
piece test
bpm 120
bars 2

lane kick  clip=beat inst=kick root=G1 gain=0.9 len=0.4s
lane bass  clip=beat inst=bass root=G1 len=0.8 duck vel=8

grid beat bars 1-2
  kick  X . . . 6 . . . X . . . . . . . | X . . . X . . . X . . . X . . .
  bass  G1 . . . .  . . . G2 . . . . . . . | .  . . . Bb1 . . . . . . . . . . .
",
        )
        .expect("a hand-written tab reads");
        assert_eq!(name, "test");
        assert_eq!(set.lanes.len(), 2);
        assert_eq!(set.lanes[0].pattern.all().len(), 32);
        assert!((set.lanes[0].pattern.at(4).velocity - 0.6).abs() < 1e-6);
        assert_eq!(set.lanes[1].pattern.at(8).offset, 12, "G2 above a G1 root");
        assert!(set.lanes[1].ducked);
    }

    #[test]
    fn a_bad_line_says_where() {
        let why = read("piece x\nlane k clip=c inst=zither\n").unwrap_err();
        assert!(why.contains("line 2"), "{why}");
        assert!(why.contains("zither"), "{why}");
    }
}
