//! The trance gate: a rhythm cut into a lane's level.

use engine::mix::Gate;

const STEP: f64 = 48_000.0 * 60.0 / 126.0 / 4.0; // a sixteenth at 126 BPM

#[test]
fn it_opens_where_the_pattern_says_and_closes_where_it_does_not() {
    let gate = Gate::new("x.x.", 0.55, 1.0);
    let at = |step: f64, into: f64| gate.gain((step * STEP + into * STEP) as u64, STEP, 48_000.0);
    // A quarter of the way into an open step: past the 3 ms ramp, before the
    // 55% point, so fully open.
    assert!(at(0.0, 0.25) > 0.99, "an open step was not open");
    assert!(at(2.0, 0.25) > 0.99, "the pattern did not repeat");
    // A closed step is shut all the way, since depth is 1.
    assert!(at(1.0, 0.25) < 0.01, "a closed step was not closed");
    assert!(at(3.0, 0.5) < 0.01, "a closed step was not closed");
    // Past the open share, an open step has shut again.
    assert!(at(0.0, 0.9) < 0.01, "the step stayed open past its length");
}

#[test]
fn depth_decides_how_hard_it_chops() {
    for depth in [0.0f32, 0.25, 0.5, 1.0] {
        let gate = Gate::new("x.", 0.55, depth);
        let closed = gate.gain((1.3 * STEP) as u64, STEP, 48_000.0);
        assert!(
            (closed - (1.0 - depth)).abs() < 0.01,
            "at depth {depth} the closed step sat at {closed:.3}"
        );
    }
    // Zero depth has to be exactly transparent, or a build would start with a
    // step in the level.
    let off = Gate::new("x.x.", 0.55, 0.0);
    for i in 0..1000 {
        assert_eq!(off.gain(i * 37, STEP, 48_000.0), 1.0);
    }
}

#[test]
fn it_never_jumps() {
    // The chop itself must not click. Consecutive samples across several bars,
    // covering every edge in the pattern.
    let gate = Gate::new("x.xx.xx.x.xx.x.x", 0.55, 1.0);
    let mut previous = gate.gain(0, STEP, 48_000.0);
    let mut largest = 0.0f32;
    for sample in 1..(STEP * 64.0) as u64 {
        let now = gate.gain(sample, STEP, 48_000.0);
        largest = largest.max((now - previous).abs());
        previous = now;
    }
    // The fastest edge is the 3 ms attack, so one sample is 1/144 of the way.
    assert!(
        largest < 0.02,
        "the gate stepped by {largest:.4} in one sample"
    );
}

#[test]
fn it_stays_locked_to_the_grid_when_the_tempo_changes() {
    // The gate is a function of the transport position, so halving the tempo
    // halves every chop and keeps the pattern aligned.
    let gate = Gate::new("x.", 0.55, 1.0);
    let slow = STEP * 2.0;
    assert!(gate.gain((0.25 * slow) as u64, slow, 48_000.0) > 0.99);
    assert!(gate.gain((1.25 * slow) as u64, slow, 48_000.0) < 0.01);
}
