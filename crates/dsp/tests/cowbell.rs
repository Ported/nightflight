//! What a cowbell must do, measured rather than listened to.

use dsp::inst::{Cowbell, cowbell};
use dsp::{SR, Voice};

/// MIDI 72 is C5, where the 808's cowbell sits.
const PITCH: f32 = 72.0;

fn render(p: cowbell::Params, pitch: f32, length: f32) -> Vec<f32> {
    let mut voice = Cowbell::new(SR, pitch, 1.0, length, p);
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
    let out = render(cowbell::Params::default(), PITCH, 0.3);
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
fn the_root_tunes_it() {
    // An octave up should roughly double the zero crossings: both modes
    // scale with the root, and the bandpass follows them.
    let crossings = |pitch: f32| {
        let out = render(cowbell::Params::default(), pitch, 0.3);
        out[..(0.1 * SR) as usize]
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    };
    let (low, high) = (crossings(60.0), crossings(72.0));
    assert!(
        high as f32 > 1.5 * low as f32,
        "C5 crossed {high} times, C4 crossed {low}: the root is not tuning it"
    );
}

#[test]
fn decay_is_how_long_the_box_rings() {
    let long = cowbell::Params {
        decay: 0.6,
        ..Default::default()
    };
    let short = cowbell::Params {
        decay: 0.08,
        ..Default::default()
    };
    let ringing = rms(&render(long, PITCH, 0.7), 0.3, 0.4);
    let dead = rms(&render(short, PITCH, 0.7), 0.3, 0.4);
    assert!(
        ringing > 4.0 * dead,
        "decay 0.6 left {ringing} at 0.3 s, 0.08 left {dead}"
    );
}

#[test]
fn the_clank_is_the_first_millisecond() {
    // The clank is highpassed noise and noise is jumps, so the largest
    // sample-to-sample jump inside the first two milliseconds must grow
    // when the clank is turned up.
    let jump = |clank: f32| {
        let p = cowbell::Params {
            clank,
            ..Default::default()
        };
        render(p, PITCH, 0.3)[..(0.002 * SR) as usize]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    let (without, with) = (jump(0.0), jump(1.0));
    assert!(
        with > 1.3 * without,
        "clank 1 jumped {with} in the first 2 ms, clank 0 jumped {without}"
    );
}
