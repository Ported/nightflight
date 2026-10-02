//! What a clap must do, measured rather than listened to.

use dsp::inst::{Clap, clap};
use dsp::{SR, Voice};

fn render(p: clap::Params, length: f32) -> Vec<f32> {
    let mut voice = Clap::new(SR, 1.0, length, p);
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
    let out = render(clap::Params::default(), 0.3);
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
fn the_bursts_refire_the_envelope() {
    // At spread 11 ms and a 5 ms burst, the level must dip before each
    // refire and jump back up at it. Levels in windows, not onset counting:
    // the window just before the second burst against the window just after
    // it. With a single burst the same pair of windows must show plain
    // decay instead — same instrument, same windows, opposite verdict.
    let p = clap::Params::default();
    let out = render(p, 0.3);
    let before = rms(&out, 0.007, 0.0105);
    let after = rms(&out, 0.011, 0.015);
    assert!(
        after > 1.5 * before,
        "second burst: {after} after vs {before} before the refire — no clap, just one hit"
    );

    let one = clap::Params {
        bursts: 1.0,
        ..Default::default()
    };
    let out = render(one, 0.3);
    let before = rms(&out, 0.007, 0.0105);
    let after = rms(&out, 0.011, 0.015);
    assert!(
        after < before,
        "with one burst the envelope must only fall: {before} then {after}"
    );
}

#[test]
fn the_tail_is_the_decay_knob() {
    let long = clap::Params {
        decay: 0.5,
        ..Default::default()
    };
    let short = clap::Params {
        decay: 0.05,
        ..Default::default()
    };
    let ringing = rms(&render(long, 0.6), 0.25, 0.35);
    let dead = rms(&render(short, 0.6), 0.25, 0.35);
    assert!(
        ringing > 4.0 * dead,
        "decay 0.5 left {ringing} in the tail window, 0.05 left {dead}"
    );
}

#[test]
fn tone_moves_the_band() {
    // Narrowband noise crosses zero at about twice its centre frequency, so
    // the crossing count in the tail is a pitch meter for the bandpass.
    //
    // *Narrowband.* The first version ran at the default ring of 0.65, where
    // the band is deliberately broad, and the broadband skirt dragged both
    // counts toward each other: 852 against 436, a ratio of 1.95 for a centre
    // ratio of 4.2. The band was moving fine; the meter assumed a Q the patch
    // did not have. Ring 0.9 narrows the band until the model holds.
    let crossings = |tone: f32| {
        let p = clap::Params {
            tone,
            ring: 0.9,
            ..Default::default()
        };
        let out = render(p, 0.3);
        out[(0.05 * SR) as usize..(0.15 * SR) as usize]
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    };
    let (low, high) = (crossings(600.0), crossings(2500.0));
    assert!(
        high as f32 > 2.0 * low as f32,
        "tone 2500 crossed {high} times, tone 600 crossed {low}: the band is not moving"
    );
}
