//! What a cymbal must do, measured rather than listened to.

use dsp::inst::{Cymbal, cymbal};
use dsp::{SR, Voice};

fn render(p: cymbal::Params, length: f32) -> Vec<f32> {
    let mut voice = Cymbal::new(SR, 1.0, length, 7, p);
    let mut out = Vec::new();
    let mut block = [0.0f32; 64];
    // A crash rings for seconds: give it room.
    for _ in 0..4000 {
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
    let out = render(cymbal::Params::default(), 1.0);
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
fn the_wash_rings_as_long_as_asked() {
    // Measured at a full second in, where the strike (0.3 s at most) is
    // certainly gone: only the wash can be sounding there.
    let crash = cymbal::Params {
        decay: 3.0,
        ..Default::default()
    };
    let tick = cymbal::Params {
        decay: 0.3,
        ..Default::default()
    };
    let ringing = rms(&render(crash, 2.0), 1.0, 1.2);
    let dead = rms(&render(tick, 2.0), 1.0, 1.2);
    assert!(
        ringing > 4.0 * dead,
        "decay 3.0 left {ringing} at one second, 0.3 left {dead}"
    );
}

#[test]
fn the_strike_is_the_front_of_the_note() {
    let pinged = cymbal::Params {
        strike: 2.0,
        ..Default::default()
    };
    let washed = cymbal::Params {
        strike: 0.0,
        ..Default::default()
    };
    let with = rms(&render(pinged, 1.0), 0.0, 0.04);
    let without = rms(&render(washed, 1.0), 0.0, 0.04);
    assert!(
        with > 1.3 * without,
        "strike 2 opened at {with}, strike 0 at {without}: the ping is not landing"
    );
}

#[test]
fn a_grabbed_cymbal_stops() {
    let open = rms(&render(cymbal::Params::default(), 2.0), 0.5, 0.7);
    let grabbed = render(cymbal::Params::default(), 0.1);
    let after = if grabbed.len() > (0.5 * SR) as usize {
        rms(&grabbed, 0.5, 0.7)
    } else {
        0.0
    };
    assert!(
        open > 1e-3,
        "an open crash should still ring at half a second"
    );
    assert!(
        after < 1e-4,
        "choked at 0.1 s it left {after} at half a second: the grab is not grabbing"
    );
}
