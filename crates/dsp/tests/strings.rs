//! The string machine: the ensemble, the breath, and the swirl.

use dsp::inst::{Strings, strings};
use dsp::phaser::Phaser;
use dsp::{SR, Voice};

fn render(p: strings::Params, seconds: f32) -> Vec<f32> {
    let mut voice = Strings::new(SR, 55.0, 1.0, seconds, 0.0, p);
    let mut out = vec![0.0f32; ((seconds + p.release + 0.1) * SR) as usize];
    voice.add(&mut out);
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn brightness(x: &[f32]) -> f32 {
    let crossings = x
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    crossings as f32 / (x.len() as f32 / SR)
}

/// White noise, for testing filters.
fn noise(n: usize) -> Vec<f32> {
    let mut state = 7u32;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
        })
        .collect()
}

#[test]
fn an_all_pass_chain_passes_everything() {
    // This is what "all-pass" means, and the whole reason a phaser works: the
    // chain changes every frequency's phase and none of their levels. On its
    // own it is nearly inaudible.
    let input = noise(48_000);
    let mut phaser = Phaser::default();
    phaser.sweep(SR, 0.0, 0.0);
    let wet: Vec<f32> = input.iter().map(|&x| phaser.tick(x, 1.0)).collect();
    let db = 20.0 * (rms(&wet) / rms(&input)).log10();
    assert!(
        db.abs() < 0.5,
        "the all-pass chain changed the level by {db:+.2} dB"
    );
}

#[test]
fn mixing_it_back_is_what_makes_the_notches() {
    // Half dry, half phase-shifted: frequencies that come back half a cycle
    // late cancel, and ones that come back in step survive. Six first-order
    // stages give exactly 180 degrees at the sweep's corner, so that frequency
    // disappears; well below it the chain barely shifts anything and nothing
    // cancels.
    //
    // Measured per frequency rather than broadband, because broadband says
    // almost nothing: the notches are deep but narrow, and over white noise the
    // whole effect is only 0.4 dB of total energy. What the ear hears is where
    // the holes are, not how much is missing.
    let level = |freq: f32| -> f32 {
        let mut phaser = Phaser::default();
        phaser.sweep(SR, 0.0, 0.0); // corner parked at the bottom, 250 Hz
        let n = (1.0 * SR) as usize;
        let mut dry = Vec::with_capacity(n);
        let mut wet = Vec::with_capacity(n);
        let mut phase = 0.0f32;
        for _ in 0..n {
            let x = (std::f32::consts::TAU * phase).sin();
            phase = (phase + freq / SR).fract();
            wet.push(phaser.tick(x, 0.5));
            dry.push(x);
        }
        // Skip the filters settling.
        let from = n / 2;
        20.0 * (rms(&wet[from..]) / rms(&dry[from..])).log10()
    };

    let at_notch = level(250.0);
    let below = level(10.0);
    assert!(
        at_notch < -20.0,
        "the corner should cancel almost completely; it lost {at_notch:.1} dB"
    );
    assert!(
        below > -1.0,
        "well below the corner nothing should cancel; it lost {below:.1} dB"
    );
}

#[test]
fn the_ensemble_beats_and_one_saw_does_not() {
    // Three detuned copies drift in and out of phase with each other, so a held
    // note's level wanders slowly. That wander is the whole illusion: the ear
    // hears players who cannot quite agree rather than three oscillators.
    let steady = strings::Params {
        attack: 0.01,
        release: 0.01,
        breathe: 0.0,
        phaser: 0.0,
        ..strings::Params::default()
    };
    let wander = |p: strings::Params| -> f32 {
        let x = render(p, 4.0);
        let window = (0.1 * SR) as usize;
        let levels: Vec<f32> = (10..35)
            .map(|i| rms(&x[i * window..(i + 1) * window]))
            .collect();
        let mean = levels.iter().sum::<f32>() / levels.len() as f32;
        let spread = levels
            .iter()
            .map(|l| (l - mean).abs())
            .fold(0.0f32, f32::max);
        spread / mean
    };
    let ensemble = wander(steady);
    let single = wander(strings::Params {
        copies: 1.0,
        ..steady
    });
    assert!(
        ensemble > 3.0 * single.max(0.001),
        "three copies wandered {:.1}% and one wandered {:.1}%: no ensemble",
        ensemble * 100.0,
        single * 100.0
    );
}

#[test]
fn breathing_opens_and_shuts_the_filter() {
    let breathing = render(
        strings::Params {
            attack: 0.01,
            breathe: 0.8,
            breathe_rate: 0.5, // fast, so a short render shows a whole cycle
            phaser: 0.0,
            ..strings::Params::default()
        },
        4.0,
    );
    let window = (0.25 * SR) as usize;
    let colours: Vec<f32> = (1..14)
        .map(|i| brightness(&breathing[i * window..(i + 1) * window]))
        .collect();
    let (low, high) = (
        colours.iter().copied().fold(f32::MAX, f32::min),
        colours.iter().copied().fold(0.0, f32::max),
    );
    assert!(
        high > 1.5 * low,
        "the breath changed brightness only from {low:.0} to {high:.0} Hz"
    );
}

#[test]
fn a_pad_arrives_rather_than_strikes() {
    // No attack transient at all. Fifty milliseconds in, a kick is already past
    // its peak; this should still be almost silent.
    let p = strings::Params::default();
    let x = render(p, 3.0);
    assert_eq!(x.first().copied(), Some(0.0));
    let early = rms(&x[..(0.05 * SR) as usize]);
    let settled = rms(&x[(p.attack * SR) as usize..((p.attack + 0.5) * SR) as usize]);
    assert!(
        early < 0.05 * settled,
        "50 ms in it was already at {:.0}% of full: that is a strike, not a swell",
        100.0 * early / settled
    );
    let tail = x[x.len() - 64..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(tail < 1e-3, "release left {tail} hanging");
}

#[test]
fn the_declaration_makes_all_three_things_agree() {
    // The point of declaring a parameter once: the field, the default and the
    // description cannot drift apart, because there is only one of each.
    use dsp::params::Parameters;

    let defaults = strings::Params::default();
    for (index, spec) in strings::Params::SPEC.iter().enumerate() {
        assert_eq!(
            defaults.get(index),
            spec.default,
            "{}'s default and its description disagree",
            spec.name
        );
        assert!(
            spec.min <= spec.default && spec.default <= spec.max,
            "{}'s default {} is outside its own range {}..={}",
            spec.name,
            spec.default,
            spec.min,
            spec.max
        );
        assert!(!spec.doc.is_empty(), "{} has no description", spec.name);

        // And a value from outside the range cannot get in.
        let mut params = defaults;
        params.set(index, 1e9);
        assert!(
            params.get(index) <= spec.max,
            "{} was not clamped",
            spec.name
        );
        params.set(index, -1e9);
        assert!(
            params.get(index) >= spec.min,
            "{} was not clamped",
            spec.name
        );
    }
    println!(
        "strings has {} parameters, {} of them logarithmic",
        strings::Params::SPEC.len(),
        strings::Params::SPEC
            .iter()
            .filter(|s| s.logarithmic)
            .count()
    );
}
