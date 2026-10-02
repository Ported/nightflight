//! What a rim must do, measured rather than listened to.

use dsp::inst::{Rim, rim};
use dsp::{SR, Voice};

/// MIDI 83 is B5, about 988 Hz: rimshot territory.
const PITCH: f32 = 83.0;

fn render(p: rim::Params, length: f32) -> Vec<f32> {
    let mut voice = Rim::new(SR, PITCH, 1.0, length, p);
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
    let out = render(rim::Params::default(), 0.15);
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
fn it_is_short_because_that_is_the_point() {
    // A rim that still sounds at 150 ms is a tom. Default decay is 40 ms,
    // so 200 ms is five time constants out — and five time constants is
    // 0.7%, not zero. The first version demanded under 0.1% and failed on a
    // correct exponential: a decay never arrives, it only approaches, and
    // the number that is actually zero is the gate's cosine at the very
    // end, which the ramp test already covers.
    let out = render(rim::Params::default(), 0.3);
    let late = rms(&out, 0.2, 0.25);
    assert!(late < 5e-3, "a default rim left {late} at 200 ms: too long");
}

#[test]
fn decay_sets_the_ring() {
    let long = rim::Params {
        decay: 0.12,
        ..Default::default()
    };
    let short = rim::Params {
        decay: 0.015,
        ..Default::default()
    };
    let ringing = rms(&render(long, 0.3), 0.06, 0.1);
    let dead = rms(&render(short, 0.3), 0.06, 0.1);
    assert!(
        ringing > 4.0 * dead,
        "decay 0.12 left {ringing} at 60-100 ms, 0.015 left {dead}"
    );
}

#[test]
fn the_second_mode_is_higher() {
    // The shell mode rides at 2.4x the ping, so turning it on must raise
    // the zero-crossing count in the ring. Measured after the click is
    // over: the click is noise and would swamp any crossing count.
    let crossings = |second: f32| {
        let p = rim::Params {
            second,
            click: 0.0,
            ..Default::default()
        };
        let out = render(p, 0.15);
        out[(0.01 * SR) as usize..(0.04 * SR) as usize]
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    };
    let (ping, shell) = (crossings(0.0), crossings(1.0));
    assert!(
        shell as f32 > 1.3 * ping as f32,
        "with the shell mode {shell} crossings, without {ping}"
    );
}
