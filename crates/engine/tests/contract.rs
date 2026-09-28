//! The contract between the audio thread and whatever is driving it.
//!
//! These are the properties that make the decoupling real rather than nominal.
//! If one of them breaks, the design has quietly stopped being what it claims.

use engine::telemetry::{Command, Telemetry};

#[test]
fn what_crosses_the_queues_is_copy_and_small() {
    // Copy, because anything with a destructor or a heap pointer would mean the
    // audio thread allocating or freeing to send a frame.
    fn assert_copy<T: Copy>() {}
    assert_copy::<Telemetry>();
    assert_copy::<Command>();

    // Small, because the queue holds several and the audio thread writes one
    // sixty times a second. A Vec sneaking in here would not fail to compile —
    // it would fail to be real-time — so the size is pinned.
    let telemetry = std::mem::size_of::<Telemetry>();
    let command = std::mem::size_of::<Command>();
    println!("telemetry frame {telemetry} bytes, command {command} bytes");
    assert!(
        telemetry < 1024,
        "a telemetry frame has grown to {telemetry} bytes"
    );
    assert!(command <= 16, "a command has grown to {command} bytes");
}

#[test]
fn the_engine_needs_no_audio_device_and_no_window() {
    // The whole point: this test runs on a machine with no sound card, which is
    // also how the offline renderer works. If the engine ever needed a device,
    // this would not compile.
    let mut engine = engine::Engine::new(dsp::SR, 126.0, engine::sets::rolling());
    let mut out = vec![0.0f32; 512];
    engine.process(&mut out);
    assert!(out.iter().any(|s| *s != 0.0), "no sound without a device");
    assert!(engine.telemetry().lane_count > 0);
}

#[test]
fn every_command_survives_the_wire() {
    // The browser sends JSON and serde turns it back into this enum with no
    // hand-written parsing between them, so the shapes have to actually match.
    // This test exists because they did not: serde's internally tagged
    // representation cannot carry a bare primitive, so `Bpm(f32)` compiled
    // happily and would have failed at run time on the first fader move.
    let messages = [
        r#"{"t":"mute","index":2,"muted":true}"#,
        r#"{"t":"level","index":0,"gain":0.8}"#,
        r#"{"t":"macro","index":1,"value":0.5}"#,
        r#"{"t":"macro","index":1,"value":null}"#,
        r#"{"t":"bpm","value":130.0}"#,
        r#"{"t":"master","value":0.6}"#,
        r#"{"t":"seek","bar":24.0}"#,
        r#"{"t":"playing","value":false}"#,
    ];
    for message in messages {
        let command: Command =
            serde_json::from_str(message).unwrap_or_else(|err| panic!("{message}: {err}"));
        // And it has to mean something to the engine, not merely parse.
        let mut engine = engine::Engine::new(dsp::SR, 126.0, engine::sets::intro());
        engine.apply(command);
    }

    // A message the page should not be able to send silently does nothing.
    assert!(serde_json::from_str::<Command>(r#"{"t":"explode"}"#).is_err());
}

#[test]
fn a_telemetry_frame_and_a_description_both_serialise() {
    let mut engine = engine::Engine::new(dsp::SR, 126.0, engine::sets::intro());
    let mut out = vec![0.0f32; 512];
    engine.process(&mut out);

    let telemetry = serde_json::to_string(&engine.telemetry()).expect("telemetry");
    assert!(telemetry.contains("\"bar\""), "{telemetry}");

    let description = serde_json::to_string(&engine.describe()).expect("description");
    // The things a timeline cannot be drawn without.
    for expected in [
        "\"lanes\"",
        "\"spans\"",
        "\"steps\"",
        "\"curve\"",
        "\"fly\"",
    ] {
        assert!(
            description.contains(expected),
            "the description has no {expected}"
        );
    }
    println!(
        "telemetry {} bytes of JSON, description {} bytes",
        telemetry.len(),
        description.len()
    );
}

#[test]
fn editing_a_step_changes_what_plays() {
    // The drum machine's whole loop: a cell is clicked, one step changes, and
    // the next time round the bar it is heard. Nothing is allocated to do it —
    // the slot already exists — which is why an editor sends one step rather
    // than a whole clip.
    let mut engine = engine::Engine::new(dsp::SR, 126.0, engine::sets::rolling());
    let kick = engine
        .describe()
        .lanes
        .iter()
        .position(|lane| lane.name == "kick")
        .expect("rolling has a kick") as u8;

    // Rolling's kick is four on the floor: steps 0, 4, 8 and 12.
    let before = engine.describe().lanes[kick as usize].steps.clone();
    assert_eq!(before[5].0, 0.0, "step 5 should start empty");

    let command: Command = serde_json::from_str(&format!(
        r#"{{"t":"set_step","lane":{kick},"step":5,"velocity":0.7,"offset":0}}"#
    ))
    .expect("the page's own JSON");
    engine.apply(command);

    let after = engine.describe().lanes[kick as usize].steps.clone();
    assert_eq!(after[5].0, 0.7, "the step did not take");
    assert_eq!(after[0].0, before[0].0, "the other steps moved");

    // And it is audible: render the bar and count the kicks.
    let mut out = vec![0.0f32; (4.0 * 60.0 / 126.0 * 4.0 * f64::from(dsp::SR)) as usize * 2];
    engine.apply(Command::Seek { bar: 0.0 });
    engine.process(&mut out);
    let onsets = out
        .chunks(2)
        .map(|frame| frame[0].abs())
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| w[0] < 0.2 && w[1] >= 0.2)
        .count();
    assert!(
        onsets >= 5,
        "expected five kicks in the bar, heard {onsets}"
    );
}

#[test]
fn a_piece_survives_being_written_down_and_read_back() {
    // The claim the whole persistence design rests on: the engine's `Set` *is*
    // the document, so writing one out and reading it back must give a piece
    // that plays identically. Not approximately — the same samples.
    for name in engine::sets::NAMES {
        let original = engine::sets::by_name(name).expect("a built-in piece");

        let json = serde_json::to_string(&original).expect("a piece serialises");
        let read_back: engine::seq::Set = serde_json::from_str(&json)
            .unwrap_or_else(|err| panic!("{name} did not survive the round trip: {err}"));

        let render = |set: engine::seq::Set| {
            let mut engine = engine::Engine::new(dsp::SR, set.bpm, set);
            let mut out = vec![0.0f32; 48_000 * 2 * 4];
            engine.process(&mut out);
            out
        };
        let before = render(original);
        let after = render(read_back);
        let differences = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        assert_eq!(
            differences, 0,
            "{name} played differently after a round trip through JSON"
        );
    }
}
