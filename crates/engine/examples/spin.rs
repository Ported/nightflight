//! Spin a 1 kHz sine around the head and write it to a WAV.
//!
//! This is the Python studio's quality metric for placement, reproduced: a pure
//! tone on a moving source should stay a pure tone. Anything else — stepping
//! delays, filters that jump between blocks, a dull interpolator whose dullness
//! varies — shows up as energy away from 1 kHz, which `tools/analyse.py --tone
//! 1000` measures. For reference, the numpy renderer manages -164 dB standing
//! still and -55 dB at track speed.
//!
//! ```text
//! cargo run -p engine --example spin -- [seconds] [path] [--block N] [--still]
//! ```

use dsp::osc::{Phasor, sine};
use dsp::space::{Ears, Motion, Placer, Position};
use engine::seq::Home;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let seconds: f32 = positional
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8.0);
    let path = positional
        .get(1)
        .map_or_else(|| "renders/spin.wav".to_string(), |s| (*s).clone());
    let block: usize = value("--block").and_then(|s| s.parse().ok()).unwrap_or(512);
    let still = args.iter().any(|a| a == "--still");

    // The hats' orbit: 1.5 m out, a lap every four bars at 126 BPM.
    let home = Home::Orbit {
        radius: 1.5,
        bars_per_lap: 4.0,
        phase: 0.0,
        elevation: 0.0,
        height: 0.4,
        height_bars: 6.0,
    };
    let bar_seconds = 4.0 * 60.0 / 126.0;

    let mut placer = Placer::new();
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: dsp::SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec).expect("create wav");

    // The tone comes from a wrapped phase accumulator, not from absolute time.
    // `sin(TAU * 1000 * t)` looks equivalent and is not: by eight seconds the
    // argument is 50,000 radians, and f32 rounding at that magnitude injects
    // about -50 dB of phase noise — which would swamp exactly what is being
    // measured here.
    let mut phase = Phasor::default();
    let mut mono = vec![0.0f32; block];
    let (mut left, mut right) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut ignored_send = vec![0.0f32; block];
    let total = (seconds * dsp::SR) as usize;
    let mut done = 0;
    while done < total {
        let bar = |sample: usize| (sample as f64 / f64::from(dsp::SR)) / bar_seconds;
        for s in mono.iter_mut() {
            *s = sine(phase.tick(1000.0, dsp::SR));
        }
        let (from, to) = if still {
            let fixed = Position::new(1.0, 0.0, -1.0);
            (fixed, fixed)
        } else {
            (
                // The orbit's angle is an accumulated lap count now, and at a
                // constant rate that is simply the bar times laps per bar.
                home.at(bar(done) * home.laps_per_bar(), bar(done))
                    .expect("orbit is placed"),
                home.at(bar(done + block) * home.laps_per_bar(), bar(done + block))
                    .expect("orbit is placed"),
            )
        };
        left.fill(0.0);
        right.fill(0.0);
        let mut ears = Ears {
            left: &mut left,
            right: &mut right,
            send: &mut ignored_send,
            send_level: 0.0,
        };
        let motion = Motion {
            from,
            to,
            offset: 0,
            period: block,
        };
        placer.place(&mono, motion, &mut ears, dsp::SR);
        for (l, r) in left.iter().zip(&right) {
            writer.write_sample(*l).expect("write");
            writer.write_sample(*r).expect("write");
        }
        done += block;
    }
    writer.finalize().expect("finalize");
    println!(
        "{path}: {seconds}s, {} block, {}",
        block,
        if still { "static" } else { "orbiting" }
    );
}
