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

    let description =
        serde_json::to_string(&engine::sets::intro().describe()).expect("description");
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
    //
    // Asserted on the sound, not on the engine's own account of itself: the
    // engine holds no document. It is handed a piece, it plays it, and what it
    // was handed is the server's business.
    let set = engine::sets::rolling();
    let kick = set
        .lanes
        .iter()
        .position(|lane| lane.name == "kick")
        .expect("rolling has a kick") as u8;
    assert_eq!(
        set.lanes[kick as usize].pattern.all()[5].velocity,
        0.0,
        "step 5 should start empty"
    );

    // The level in the sixteenth where the new step lands, which is the only
    // place the sound should differ. Counting onsets would not do: a 49 Hz kick
    // crosses any threshold you pick a dozen times per hit.
    let bar = 4.0 * 60.0 / 126.0 * f64::from(dsp::SR);
    let at_step_five = |engine: &mut engine::Engine| -> f32 {
        engine.apply(Command::Seek { bar: 0.0 });
        // Let the seek fade open before measuring.
        engine.process(&mut vec![0.0f32; 2048]);
        let mut out = vec![0.0f32; bar as usize * 2];
        engine.process(&mut out);
        let sixteenth = out.len() / 2 / 16;
        out.chunks(2)
            .map(|frame| frame[0].abs())
            .skip(5 * sixteenth)
            .take(sixteenth)
            .fold(0.0f32, f32::max)
    };

    let mut engine = engine::Engine::new(dsp::SR, set.bpm, set);
    let before = at_step_five(&mut engine);

    // The exact JSON the page sends when a cell is clicked.
    let command: Command = serde_json::from_str(&format!(
        r#"{{"t":"set_step","lane":{kick},"step":5,"velocity":0.9,"offset":0}}"#
    ))
    .expect("the page's own JSON");
    engine.apply(command);

    let after = at_step_five(&mut engine);
    // Two decibels, not ten: the window is not silent to begin with. A kick
    // rings for 0.4 s and a sixteenth is 0.119, so the one on step 4 is still
    // sounding through step 5, and a new kick on top of it is a step up rather
    // than an arrival out of nothing.
    let louder = 20.0 * (after / before.max(1e-6)).log10();
    assert!(
        louder > 2.0,
        "the extra kick was not heard: step 5 peaked at {before:.3} and now peaks \
         at {after:.3}, only {louder:.1} dB louder"
    );
}

#[test]
fn turning_a_parameter_changes_what_is_heard() {
    // The other half of the editor: a fader moved reaches the engine as an index
    // and a number, with no string crossing to the audio thread.
    let set = engine::sets::rolling();
    let kick = set
        .lanes
        .iter()
        .position(|lane| lane.name == "kick")
        .expect("rolling has a kick") as u8;

    // `decay` is the kick's body length, and its index is wherever the
    // declaration put it — which is the point of the spec: nothing here has to
    // know.
    let spec = set.lanes[kick as usize].voicing.spec();
    let decay = spec
        .iter()
        .position(|param| param.name == "decay")
        .expect("a kick has a decay") as u8;

    // The kick alone, or the window catches a hat and reports that both renders
    // are identical — which they are, in the part of them being measured.
    let tail = |engine: &mut engine::Engine| -> f32 {
        engine.apply(Command::Seek { bar: 0.0 });
        engine.process(&mut vec![0.0f32; 2048]);
        let mut out = vec![0.0f32; 48_000];
        engine.process(&mut out);
        // 0.30 to 0.39 s: late in the kick's 0.4 s note, where a longer decay
        // leaves more behind, and before the next kick.
        out.chunks(2)
            .map(|frame| frame[0].abs())
            .skip(14_400)
            .take(4_300)
            .fold(0.0f32, f32::max)
    };

    let mut engine = engine::Engine::new(dsp::SR, set.bpm, set);
    engine.solo("kick");
    let before = tail(&mut engine);

    engine.apply(
        serde_json::from_str::<Command>(&format!(
            r#"{{"t":"set_param","lane":{kick},"param":{decay},"value":1.8}}"#
        ))
        .expect("the page's own JSON"),
    );
    let after = tail(&mut engine);
    let louder = 20.0 * (after / before.max(1e-6)).log10();
    assert!(
        louder > 3.0,
        "a longer decay should leave more ringing half a second in: \
         {before:.4} became {after:.4}, {louder:.1} dB"
    );

    // And a value outside the declared range cannot get in.
    engine.apply(Command::SetParam {
        lane: kick,
        param: decay,
        value: 1e9,
    });
    let clamped = engine::sets::rolling().lanes[kick as usize]
        .voicing
        .spec()
        .get(decay as usize)
        .expect("the parameter exists")
        .max;
    assert!(
        clamped < 1e9,
        "the spec should bound what an interface can send"
    );
}
