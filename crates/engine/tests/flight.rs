//! The helicopter: the spiral, the early firing, and the breath.

use engine::Engine;
use engine::automation::Flight;

#[test]
fn it_starts_far_away_and_lands_on_its_home() {
    let flight = Flight::default();
    let far = flight.offset(0.0);
    assert!(
        (far.distance() - (40.0f32.powi(2) + 15.0f32.powi(2)).sqrt()).abs() < 0.5,
        "40 m out and 15 m up is 42.7 m away, not {}",
        far.distance()
    );
    // Behind and to the left, which is where 225 degrees clockwise from ahead is.
    assert!(
        far.x < 0.0 && far.z > 0.0,
        "225 degrees should be behind-left"
    );

    let landed = flight.offset(1.0);
    assert!(
        landed.distance() < 0.3,
        "it should land on its home, not {} m away",
        landed.distance()
    );
}

#[test]
fn it_covers_the_distance_early_and_slows_to_touch_down() {
    // Squared, not linear: halfway through the approach it is already three
    // quarters of the way in. A linear approach reads as a machine on rails.
    let flight = Flight::default();
    let half = flight.offset(0.5);
    assert!(
        (half.x * half.x + half.z * half.z).sqrt() < 0.3 * flight.distance,
        "halfway in it should have covered most of the ground"
    );
    // And it descends more slowly than it closes, so it comes in over the top.
    assert!(half.y > 0.2 * flight.height);
}

#[test]
fn the_travel_time_is_what_it_should_be() {
    // 40 m out and 15 m up is 42.7 m, which sound covers in 124 ms — a quarter
    // of a beat at 126 BPM. That is why flights fire early.
    let flight = Flight::default();
    let travel = Flight::travel(flight.offset(0.0));
    assert!(
        (0.12..0.13).contains(&travel),
        "wanted about 124 ms of travel, got {:.0} ms",
        travel * 1000.0
    );
    assert!(Flight::travel(flight.offset(1.0)) < 0.002);
}

/// Level of a window, in dB. Not K-weighted: this only compares like with like.
fn level(x: &[f32]) -> f32 {
    let power = x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32;
    10.0 * power.max(1e-20).log10()
}

/// Render the arrival up to `bars` and return interleaved stereo.
fn arrival(bars: f64) -> (Vec<f32>, Engine) {
    let mut engine = Engine::new(dsp::SR, 126.0, engine::sets::intro());
    let bar_samples = 4.0 * 60.0 / 126.0 * f64::from(dsp::SR);
    let mut out: Vec<f32> = Vec::new();
    let mut buf = vec![0.0f32; 256];
    while (out.len() / 2) < (bars * bar_samples) as usize {
        engine.process(&mut buf);
        out.extend_from_slice(&buf);
    }
    (out, engine)
}

const BAR_SAMPLES: f64 = 4.0 * 60.0 / 126.0 * 48_000.0;

#[test]
fn the_beat_arrives_and_the_landing_is_a_step_up() {
    let (out, engine) = arrival(31.0);
    let at = |from: f64, to: f64| -> &[f32] {
        let (a, b) = (
            (from * BAR_SAMPLES) as usize * 2,
            (to * BAR_SAMPLES) as usize * 2,
        );
        &out[a..b.min(out.len())]
    };

    // The intro is playing throughout, so this is relative: the approach has to
    // grow, and the landing has to be a clear step above it.
    let early = level(at(11.0, 15.0));
    let late = level(at(22.0, 26.0));
    let landed = level(at(27.0, 30.0));
    assert!(
        late > early + 1.0,
        "the approach did not grow: {early:.1} dB then {late:.1} dB"
    );
    assert!(
        landed > late + 2.0,
        "the landing should be a clear step up: {late:.1} then {landed:.1} dB"
    );
    assert_eq!(engine.state().dropped, 0, "voices were dropped");
}

#[test]
fn the_breath_is_a_hole_and_not_silence() {
    let (out, _) = arrival(27.0);
    let beat = BAR_SAMPLES / 4.0;
    let window = |from: f64, to: f64| -> &[f32] { &out[from as usize * 2..to as usize * 2] };
    let landing = 26.0 * BAR_SAMPLES;
    let breath = level(window(landing - beat, landing));
    let before = level(window(landing - 2.0 * beat, landing - beat));

    assert!(
        breath < before - 1.0,
        "the breath should be a drop-out: {before:.1} dB then {breath:.1} dB"
    );
    // But not silence. The glass and the reverb ring through it, and that is
    // what makes the hole feel like held breath rather than a gap in the tape.
    assert!(
        breath > -50.0,
        "the breath went silent ({breath:.1} dB): the tails should carry it"
    );
}
