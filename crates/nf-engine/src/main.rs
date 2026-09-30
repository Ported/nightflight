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

use std::path::{Path, PathBuf};
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
    let path = PathBuf::from(what);
    let (name, mut set) = if path.extension().is_some_and(|e| e == "tab") {
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("could not read {}: {err}", path.display()))?;
        engine::tab::read(&text).map_err(|why| format!("{}: {why}", path.display()))?
    } else {
        let path = if path.extension().is_some_and(|e| e == "json") {
            path
        } else {
            engine::document::directory().join(format!("{what}.json"))
        };
        match engine::document::load(&path) {
            Ok(piece) => (piece.name, piece.set),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let set = engine::sets::by_name(what).ok_or_else(|| {
                    format!(
                        "no piece, file or built-in called {what:?}.\nbuilt in: {:?}",
                        engine::sets::NAMES
                    )
                })?;
                (what.to_string(), set)
            }
            Err(err) => return Err(format!("could not read {}: {err}", path.display())),
        }
    };
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

fn index() -> Result<(), String> {
    let library = engine::library::index();
    let mut pieces: Vec<serde_json::Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(engine::document::directory()) {
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json" || e == "tab"))
            .collect();
        found.sort();
        for path in found {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok((_, set)) = load(path.to_str().unwrap_or(stem)) else {
                continue;
            };
            let mut clips: Vec<&str> = Vec::new();
            for lane in &set.lanes {
                if !clips.contains(&lane.clip.as_str()) {
                    clips.push(&lane.clip);
                }
            }
            pieces.push(serde_json::json!({
                "name": stem,
                "bpm": set.bpm,
                "bars": set.length_bars,
                "lanes": set.lanes.len(),
                "clips": clips,
                "path": path,
            }));
        }
    }
    let out = serde_json::json!({
        "pieces": pieces,
        "patches": library.patches,
        "clips": library.clips,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
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
