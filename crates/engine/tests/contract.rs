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
    assert!(engine.telemetry().part_count > 0);
}
