//! Everything that touches the format or makes a sound.
//!
//! The half of `nf` that has to be Rust, because the tab parser lives here and
//! is round-trip tested, and a second parser in another language would drift
//! from it. The Python side never reads a tab file; it asks this.
//!
//! Output is plain on stdout and diagnostics on stderr, or JSON with `--json`
//! where a caller wants structure. Exit non-zero on anything that went wrong.
//!
//! ```text
//! nf-engine describe <piece>              the piece as tab
//! nf-engine render   <piece> out.wav      render it
//! nf-engine check    <file.tab>           read it and say what is wrong
//! nf-engine convert  <in> <out>           tab <-> json, by extension
//! nf-engine index                         what the library holds
//! ```
//!
//! A `<piece>` is a name in `pieces/`, a path to a `.json`, or a path to a
//! `.tab`. All three work everywhere one is asked for, because the thing you
//! are iterating on is a file and the thing you saved is a name.

use std::path::Path;
use std::process::ExitCode;

use engine::seq::Set;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(verb) = args.first().map(String::as_str) else {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    };
    let rest: Vec<&str> = args[1..].iter().map(String::as_str).collect();

    let result = match verb {
        "describe" => describe(&rest),
        "render" => render(&rest),
        "check" => check(&rest),
        "convert" => convert(&rest),
        "index" => index(),
        "lanes" => lanes(&rest),
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("{other:?} is not a verb.\n\n{}", usage())),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("{why}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> String {
    "nf-engine describe <piece>\n\
     nf-engine render   <piece> <out.wav> [--seconds N] [--bpm N] [--solo lane]\n\
     nf-engine check    <file.tab>\n\
     nf-engine convert  <in> <out>\n\
     nf-engine lanes    <piece>\n\
     nf-engine index"
        .to_string()
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| *a == name)
        .and_then(|i| args.get(i + 1))
        .copied()
}

fn positional<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a.starts_with("--") {
            skip = true;
        } else {
            out.push(*a);
        }
    }
    out
}

/// A name in `pieces/`, a `.json`, or a `.tab`. Patches named by a lane are
/// resolved from the library either way, so what renders is what plays.
fn load(what: &str) -> Result<(String, Set), String> {
    let (name, mut set) = engine::document::find(what).map_err(|err| err.to_string())?;
    for missing in engine::library::resolve(&mut set) {
        eprintln!("patch not found: {missing}");
    }
    Ok((name, set))
}

fn describe(args: &[&str]) -> Result<(), String> {
    let what = positional(args)
        .first()
        .copied()
        .ok_or("describe needs a piece")?;
    let (name, set) = load(what)?;
    print!("{}", engine::tab::write(&name, &set));
    Ok(())
}

fn check(args: &[&str]) -> Result<(), String> {
    let what = positional(args)
        .first()
        .copied()
        .ok_or("check needs a file")?;
    let (name, set) = load(what)?;
    // Reading it is most of the check. The rest is what a reader cannot know:
    // whether the patches a lane names are actually there.
    let mut trouble = Vec::new();
    for lane in &set.lanes {
        if let Some(patch) = &lane.patch
            && engine::library::load_patch(patch).is_err()
        {
            trouble.push(format!("{}: no patch called {patch:?}", lane.name));
        }
        if lane.pattern.all().iter().all(|s| s.velocity <= 0.0) {
            trouble.push(format!("{}: every step is a rest", lane.name));
        }
    }
    if trouble.is_empty() {
        println!(
            "{name}: {} lanes, {} bars at {} BPM — reads clean",
            set.lanes.len(),
            set.length_bars,
            set.bpm
        );
        Ok(())
    } else {
        Err(trouble.join("\n"))
    }
}

fn convert(args: &[&str]) -> Result<(), String> {
    let p = positional(args);
    let (from, to) = match (p.first(), p.get(1)) {
        (Some(a), Some(b)) => (*a, *b),
        _ => return Err("convert needs an input and an output".into()),
    };
    let (name, set) = load(from)?;
    let out = Path::new(to);
    if out.extension().is_some_and(|e| e == "tab") {
        std::fs::write(out, engine::tab::write(&name, &set))
            .map_err(|err| format!("could not write {to}: {err}"))?;
    } else {
        engine::document::save(out, &name, &set)
            .map_err(|err| format!("could not write {to}: {err}"))?;
    }
    println!("{to}");
    Ok(())
}

