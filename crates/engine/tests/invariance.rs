//! The engine must not care how the audio device carves up time.
//!
//! This is the most valuable test in the project: it catches every bug where
//! state is reset, double-advanced or leaked across a block boundary, and it is
//! what makes an offline render of a jam trustworthy — the render uses
//! different block sizes from the live stream, and must sound the same.

use engine::Engine;

/// Render `frames` samples through blocks of `block` frames.
fn render(block: usize, frames: usize) -> Vec<f32> {
    let mut engine = Engine::new(dsp::SR, 126.0, engine::sets::rolling());
    let mut out = Vec::with_capacity(frames * 2);
    let mut buf = vec![0.0f32; block * 2];
    while out.len() < frames * 2 {
        engine.process(&mut buf);
        out.extend_from_slice(&buf);
    }
    out.truncate(frames * 2);
    out
}

#[test]
fn block_size_does_not_change_the_output() {
    let frames = 48_000 * 2;
    let reference = render(512, frames);
    for block in [1, 7, 64, 128, 137, 480, 1024] {
        let other = render(block, frames);
        let differences = reference.iter().zip(&other).filter(|(a, b)| a != b).count();
        assert_eq!(differences, 0, "block size {block} changed the output");
    }
}

#[test]
fn nothing_clicks_and_nothing_is_infinite() {
    let out = render(128, 48_000 * 4);
    assert!(out.iter().all(|s| s.is_finite()), "non-finite sample");

    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "silence: peak {peak}");
    assert!(peak <= 1.0, "clipping: peak {peak}");

    // Clicks are tested where they can be attributed: at a voice's own
    // boundaries, in `dsp/tests/kick.rs`. Across a mix of noise transients the
    // largest sample-to-sample jump says nothing — a beater click *is* a jump.
}
