//! Placement: the geometry, the delay line, and the two cues.

use dsp::SR;
use dsp::space::{Delay, Ears, HEAD_RADIUS, Motion, Placer, Position, SPEED_OF_SOUND, woodworth};

/// Place a short impulse and return both ears.
fn place(position: Position, n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut placer = Placer::new();
    let mut mono = vec![0.0f32; n];
    mono[0] = 1.0;
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut send = vec![0.0f32; n];
    // One block, so the position is static.
    let mut ears = Ears {
        left: &mut left,
        right: &mut right,
        send: &mut send,
        send_level: 0.0,
    };
    let motion = Motion {
        from: position,
        to: position,
        offset: 0,
        period: n,
    };
    placer.place(&mono, motion, &mut ears, SR);
    (left, right)
}

fn peak_index(x: &[f32]) -> usize {
    x.iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map_or(0, |(i, _)| i)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn woodworth_matches_the_spherical_head() {
    // Straight ahead: no difference at all.
    assert!(woodworth(0.0).abs() < 1e-9);
    // Hard right: the arc plus the straight part, (a/c)(pi/2 + 1) = 0.80 ms.
    let full = HEAD_RADIUS / SPEED_OF_SOUND * (std::f32::consts::FRAC_PI_2 + 1.0);
    assert!((woodworth(1.0) - full).abs() < 1e-9);
    assert!((0.79e-3..0.81e-3).contains(&woodworth(1.0)));
    // Symmetric, and positive means the source is on the right.
    assert!((woodworth(-1.0) + woodworth(1.0)).abs() < 1e-9);
    // Monotonic across the front, so a source crossing the head never jumps.
    let mut previous = woodworth(-1.0);
    for i in -9..=10 {
        let next = woodworth(i as f32 / 10.0);
        assert!(next > previous, "ITD went backwards at {i}");
        previous = next;
    }
}

#[test]
fn the_delay_line_reads_back_what_was_written() {
    let mut delay = Delay::new();
    for i in 0..500 {
        delay.push(if i == 0 { 1.0 } else { 0.0 });
    }
    // 499 samples have been pushed since the impulse.
    assert!(
        (delay.read(499.0) - 1.0).abs() < 1e-6,
        "integer delay lost the impulse"
    );
    assert!(
        delay.read(250.0).abs() < 1e-6,
        "found something that was never written"
    );
}

#[test]
fn a_fractional_delay_does_not_dull_the_top_end() {
    // The reason for 4-point Lagrange instead of linear interpolation. Reading
    // half a sample late is a filter, and linear interpolation is a bad one: at
    // 10 kHz, where hats live, it costs about 2 dB, and the loss varies with the
    // fraction, so a source moving smoothly shimmers. Rather than take that on
    // trust, measure both on the same signal.
    let freq = 10_000.0;
    let mut delay = Delay::new();
    let mut raw = Vec::new();
    let (mut lagrange, mut linear) = (0.0f32, 0.0f32);
    for i in 0..4000 {
        let t = i as f32 / SR;
        let x = (std::f32::consts::TAU * freq * t).sin();
        raw.push(x);
        delay.push(x);
        if i > 200 {
            lagrange = lagrange.max(delay.read(100.5).abs());
            // The same read, done the cheap way: halfway between two samples.
            let a = raw[i - 100];
            let b = raw[i - 101];
            linear = linear.max((0.5 * (a + b)).abs());
        }
    }
    let (good, bad) = (20.0 * lagrange.log10(), 20.0 * linear.log10());
    // Theory: -0.53 dB for cubic Lagrange at this frequency, -2.01 for linear.
    assert!(good > -0.8, "Lagrange lost {good:.2} dB at 10 kHz");
    assert!(
        good - bad > 1.0,
        "Lagrange ({good:.2} dB) should beat linear ({bad:.2} dB) by over a dB"
    );
}

#[test]
fn a_centred_source_reaches_both_ears_alike() {
    let (left, right) = place(Position::new(0.0, 0.0, -1.5), 600);
    let difference = left
        .iter()
        .zip(&right)
        .map(|(l, r)| (l - r).abs())
        .fold(0.0f32, f32::max);
    assert!(
        difference < 1e-9,
        "the ears differ by {difference} dead ahead"
    );
    assert!(peak(&left) > 0.0, "nothing arrived");
}

#[test]
fn a_source_on_the_right_arrives_earlier_and_louder_on_the_right() {
    let (left, right) = place(Position::new(1.5, 0.0, 0.0), 600);

    let itd = peak_index(&left) as i64 - peak_index(&right) as i64;
    // 38.5 samples of Woodworth ITD, plus a few from the shadow lowpass's own
    // group delay on the far ear — this is a peak, not a phase measurement.
    assert!(
        (30..55).contains(&itd),
        "wanted about 40 samples of ITD, got {itd}"
    );

    let ild_db = 20.0 * (peak(&right) / peak(&left)).log10();
    // Broadband shadowing is -14 dB; the far ear also loses its top, and an
    // impulse is mostly top.
    assert!(
        (10.0..30.0).contains(&ild_db),
        "wanted a strong level difference, got {ild_db:.1} dB"
    );

    // Travel: 1.5 m is 210 samples at 343 m/s.
    let travel = (1.5 / SPEED_OF_SOUND * SR) as i64;
    let arrival = peak_index(&right) as i64;
    assert!(
        (arrival - travel).abs() < 4,
        "the near ear heard it at {arrival}, expected about {travel}"
    );
}

#[test]
fn front_and_back_are_indistinguishable_for_now() {
    // Not a bug: front/back and height live entirely in the folds of the outer
    // ear, and there is no measured head response here yet. This test exists so
    // that when the HRTF lands, it fails — and has to be rewritten.
    let (front_l, front_r) = place(Position::new(0.6, 0.0, -1.2), 600);
    let (back_l, back_r) = place(Position::new(0.6, 0.0, 1.2), 600);
    let difference = front_l
        .iter()
        .chain(&front_r)
        .zip(back_l.iter().chain(&back_r))
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        difference < 1e-9,
        "front and back already differ by {difference}: has an HRTF landed?"
    );
}