/// What a piece is made of, so a caller can solo each one in turn.
///
/// Here rather than in `nf` because finding this out means reading the piece,
/// and reading a piece is this side's job — the Python never parses a tab,
/// not even the easy lines at the top of one.
fn lanes(args: &[&str]) -> Result<(), String> {
    let what = positional(args)
        .first()
        .copied()
        .ok_or("lanes needs a piece")?;
    let (_, set) = load(what)?;
    let seconds_per_step = 60.0 / set.bpm / 4.0;

    let out: Vec<serde_json::Value> = set
        .lanes
        .iter()
        .map(|lane| {
            // How many of a lane's own notes sound at once where they are
            // closest together. Over one means a melodic line is playing
            // itself as a chord, which is a bug you can hear and could not
            // previously be told about — the Mozart lead in `gatetest` had
            // 5.7 and nobody knew until it was played out loud.
            let sounding: Vec<usize> = lane
                .pattern
                .all()
                .iter()
                .enumerate()
                .filter(|(_, step)| step.velocity > 0.0)
                .map(|(i, _)| i)
                .collect();
            let steps = lane.pattern.all().len().max(1);
            let tightest = sounding
                .windows(2)
                .map(|w| w[1] - w[0])
                // The pattern loops, so the wrap counts as a gap too.
                .chain(match (sounding.first(), sounding.last()) {
                    (Some(&a), Some(&z)) if sounding.len() > 1 => Some(steps - z + a),
                    _ => None,
                })
                .min();

            let held = match lane.length {
                engine::seq::Length::Steps(n) => n,
                engine::seq::Length::Seconds(s) => s / seconds_per_step,
            };
            // The release runs on after the note ends, and every instrument
            // but the kick has one.
            let release = lane
                .voicing
                .spec()
                .iter()
                .position(|s| s.name == "release")
                .map_or(0.0, |i| lane.voicing.param(i) / seconds_per_step);

            let overlap = tightest.map(|gap| (held + release) / gap as f32);
            let driven = set.macros.iter().any(|m| {
                m.mappings.iter().any(|map| {
                    map.lane == lane.name && map.target == engine::automation::Target::Level
                })
            });

            serde_json::json!({
                "name": lane.name,
                "clip": lane.clip,
                "instrument": lane.voicing.instrument(),
                "pitched": lane.voicing.pitched(),
                // Whether it holds at full level while the note lasts. An
                // instrument with an amplitude decay is already quiet by the
                // time its next note arrives, so overlapping it is an
                // arpeggio; one that sustains makes a cluster instead.
                "sustains": !lane
                    .voicing
                    .spec()
                    .iter()
                    .any(|s| s.name == "decay"),
                "gain": lane.gain,
                "gated": lane.gate.is_some(),
                "notes": sounding.len(),
                "held_steps": held,
                "tightest_gap_steps": tightest,
                "overlap": overlap,
                "level_driven_by_macro": driven,
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string(&out).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn index() -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&engine::library::index()).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn render(args: &[&str]) -> Result<(), String> {
    let p = positional(args);
    let (what, out) = match (p.first(), p.get(1)) {
        (Some(a), Some(b)) => (*a, *b),
        _ => return Err("render needs a piece and an output path".into()),
    };
    let (_, set) = load(what)?;

    let bpm = flag(args, "--bpm")
        .and_then(|b| b.parse().ok())
        .unwrap_or(set.bpm);
    // A whole piece by default: the length it says it is, which is almost
    // always what you meant and never needs a number on the command line.
    let seconds = flag(args, "--seconds")
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(set.length_bars * 240.0 / bpm);

    let mut engine = engine::Engine::new(dsp::SR, bpm, set);
    if let Some(lane) = flag(args, "--solo")
        && !engine.solo(lane)
    {
        return Err(format!(
            "no lane called {lane:?}; lanes: {:?}",
            engine.lane_names()
        ));
    }
    if args.contains(&"--dry") {
        for lane in engine.lane_names() {
            engine.set_send(&lane, 0.0);
        }
    }

    if let Some(parent) = Path::new(out).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let spec = hound::WavSpec {
        channels: 2,
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        sample_rate: dsp::SR as u32,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(out, spec)
        .map_err(|err| format!("could not write {out}: {err}"))?;

    // Block by block, the same call the audio thread makes. The renderer is
    // not a second code path — that is the whole reason offline and live sound
    // the same.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total = (seconds * dsp::SR) as usize;
    let mut block = vec![0.0f32; 512 * 2];
    let mut done = 0;
    while done < total {
        let frames = (total - done).min(512);
        let slice = &mut block[..frames * 2];
        slice.fill(0.0);
        engine.process(slice);
        for s in slice.iter() {
            writer.write_sample(*s).map_err(|err| err.to_string())?;
        }
        done += frames;
    }
    writer.finalize().map_err(|err| err.to_string())?;

    let t = engine.telemetry();
    println!(
        "{out}: {seconds:.3}s at {bpm} BPM, ended at bar {:.2}, {} dropped",
        t.bar, t.dropped
    );
    Ok(())
}
