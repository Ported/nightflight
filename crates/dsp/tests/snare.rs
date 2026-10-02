//! What a snare must do, measured rather than listened to.

use dsp::inst::{Snare, snare};
use dsp::{SR, Voice};

/// MIDI 54 is about 185 Hz: where a snare's head actually sits.
const PITCH: f32 = 54.0;

fn render(p: snare::Params, length: f32) -> Vec<f32> {
    let mut voice = Snare::new(SR, PITCH, 1.0, length, p);
    let mut out = Vec::new();
    let mut block = [0.0f32; 64];
    for _ in 0..1000 {
        block.fill(0.0);
        voice.add(&mut block);
        out.extend_from_slice(&block);
        if voice.finished() {
            break;
        }
    }
    out
}

fn max_jump(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// Root-mean-square level between two times. Levels in a window, not onsets by
/// threshold: the project has been burned on the latter three times.
fn rms(x: &[f32], from: f32, to: f32) -> f32 {
    let window = &x[(from * SR) as usize..((to * SR) as usize).min(x.len())];
    (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt()
}

#[test]
fn it_starts_and_ends_on_a_ramp() {
    let out = render(snare::Params::default(), 0.25);
    assert_eq!(
        out.first().copied(),
        Some(0.0),
        "attack must start at silence"
    );
    // The gate's release is a raised cosine, so the tail arrives at zero.
    let tail = out[out.len() - 8..]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(tail < 1e-4, "release left {tail} hanging: that is a click");
}

#[test]
fn the_drum_alone_is_smooth_and_the_wires_are_not() {
    // With the wires off, what is left is two sines: the largest
    // sample-to-sample jump should be what two drifting sines can produce and
    // no more. The wires are highpassed noise, which is nothing but jumps —
    // so turning them on must raise that figure several times over. This
    // pins each layer to its role without caring about absolute levels.
    let drum_only = snare::Params {
        snap: 0.0,
        ..Default::default()
    };
    let smooth = max_jump(&render(drum_only, 0.25));
    let snappy = max_jump(&render(snare::Params::default(), 0.25));
    assert!(
        smooth < 0.12,
        "the drum alone jumped {smooth}: a sine pair should be smoother"
    );
    assert!(
        snappy > 3.0 * smooth,
        "wires on ({snappy}) vs off ({smooth}): the snap should dominate the transient"
    );
}

#[test]
fn the_wires_outlast_the_drum_when_asked() {
    // The whole point of separate decays: the body dies while the wires ring
    // on. Measured where the difference should be — a window well after the
    // hit — not at the hit, where both sound the same.
    //
    // The first version of this test used the default body decay and failed:
    // the drum's own tail is still at 8% of full level at 0.3 s, which is far
    // louder than dead wires, so both windows measured the drum and the ratio
    // came out at 3.3. The window was hearing the other layer — the same
    // mistake as measuring a kick through a sounding hat. Solo the lane: the
    // drum is turned down to its shortest decay so the window holds only wires.
    let long = snare::Params {
        decay: 0.03,
        snap_decay: 0.4,
        ..Default::default()
    };
    let short = snare::Params {
        snap_decay: 0.03,
        ..long
    };
    let ringing = rms(&render(long, 0.5), 0.25, 0.35);
    let dead = rms(&render(short, 0.5), 0.25, 0.35);
    assert!(
        ringing > 4.0 * dead,
        "snap_decay 0.4 left {ringing} in the tail window, 0.03 left {dead}: \
         the wires' decay did not reach the tail"
    );
}

#[test]
fn a_note_that_ends_early_is_choked() {
    let open = render(snare::Params::default(), 0.5);
    let choked = render(snare::Params::default(), 0.05);
    let open_tail = rms(&open, 0.1, 0.2);
    assert!(
        open_tail > 1e-3,
        "a 0.5 s note should still be sounding at 0.1 s, measured {open_tail}"
    );
    // The choked render has already finished; anything after the gate is zeros.
    let choked_tail = if choked.len() > (0.1 * SR) as usize {
        rms(&choked, 0.1, 0.2)
    } else {
        0.0
    };
    assert!(
        choked_tail < 1e-4,
        "a note ending at 0.05 s left {choked_tail} at 0.1 s: the choke is not choking"
    );
}

#[test]
fn sweep_starts_the_drum_higher() {
    // Pitch measured as zero crossings over the first 20 ms, on the drum alone:
    // crossing counting is safe here because a sine pair crosses cleanly,
    // where a full snare's noise would cross hundreds of times.
    let crossings = |p: snare::Params| {
        let out = render(p, 0.25);
        out[..(0.02 * SR) as usize]
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    };
    let still = snare::Params {
        snap: 0.0,
        sweep: 1.0,
        ..Default::default()
    };
    let swept = snare::Params {
        sweep: 4.0,
        ..still
    };
    let (still, swept) = (crossings(still), crossings(swept));
    assert!(
        swept as f32 > 1.5 * still as f32,
        "sweep 4 crossed {swept} times in 20 ms, sweep 1 crossed {still}: \
         the sweep is not raising the start"
    );
}
