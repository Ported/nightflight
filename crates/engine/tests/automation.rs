//! Curves, spans and macros.

use engine::Engine;
use engine::automation::{Curve, Mapping, Play, Target, Transition};

#[test]
fn a_curve_interpolates_and_then_holds() {
    let curve = Curve::new(vec![(0.0, 0.1), (10.0, 0.5), (20.0, 0.2)]);
    assert!((curve.at(0.0) - 0.1).abs() < 1e-6);
    assert!((curve.at(5.0) - 0.3).abs() < 1e-6, "should be halfway");
    assert!((curve.at(10.0) - 0.5).abs() < 1e-6);
    assert!((curve.at(15.0) - 0.35).abs() < 1e-6);
    // Before the first point and after the last it holds, so a curve can never
    // be asked a question it has no answer to.
    assert!((curve.at(-100.0) - 0.1).abs() < 1e-6);
    assert!((curve.at(1e6) - 0.2).abs() < 1e-6);
}

#[test]
fn a_span_fades_in_and_out_and_is_silent_outside() {
    let play = Play {
        start: 4.0,
        end: 20.0,
        enter: Transition::Fade(4.0),
        leave: Transition::Fade(2.0),
    };
    assert_eq!(play.level(3.9), None, "before the span");
    assert_eq!(play.level(20.0), None, "the end is exclusive");
    assert!(
        play.level(4.0).unwrap() < 1e-6,
        "a fade starts from silence"
    );
    assert!((play.level(6.0).unwrap() - 0.5).abs() < 1e-6, "halfway in");
    assert!(
        (play.level(12.0).unwrap() - 1.0).abs() < 1e-6,
        "full in the middle"
    );
    assert!(
        (play.level(19.0).unwrap() - 0.5).abs() < 1e-6,
        "halfway out"
    );

    // A cut is a cut.
    let cut = Play::new(4.0, 8.0);
    assert!((cut.level(4.0).unwrap() - 1.0).abs() < 1e-6);
    assert_eq!(cut.level(8.0), None);
}

#[test]
fn a_mapping_shapes_its_range() {
    let linear = Mapping::new("x", Target::Level, 100.0, 900.0);
    assert!((linear.value(0.0) - 100.0).abs() < 1e-3);
    assert!((linear.value(0.5) - 500.0).abs() < 1e-3);
    assert!((linear.value(1.0) - 900.0).abs() < 1e-3);

    // Squared: most of the movement happens late, which is what a build wants.
    let steep = Mapping::new("x", Target::Level, 0.0, 16.0).shaped(2.0);
    assert!((steep.value(0.5) - 4.0).abs() < 1e-3);
    assert!((steep.value(0.25) - 1.0).abs() < 1e-3);
    // And it is clamped, so a fader cannot be pushed past its own range.
    assert!((steep.value(2.0) - 16.0).abs() < 1e-3);
}

/// Run the engine to a given bar and return it.
fn play_to(bar: f64) -> Engine {
    let mut engine = Engine::new(dsp::SR, 126.0, engine::sets::intro());
    let mut buf = vec![0.0f32; 256];
    while engine.state().bar < bar {
        engine.process(&mut buf);
    }
    engine
}

#[test]
fn the_macros_actually_move_the_parts() {
    // At the start the chop is off, the ring turns slowly, the pad is nearly
    // silent.
    let early = play_to(0.5);
    let start = early.debug_lanes();
    assert!(
        start.contains("pad 1: v0."),
        "the pad should start under 1.0: {start}"
    );
    assert!(
        start.contains("l7."),
        "the glass ring should still be slow: {start}"
    );

    // By the dominant pedal the chop is full and the ring turns once a bar.
    let late = play_to(23.0);
    let end = late.debug_lanes();
    assert!(
        end.contains("g1.00"),
        "the chop should be full by bar 22: {end}"
    );
    assert!(
        end.contains("pad 1: v4.") || end.contains("pad 1: v5."),
        "the pad should have swelled: {end}"
    );
    assert!(
        end.contains("l1."),
        "the ring should be at a lap a bar: {end}"
    );
}

#[test]
fn a_hand_on_a_fader_beats_the_score() {
    let mut engine = play_to(1.0);
    let automated = engine.macro_values();
    assert!(engine.set_macro("chop", Some(1.0)), "no chop macro");
    let mut buf = vec![0.0f32; 256];
    engine.process(&mut buf);
    assert!(
        engine.debug_lanes().contains("g1.00"),
        "a manual macro did not override the curve"
    );

    // Handing it back returns to the curve.
    assert!(engine.set_macro("chop", None));
    engine.process(&mut buf);
    let back = engine.macro_values();
    assert_eq!(
        automated
            .iter()
            .find(|(n, _)| *n == "chop")
            .map(|(_, v)| *v),
        back.iter().find(|(n, _)| *n == "chop").map(|(_, v)| *v),
        "releasing the fader should return to the curve"
    );
    assert!(!engine.set_macro("nonesuch", Some(0.5)));
}

#[test]
fn lanes_stop_starting_notes_when_their_span_ends() {
    // The intro's spans all end at bar 26. Past that nothing new may start, but
    // the reverb keeps ringing — and it should, or the piece would end on a
    // cliff.
    let mut engine = play_to(27.0);
    let before = engine.state().voices;
    let mut buf = vec![0.0f32; 256];
    for _ in 0..2000 {
        engine.process(&mut buf);
    }
    assert!(
        engine.state().voices <= before,
        "voices are still being started after the score ended"
    );
    assert_eq!(engine.state().dropped, 0);
}
