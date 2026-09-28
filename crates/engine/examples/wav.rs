//! Render the engine to a WAV, for checking the numbers without a sound card.
//!
//! ```text
//! cargo run -p engine --example wav -- [seconds] [path] [--bpm N] [--solo name] [--mute name] [--dry] [--send-scale X] [--set name] [--gate depth] [--gain X]
//! ```

use engine::Engine;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let positional: Vec<&String> = {
        let mut out = Vec::new();
        let mut skip = false;
        for a in &args {
            if skip {
                skip = false;
            } else if a.starts_with("--") {
                skip = true;
            } else {
                out.push(a);
            }
        }
        out
    };
    let seconds: f32 = positional
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4.0);
    let path = positional
        .get(1)
        .map_or_else(|| "renders/out.wav".to_string(), |s| (*s).clone());

    let bpm: f32 = flag("--bpm").and_then(|s| s.parse().ok()).unwrap_or(126.0);
    let name = flag("--set").unwrap_or_else(|| "rolling".to_string());
    let Some(set) = engine::sets::by_name(&name) else {
        eprintln!("no set named {name:?}; have {:?}", engine::sets::NAMES);
        std::process::exit(1);
    };
    let mut engine = Engine::new(dsp::SR, bpm, set);
    if args.iter().any(|a| a == "--dry") {
        for name in engine.lane_names() {
            engine.set_send(&name, 0.0);
        }
    }
    if let Some(scale) = flag("--gain").and_then(|s| s.parse::<f32>().ok()) {
        engine.set_master(engine.master() * scale);
    }
    if let Some(depth) = flag("--gate").and_then(|s| s.parse::<f32>().ok()) {
        for name in engine.lane_names() {
            engine.set_gate_depth(&name, depth);
        }
    }
    if let Some(scale) = flag("--send-scale").and_then(|s| s.parse::<f32>().ok()) {
        for (name, send) in engine.sends() {
            engine.set_send(&name, send * scale);
        }
    }
    if let Some(name) = flag("--solo")
        && !engine.solo(&name)
    {
        eprintln!("no lane named {name:?}; lanes: {:?}", engine.lane_names());
        std::process::exit(1);
    }
    for (i, arg) in args.iter().enumerate() {
        if arg == "--mute"
            && let Some(name) = args.get(i + 1)
            && !engine.set_muted(name, true)
        {
            eprintln!("no lane named {name:?}; lanes: {:?}", engine.lane_names());
            std::process::exit(1);
        }
    }

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: dsp::SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec).expect("create wav");

    // An awkward block size on purpose: nothing here may depend on it.
    let mut block = [0.0f32; 137 * 2];
    let total = (seconds * dsp::SR) as usize;
    let mut written = 0;
    while written < total {
        engine.process(&mut block);
        for &s in &block {
            writer.write_sample(s).expect("write");
        }
        written += block.len() / 2;
    }
    writer.finalize().expect("finalize");
    let state = engine.state();
    println!(
        "{path}: {seconds}s at {bpm} BPM, ended at bar {:.2}, {} voices live, {} dropped",
        state.bar, state.voices, state.dropped
    );
}
