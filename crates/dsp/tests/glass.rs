//! FM synthesis: that a whole-number ratio stays in tune, and that the sound
//! darkens as it rings.

use dsp::filter::{Mode, Svf};
use dsp::inst::{Glass, glass};
use dsp::{SR, Voice, hz};

fn render(p: glass::Params, pitch: f32, length: f32) -> Vec<f32> {
    let mut voice = Glass::new(SR, pitch, 1.0, length, 0.0, p);
    let mut out = vec![0.0f32; ((length + p.release + 0.1) * SR) as usize];
    voice.add(&mut out);
    out
}

/// How well the waveform repeats after `period` samples, from 0 to 1. This is
/// the definition of having a pitch: a periodic wave has one, and how nearly
/// periodic it is decides how clearly.
fn periodicity(x: &[f32], period: f32) -> f32 {
    let lag = period.round() as usize;
    let (mut dot, mut energy) = (0.0f64, 0.0f64);
    for i in 0..x.len() - lag {
        dot += f64::from(x[i]) * f64::from(x[i + lag]);
        energy += f64::from(x[i]) * f64::from(x[i]);
    }
    (dot / energy.max(1e-12)) as f32
}

fn brightness(x: &[f32]) -> f32 {
    let crossings = x
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    crossings as f32 / (x.len() as f32 / SR)
}

#[test]
fn a_whole_number_ratio_stays_in_tune() {
    // The heart of FM: modulating a carrier adds partials at carrier plus and
    // minus whole multiples of the modulator. If the modulator is a whole
    // number times the carrier, every one of those lands on a harmonic of the
    // carrier, so the waveform still repeats at the carrier's period and the
    // ear hears a note. Detune is off here, since two carriers beating against
    // each other break the repetition on purpose.
    let pitch = 55.0; // G3, 196 Hz
    let period = SR / hz(pitch);
    let tuned = glass::Params {
        detune: 0.0,
        strike: 0.0,
        index: 2.5,
        index_decay: 10.0, // hold the brightness steady for the measurement
        ..glass::Params::default()
    };

    let integer = periodicity(
        &render(
            glass::Params {
                ratio: 3.0,
                ..tuned
            },
            pitch,
            1.0,
        ),
        period,
    );
    assert!(
        integer > 0.9,
        "ratio 3 should repeat at the carrier's period; got {integer:.3}"
    );

    // And a ratio between whole numbers does not: the partials fall between the
    // harmonics, the waveform never repeats, and the ear gives up on pitch and
    // hears metal. It is the same reason the 808's six unrelated squares sound
    // like a cymbal.
    let inharmonic = periodicity(
        &render(
            glass::Params {
                ratio: 3.41,
                ..tuned
            },
            pitch,
            1.0,
        ),
        period,
    );
    assert!(
        inharmonic < 0.5,
        "ratio 3.41 should not repeat at the carrier's period; got {inharmonic:.3}"
    );
}

#[test]
fn it_starts_bright_and_melts() {
    // What makes it glass rather than an organ: a struck object is brightest at
    // the instant it is hit. Letting the modulation index fall away does that,
    // and the spectrum narrows towards a nearly pure tone.
    let p = glass::Params {
        strike: 0.0,
        index: 3.0,
        index_decay: 0.12,
        decay: 2.0,
        ..glass::Params::default()
    };
    let x = render(p, 55.0, 1.0);
    let early = brightness(&x[(0.005 * SR) as usize..(0.05 * SR) as usize]);
    let late = brightness(&x[(0.7 * SR) as usize..(0.95 * SR) as usize]);
    assert!(
        late < 0.6 * early,
        "the tone did not melt: {early:.0} Hz early, {late:.0} Hz late"
    );

    // With the index held up it should not melt.
    let steady = render(
        glass::Params {
            index_decay: 100.0,
            ..p
        },
        55.0,
        1.0,
    );
    let early = brightness(&steady[(0.005 * SR) as usize..(0.05 * SR) as usize]);
    let late = brightness(&steady[(0.7 * SR) as usize..(0.95 * SR) as usize]);
    assert!(
        (late / early - 1.0).abs() < 0.3,
        "with the index held, brightness still changed: {early:.0} -> {late:.0} Hz"
    );
}

#[test]
fn the_strike_puts_something_high_at_the_onset() {
    // A near-pure tone cannot be placed in space: the cues for front, back and
    // height all live above about 4 kHz, so it is worth checking
    // a voice's share of energy up there before spatialising it. The strike is
    // a few milliseconds of high noise that gives the ear an onset to locate.
    //
    // Measured by filtering, not by counting zero crossings: over the four
    // milliseconds the strike lasts, a crossing count has a resolution of
    // 250 Hz, which is too coarse to say anything.
    let above_4k = |x: &[f32], hz: f32| -> f32 {
        let mut filter = Svf::new(SR, hz, 0.0);
        x.iter()
            .map(|&s| {
                let high = filter.tick(s, Mode::High);
                high * high
            })
            .sum::<f32>()
    };
    // Isolated by difference, since the two renders are identical but for the
    // strike. Measured at 8 kHz rather than 4: at index 2.2 the tone's own
    // attack is bright enough to fill 4-8 kHz on its own (the strike is 5.8 dB
    // *under* it there), and only higher up does the strike take over.
    let window = (0.01 * SR) as usize;
    let tone = render(
        glass::Params {
            strike: 0.0,
            ..glass::Params::default()
        },
        55.0,
        0.5,
    );
    let full = render(glass::Params::default(), 55.0, 0.5);
    let strike: Vec<f32> = full.iter().zip(&tone).map(|(a, b)| a - b).collect();

    let db = 10.0
        * (above_4k(&strike[..window], 8000.0) / above_4k(&tone[..window], 8000.0).max(1e-20))
            .log10();
    assert!(
        db > 3.0,
        "above 8 kHz the strike is only {db:+.1} dB against the tone: nothing to locate"
    );
}

#[test]
fn it_starts_and_ends_on_a_ramp() {
    let x = render(glass::Params::default(), 55.0, 0.5);
    assert_eq!(
        x.first().copied(),
        Some(0.0),
        "attack must start at silence"
    );
    let tail = x[x.len() - 64..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(tail < 1e-3, "release left {tail} hanging");
    assert!(x.iter().all(|s| s.is_finite()), "non-finite sample");
}
