//! What a kick must do, measured rather than listened to.

use dsp::inst::{Kick, kick};
use dsp::{SR, Voice};

fn render(p: kick::Params, length: f32) -> Vec<f32> {
    let mut voice = Kick::new(SR, 31.0, 1.0, length, p);
    let mut out = Vec::new();
    let mut block = [0.0f32; 64];
    // Well past the note: stop when the voice says it has rung out.
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

fn max_jump(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn it_starts_and_ends_on_a_ramp() {
    let out = render(kick::Params::default(), 0.4);
    assert_eq!(
        out.first().copied(),
        Some(0.0),
        "attack must start at silence"
    );
    // The release is a raised cosine, so the tail arrives at zero, not near it.
    let tail = out[out.len() - 8..]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(tail < 1e-4, "release left {tail} hanging: that is a click");
}

#[test]
fn it_lasts_as_long_as_it_was_asked_to() {
    let out = render(kick::Params::default(), 0.4);
    let seconds = out.len() as f32 / SR;
    // 0.4 s of note plus the 20 ms release, rounded up to whole blocks.
    assert!(
        (0.42..0.43).contains(&seconds),
        "wanted about 0.42 s, got {seconds}"
    );
}

#[test]
fn the_sub_alone_is_smooth() {
    // With the noise layers off, every sample follows from the last: a jump
    // bigger than the sweep's own steepest slope would be a discontinuity.
    let p = kick::Params {
        click: 0.0,
        knock: 0.0,
        ..kick::Params::default()
    };
    let out = render(p, 0.4);
    // The sweep starts at 11 x 49 Hz = 539 Hz, so the raw slope is at most
    // 2*pi*539/48000 = 0.071, which the drive of 3 steepens near the zero
    // crossing by about 3x.
    let jump = max_jump(&out);
    assert!(
        jump < 0.3,
        "jump {jump} is steeper than the waveform allows"
    );
}

#[test]
fn punch_starts_the_sweep_higher() {
    // The whole point of kick B: a second, very fast pitch drop from high up.
    // It cannot be measured as loudness — with drive 3.0 the sine is squashed
    // nearly into a square wave, so the energy in the first milliseconds is the
    // same whatever the pitch. So count cycles instead.
    //
    // With the noise layers *on* this measure inverts, which is worth knowing:
    // the punched sine sits harder against the saturator's ceiling, so the
    // click cannot push it back across zero, while a lower pitch lingers near
    // zero where noise adds crossings that are not cycles. Isolate the sub.
    let quiet = kick::Params {
        click: 0.0,
        knock: 0.0,
        ..kick::Params::default()
    };
    let cycles = |p: kick::Params| -> usize {
        let out = render(p, 0.4);
        let n = (0.012 * SR) as usize;
        out[..n]
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count()
    };
    let with = cycles(quiet);
    let without = cycles(kick::Params {
        punch: 0.0,
        ..quiet
    });
    assert!(
        with > without,
        "punch did not raise the early pitch: {with} vs {without} cycles in 12 ms"
    );
}
