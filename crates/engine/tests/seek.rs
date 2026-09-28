//! Scrubbing: moving the playhead while it plays.

use engine::Engine;
use engine::telemetry::Command;

const BAR_SAMPLES: f64 = 4.0 * 60.0 / 126.0 * 48_000.0;

fn engine() -> Engine {
    Engine::new(dsp::SR, 126.0, engine::sets::intro())
}

/// Render `frames` frames, returning interleaved stereo.
fn play(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    let mut buf = vec![0.0f32; 128 * 2];
    while out.len() < frames * 2 {
        engine.process(&mut buf);
        out.extend_from_slice(&buf);
    }
    out.truncate(frames * 2);
    out
}

#[test]
fn seeking_moves_the_transport() {
    let mut engine = engine();
    play(&mut engine, 4096);
    engine.apply(Command::Seek { bar: 20.0 });
    // The fade has to close before the jump, so give it a moment.
    play(&mut engine, 2048);
    let bar = engine.state().bar;
    assert!(
        (20.0..20.2).contains(&bar),
        "asked for bar 20, landed at {bar:.3}"
    );

    // And back to the start.
    engine.apply(Command::Seek { bar: 0.0 });
    play(&mut engine, 2048);
    assert!(engine.state().bar < 0.2, "did not return to the start");
}

#[test]
fn a_seek_does_not_click() {
    // The whole reason the engine fades around a jump. Without it, cutting from
    // one place in a piece to another is a step in the waveform, and a step is a
    // click — the loudest thing in the session, on headphones.
    let mut engine = engine();
    // Well into the arrival, where plenty is ringing.
    engine.apply(Command::Seek { bar: 24.0 });
    play(&mut engine, (2.0 * BAR_SAMPLES) as usize);

    engine.apply(Command::Seek { bar: 2.0 });
    let after = play(&mut engine, (0.5 * BAR_SAMPLES) as usize);

    let jump = after
        .chunks(2)
        .map(|f| f[0])
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        jump < 0.05,
        "the seek stepped by {jump:.4} in one sample: that is a click"
    );
    assert!(after.iter().all(|s| s.is_finite()));
}

#[test]
fn a_seek_stops_what_was_ringing() {
    // A pad note has a four-second release. Left alone it would ring straight
    // over the top of wherever the playhead went.
    let mut engine = engine();
    play(&mut engine, (8.0 * BAR_SAMPLES) as usize);
    let before = engine.state().voices;
    assert!(before > 0, "nothing was playing to begin with");

    engine.apply(Command::Seek { bar: 1.0 });
    // Long enough for the fade to close and the jump to happen, short enough
    // that few new notes have started.
    play(&mut engine, 1024);
    assert!(
        engine.state().voices < before,
        "{} voices survived the jump, from {before}",
        engine.state().voices
    );
}

#[test]
fn seeking_keeps_the_pattern_in_phase() {
    // Seek to the bar the beat lands on and the kick should be right there on
    // the downbeat, not wherever the step counter happened to be.
    let mut engine = engine();
    engine.apply(Command::Seek { bar: 26.0 });
    let out = play(&mut engine, (1.2 * BAR_SAMPLES) as usize);
    let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();

    // Onsets from a smoothed envelope, the same way the transport test finds
    // beats: a bare threshold counts one 49 Hz kick several times over.
    let window = (0.030 * f64::from(dsp::SR)) as usize;
    let mut envelope = Vec::with_capacity(left.len());
    let mut sum = 0.0f32;
    for (i, &s) in left.iter().enumerate() {
        sum += s.abs();
        if i >= window {
            sum -= left[i - window].abs();
        }
        envelope.push(sum / window as f32);
    }
    let peak = envelope.iter().fold(0.0f32, |m, &e| m.max(e));
    let (high, low) = (0.4 * peak, 0.25 * peak);
    let mut hits = Vec::new();
    let mut above = false;
    for (i, &e) in envelope.iter().enumerate() {
        if !above && e > high {
            hits.push(i);
            above = true;
        } else if above && e < low {
            above = false;
        }
    }

    assert!(
        hits.len() >= 4,
        "expected four kicks in a bar after seeking to the landing, found {}",
        hits.len()
    );
    // The first one lands inside the fade plus the envelope's own 30 ms window.
    let first = hits[0] as f64 / f64::from(dsp::SR);
    assert!(
        first < 0.05,
        "the downbeat arrived {:.0} ms after the seek, not on it",
        first * 1000.0
    );
    // And the spacing is a beat: 0.476 s at 126 BPM.
    let gap = (hits[2] - hits[1]) as f64 / f64::from(dsp::SR);
    assert!(
        (0.45..0.50).contains(&gap),
        "the kicks are {gap:.3} s apart, not a beat"
    );
}

#[test]
fn stopping_freezes_the_transport_but_lets_the_tail_ring() {
    let mut engine = engine();
    play(&mut engine, (8.0 * BAR_SAMPLES) as usize);
    let bar = engine.state().bar;

    engine.set_playing(false);
    let after = play(&mut engine, (0.5 * BAR_SAMPLES) as usize);

    // The clock has not moved.
    assert!(
        (engine.state().bar - bar).abs() < 1e-9,
        "the transport moved while stopped: {bar} -> {}",
        engine.state().bar
    );
    // But there is still sound: a four-second pad release and a seven-second
    // room do not stop because a button was pressed.
    let level = after.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(level > 1e-4, "stopping cut the tail off: peak {level:.6}");
    // And it did not click on the way, because nothing was interrupted.
    let jump = after
        .chunks(2)
        .map(|f| f[0])
        .collect::<Vec<_>>()
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(jump < 0.05, "stopping stepped by {jump:.4}");

    // Left stopped for long enough, it goes quiet on its own.
    let much_later = play(&mut engine, (12.0 * f64::from(dsp::SR)) as usize);
    let end = much_later[much_later.len() - 4096..]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(end < 1e-4, "still ringing twelve seconds later: {end:.6}");
}

#[test]
fn starting_again_carries_on_from_where_it_stopped() {
    let mut engine = engine();
    play(&mut engine, (4.0 * BAR_SAMPLES) as usize);
    engine.set_playing(false);
    let stopped_at = engine.state().bar;
    play(&mut engine, (2.0 * BAR_SAMPLES) as usize);

    engine.set_playing(true);
    play(&mut engine, (1.0 * BAR_SAMPLES) as usize);
    let now = engine.state().bar;
    assert!(
        (now - stopped_at - 1.0).abs() < 0.05,
        "one bar after restarting should be one bar on: {stopped_at:.3} -> {now:.3}"
    );
}

#[test]
fn the_playhead_can_be_moved_while_stopped() {
    // The useful combination: stop, scrub to the bar you want to hear, play.
    let mut engine = engine();
    engine.set_playing(false);
    engine.apply(Command::Seek { bar: 24.0 });
    play(&mut engine, 4096);
    let bar = engine.state().bar;
    assert!(
        (24.0..24.05).contains(&bar),
        "seeking while stopped landed at {bar:.3}"
    );

    engine.set_playing(true);
    play(&mut engine, (0.5 * BAR_SAMPLES) as usize);
    assert!(engine.state().bar > 24.4, "it did not start from there");
}
