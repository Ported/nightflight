//! The timeline's waveforms, rendered from the piece itself.
//!
//! The page wants each clip's row to look like what it sounds like, and the
//! only honest way to know that is to play it: one offline pass of the
//! document through the same `process()` the stream calls, reading each
//! lane's peak after every sixteenth of a bar. A lane's peak is already in
//! the telemetry — the meters needed it first — so the render costs one
//! engine and no new tap into the audio path.
//!
//! Lanes fold into their clip by maximum rather than by sum: a peak is not
//! additive, and max is how a drum bus reads — the kick's spikes with the
//! hat showing between them.
//!
//! Each clip is then normalised to its own loudest moment. Absolute level is
//! the mixer's business and the meters already show it; the row's business is
//! *shape*, and at absolute scale a hat row under a kick row is a flat line.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use engine::Engine;
use engine::seq::Set;

use crate::Session;

/// Slices per bar: one per sixteenth, the grid the music is written on.
const PER_BAR: usize = 16;

/// Keep rendering whatever document the session holds, whenever it changes.
///
/// A worker that polls, like the file watcher, rather than a render kicked
/// from every site that replaces the document — those sites hold the session
/// lock, and a render takes seconds.
pub fn spawn(session: Arc<Mutex<Session>>) {
    thread::spawn(move || {
        // What was last rendered. The name catches `load`, the generation
        // catches the watcher, and `performed` catches the conductor — a
        // brought-in clip changes what the document plays without any file
        // changing.
        let mut rendered: Option<(String, u64, u64)> = None;
        loop {
            let next = {
                let session = session.lock().expect("no panics hold this");
                let key = (session.name.clone(), session.generation, session.performed);
                (rendered.as_ref() != Some(&key)).then(|| (key, session.document.clone()))
            };
            let Some((key, set)) = next else {
                thread::sleep(Duration::from_millis(250));
                continue;
            };
            let message = render(set);
            {
                let mut session = session.lock().expect("no panics hold this");
                // Publish only if this is still the document on stage; a
                // reload mid-render means going round again, not showing a
                // picture of the old piece.
                if (session.name.as_str(), session.generation, session.performed)
                    == (key.0.as_str(), key.1, key.2)
                {
                    session.waves = Some(message);
                    session.waves_generation += 1;
                }
            }
            rendered = Some(key);
        }
    });
}

/// One pass over the whole piece, peaks folded into clips, as the message to
/// send. Returns the serialised JSON so the lock is held for a clone, not for
/// serialisation.
fn render(set: Set) -> String {
    // Clips in first-appearance order, each with its document lane indices —
    // the same order the page derives, so the rows match.
    let mut clips: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, lane) in set.lanes.iter().enumerate() {
        match clips.iter_mut().find(|(name, _)| *name == lane.clip) {
            Some((_, lanes)) => lanes.push(index),
            None => clips.push((lane.clip.clone(), vec![index])),
        }
    }

    let bars = set.length_bars.max(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let slices = (bars * PER_BAR as f32).ceil() as usize;
    // A sixteenth of a bar, in samples: a bar is four beats.
    let per_slice = f64::from(dsp::SR) * 15.0 / f64::from(set.bpm);

    let mut engine = Engine::new(dsp::SR, set.bpm, set);
    let mut peaks: Vec<Vec<f32>> = vec![Vec::with_capacity(slices); clips.len()];
    let mut block = vec![0.0f32; 1024 * 2];
    let mut done: u64 = 0;
    for slice in 1..=slices {
        // Boundaries land on the rounded running total, so the error in a
        // fractional slice length never accumulates.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let until = (per_slice * slice as f64).round() as u64;
        while done < until {
            #[allow(clippy::cast_possible_truncation)]
            let frames = ((until - done) as usize).min(1024);
            let out = &mut block[..frames * 2];
            out.fill(0.0);
            engine.process(out);
            done += frames as u64;
        }
        // `level` is the peak since the last telemetry frame, and reading it
        // resets it — exactly one slice's worth, by construction.
        let frame = engine.telemetry();
        for (clip, (_, lanes)) in clips.iter().enumerate() {
            let peak = lanes
                .iter()
                .filter_map(|&lane| frame.lanes.get(lane))
                .map(|state| state.level)
                .fold(0.0_f32, f32::max);
            peaks[clip].push(peak);
        }
    }

    let mut out = serde_json::Map::new();
    for ((name, _), peaks) in clips.into_iter().zip(peaks) {
        let loudest = peaks.iter().copied().fold(0.0_f32, f32::max).max(1e-6);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let quantised: Vec<u8> = peaks
            .iter()
            .map(|peak| (peak / loudest * 255.0).round() as u8)
            .collect();
        out.insert(name, quantised.into());
    }
    serde_json::json!({ "t": "waves", "per_bar": PER_BAR, "clips": out }).to_string()
}
