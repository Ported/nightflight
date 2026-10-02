//! The timeline's waveforms, rendered from the piece itself.
//!
//! The page wants each clip's row to look like what it sounds like, and the
//! only honest way to know that is to play it: one offline pass of the
//! document through the same `process()` the stream calls, reading each
//! lane's peak after every sixteenth of a bar. A lane's peak is already in
//! the telemetry — the meters needed it first — so the render costs one
//! engine and no new tap into the audio path.
//!
//! Every lane's own shape is kept and sent, for rows the page has unfolded.
//! For the clip's single row, lanes fold together by maximum rather than by
//! sum: a peak is not additive, and max is how a drum bus reads — the kick's
//! spikes with the hat showing between them.
//!
//! Clip and lane alike are normalised to their own loudest moment. Absolute
//! level is the mixer's business and the meters already show it; a row's
//! business is *shape*, and at absolute scale a hat row under a kick row is
//! a flat line.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use engine::Engine;
use engine::seq::Set;

use crate::Session;

/// Slices per bar. A 64th of a bar is ~29ms at 128 BPM — sharp enough that a
/// kick's transient still reads when the page is zoomed to a couple of bars.
/// The page decimates to pixel columns when zoomed out, so the extra slices
/// cost bytes on the wire, not rectangles on the screen.
const PER_BAR: usize = 64;

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
    let names: Vec<String> = set.lanes.iter().map(|lane| lane.name.clone()).collect();

    let bars = set.length_bars.max(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let slices = (bars * PER_BAR as f32).ceil() as usize;
    // One slice, in samples: a bar is four beats, 240/bpm seconds. Derived
    // from PER_BAR — a constant here already lied once, as 15.0, which is a
    // sixteenth of a bar and quietly broke the day PER_BAR stopped being 16.
    #[allow(clippy::cast_precision_loss)]
    let per_slice = f64::from(dsp::SR) * 240.0 / (f64::from(set.bpm) * PER_BAR as f64);

    let mut engine = Engine::new(dsp::SR, set.bpm, set);
    let mut peaks: Vec<Vec<f32>> = vec![Vec::with_capacity(slices); names.len()];
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
        for (lane, peaks) in peaks.iter_mut().enumerate() {
            peaks.push(frame.lanes.get(lane).map_or(0.0, |state| state.level));
        }
    }

    // Each lane its own shape; each clip the max of its lanes, slice by
    // slice — the single row an unfolded clip still keeps at its head.
    let mut lane_rows = serde_json::Map::new();
    for (name, peaks) in names.iter().zip(&peaks) {
        lane_rows.insert(name.clone(), quantised(peaks).into());
    }
    let mut clip_rows = serde_json::Map::new();
    for (name, lanes) in clips {
        let folded: Vec<f32> = (0..slices)
            .map(|s| lanes.iter().map(|&l| peaks[l][s]).fold(0.0_f32, f32::max))
            .collect();
        clip_rows.insert(name, quantised(&folded).into());
    }
    serde_json::json!({
        "t": "waves", "per_bar": PER_BAR, "clips": clip_rows, "lanes": lane_rows,
    })
    .to_string()
}

/// Normalised to its own loudest moment and packed into bytes.
fn quantised(peaks: &[f32]) -> Vec<u8> {
    let loudest = peaks.iter().copied().fold(0.0_f32, f32::max).max(1e-6);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    peaks
        .iter()
        .map(|peak| (peak / loudest * 255.0).round() as u8)
        .collect()
}
