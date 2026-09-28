//! Dump the reverb's impulse response to a WAV, so its decay can be measured.
//!
//! `cargo run -p dsp --example ir -- [rt60] [damping] [path]`

use dsp::SAMPLE_RATE;
use dsp::reverb::Reverb;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rt60: f32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(7.0);
    let damping: f32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.6);
    let path = args
        .get(2)
        .map_or_else(|| "renders/ir.wav".to_string(), |s| s.clone());

    let mut reverb = Reverb::new(dsp::SR, rt60, damping, 0.03, 100.0);
    let n = ((rt60 * 1.3 + 0.2) * dsp::SR) as usize;
    let mut bus = vec![0.0f32; n];
    bus[0] = 1.0;
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    reverb.process(&bus, &mut left, &mut right, 1.0);

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec).expect("create wav");
    for (l, r) in left.iter().zip(&right) {
        writer.write_sample(*l).expect("write");
        writer.write_sample(*r).expect("write");
    }
    writer.finalize().expect("finalize");

    let energy: f32 = left.iter().chain(&right).map(|s| s * s).sum();
    println!(
        "{path}: rt60 {rt60}s damping {damping}, peak {:.4}, energy per ear {:.4}",
        left.iter()
            .chain(&right)
            .fold(0.0f32, |m, s| m.max(s.abs())),
        (energy / 2.0).sqrt()
    );
}
