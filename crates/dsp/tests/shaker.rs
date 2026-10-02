//! What a shaker must do, measured rather than listened to.

use dsp::inst::{Shaker, shaker};
use dsp::{SR, Voice};

fn render(p: shaker::Params, length: f32, seed: u32) -> Vec<f32> {
    let mut voice = Shaker::new(SR, 1.0, length, seed, p);
    let mut out = Vec::new();
    let mut block = [0.0f32; 64];
    for _ in 0..1000 {
        block.fill(0.0);
        voice.add(&mut block);
        out.extend_from_slice(&block);
        if voice.finished() {
            break;
        }
    }
    out
}

fn rms(x: &[f32], from: f32, to: f32) -> f32 {
    let window = &x[(from * SR) as usize..((to * SR) as usize).min(x.len())];
    (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt()
}

#[test]
fn it_starts_and_ends_on_a_ramp() {
    let out = render(shaker::Params::default(), 0.15, 7);
    assert_eq!(
        out.first().copied(),
        Some(0.0),
        "attack must start at silence"
    );
    let tail = out[out.len() - 8..]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(tail < 1e-4, "release left {tail} hanging: that is a click");
}

#[test]
fn the_front_is_soft() {
    // The grains take `attack` to land, so with a 30 ms throw the first
    // five milliseconds must be quieter than the peak that follows. A hat
    // would fail this test — which is the difference being asserted.
    let p = shaker::Params {
        attack: 0.03,
        ..Default::default()
    };
    let out = render(p, 0.2, 7);
    let front = rms(&out, 0.0, 0.005);
    let bloom = rms(&out, 0.025, 0.035);
    assert!(
        bloom > 2.0 * front,
        "front {front}, bloom {bloom}: the chick should build, not bite"
    );
}

#[test]
fn decay_sets_the_fall() {
    let long = shaker::Params {
        decay: 0.25,
        ..Default::default()
    };
    let short = shaker::Params {
        decay: 0.03,
        ..Default::default()
    };
    let ringing = rms(&render(long, 0.4, 7), 0.15, 0.25);
    let dead = rms(&render(short, 0.4, 7), 0.15, 0.25);
    assert!(
        ringing > 4.0 * dead,
        "decay 0.25 left {ringing} at 150-250 ms, 0.03 left {dead}"
    );
}

#[test]
fn no_two_shakes_match_but_renders_repeat() {
    let a = render(shaker::Params::default(), 0.15, 1);
    let b = render(shaker::Params::default(), 0.15, 2);
    let again = render(shaker::Params::default(), 0.15, 1);
    assert_eq!(a, again, "the same seed must render bit-identically");
    assert_ne!(a, b, "different seeds must shake differently");
}
