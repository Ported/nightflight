//! The clock and the pump: two mechanisms where being wrong is inaudible until
//! it is embarrassing.

use engine::Engine;
use engine::mix::Duck;

/// Render `frames` frames and return the sample index of every onset.
///
/// Onsets come from a smoothed envelope with hysteresis, not from the waveform
/// crossing a threshold: a 49 Hz kick crosses any level you like every ten
/// milliseconds, so a bare threshold counts one hit four times over.
fn onsets(engine: &mut Engine, frames: usize) -> Vec<usize> {
    let mut buf = vec![0.0f32; 128 * 2];
    let mut mono = Vec::with_capacity(frames);
    while mono.len() < frames {
        engine.process(&mut buf);
        mono.extend(buf.chunks(2).map(|f| f[0]));
    }
    mono.truncate(frames);

    // Running mean of |x| over 30 ms: longer than a cycle of the lowest note.
    let window = (0.030 * dsp::SR) as usize;
    let mut envelope = Vec::with_capacity(mono.len());
    let mut sum = 0.0;
    for (i, &s) in mono.iter().enumerate() {
        sum += s.abs();
        if i >= window {
            sum -= mono[i - window].abs();
        }
        envelope.push(sum / window as f32);
    }

    let peak = envelope.iter().fold(0.0f32, |m, &e| m.max(e));
    let (high, low) = (0.3 * peak, 0.2 * peak);
    let mut hits = Vec::new();
    // If the window opens mid-note there is no onset to find: we did not see it
    // begin. Starting `above` at false would report the tail of the previous
    // kick as a hit at sample 0, and drag the measured gap down.
    let mut above = envelope.first().is_some_and(|&e| e > low);
    for (i, &e) in envelope.iter().enumerate() {
        if !above && e > high {
            hits.push(i);
            above = true;
        } else if above && e < low {
            above = false;
        }
    }
    hits
}

#[test]
fn a_tempo_change_means_from_here_on() {
    // Four seconds at 126, then four at 63. Without the anchor in Clock, the
    // second half either goes silent (the next step's absolute position
    // doubles) or fires a burst of catch-up steps. With it, the beat simply
    // gets slower.
    let mut engine = Engine::new(dsp::SR, 126.0, engine::sets::rolling());
    // The kick alone: with hats and bass in the mix the onset finder counts
    // sixteenths, which is a measurement of the pattern, not of the tempo.
    engine.solo("kick");
    let half = (4.0 * dsp::SR) as usize;
    let first = onsets(&mut engine, half);
    engine.set_bpm(63.0);
    let second = onsets(&mut engine, half);

    // The median gap, not the mean: one odd interval at a window edge should
    // not move the answer.
    let beat = |hits: &[usize]| -> f32 {
        let mut gaps: Vec<f32> = hits.windows(2).map(|w| (w[1] - w[0]) as f32).collect();
        assert!(!gaps.is_empty(), "no beats found at all");
        gaps.sort_by(f32::total_cmp);
        gaps[gaps.len() / 2] / dsp::SR
    };
    let fast = beat(&first);
    let slow = beat(&second);
    assert!(
        (0.46..0.49).contains(&fast),
        "126 BPM should beat every 0.476 s, got {fast}"
    );
    assert!(
        (0.93..0.98).contains(&slow),
        "63 BPM should beat every 0.952 s, got {slow}"
    );
    // The give-away failure: a burst of catch-up steps, or nothing at all.
    assert!(
        second.len() >= 3,
        "only {} beats in four seconds after the tempo change",
        second.len()
    );
}

#[test]
fn the_state_stays_sane_across_a_tempo_change() {
    let mut engine = Engine::new(dsp::SR, 126.0, engine::sets::rolling());
    let mut buf = vec![0.0f32; 256];
    for _ in 0..200 {
        engine.process(&mut buf);
    }
    let before = engine.state();
    engine.set_bpm(63.0);
    let after = engine.state();
    // Halving the tempo must not move where we already are in the music.
    assert!(
        (after.bar - before.bar).abs() < 1e-6,
        "bar jumped from {} to {} on a tempo change",
        before.bar,
        after.bar
    );
    assert_eq!(after.dropped, 0, "voices were dropped");
}

#[test]
fn the_duck_dips_six_db_and_swells_back() {
    let mut duck = Duck::new(dsp::SR, 6.0, 0.005, 0.2);
    assert!((duck.tick() - 1.0).abs() < 0.01, "should start open");

    duck.trigger();
    let mut lowest = 1.0f32;
    for _ in 0..(0.02 * dsp::SR) as usize {
        lowest = lowest.min(duck.tick());
    }
    let db = 20.0 * lowest.log10();
    assert!(
        (-6.5..-5.5).contains(&db),
        "wanted about -6 dB of duck, got {db:.2} dB"
    );

    // 200 ms is the release time constant, so most of the way back by then and
    // essentially all the way back by three of them. This swell is the pump.
    let mut level = lowest;
    for _ in 0..(0.2 * dsp::SR) as usize {
        level = duck.tick();
    }
    assert!(
        level > 0.75,
        "after 200 ms the duck was still at {level:.3}"
    );
    // Three time constants from -6 dB is 1 - 0.5*exp(-3) = 0.975, so 0.97 is
    // the ceiling here, not 0.99. A one-pole never quite arrives.
    for _ in 0..(0.4 * dsp::SR) as usize {
        level = duck.tick();
    }
    assert!(
        level > 0.97,
        "after 600 ms the duck was still at {level:.3}"
    );
}
