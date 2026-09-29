//! A document, narrowed to the one thing being edited.
//!
//! Every editor in this tool needs the same thing: to hear what it is working on
//! *by itself*. A clip editor that auditioned by looping the transport over the
//! clip's bars would still play everything else the piece puts there — and worse,
//! a clip that appears nowhere in the piece yet could not be heard at all.
//!
//! So an audition is not a way of playing the piece. It is a small piece of its
//! own, built here from the material under the cursor, and handed to the engine
//! in place of the real one. Which is why the engine can be replaced while it
//! runs: that capability exists for this.
//!
//! Two things are deliberately left behind:
//!
//! - **Spans.** A lane with no spans always plays, so the clip starts at bar
//!   zero however late it arrives in the piece.
//! - **Macros.** Conducting is something done to a piece. While you are judging
//!   the sound of a clip it should hold still, at the parameters you can see on
//!   the faders, so what you hear is what you are about to save.
//!
//! The room stays, because how much of a lane goes to the reverb is part of how
//! that lane sounds, not part of the arrangement.

use crate::seq::{Set, STEPS_PER_BAR};

/// Bars this set's longest pattern occupies, at least one.
#[must_use]
fn bars(set: &Set) -> f32 {
    let steps = set
        .lanes
        .iter()
        .map(|lane| lane.pattern.all().len())
        .max()
        .unwrap_or(0);
    #[allow(clippy::cast_precision_loss)]
    let bars = (steps as f32 / STEPS_PER_BAR as f32).ceil();
    bars.max(1.0)
}

/// One clip, alone, looping.
///
/// Returns the set to play and the document lane index each of its lanes came
/// from, so whoever holds the document can still say which fader is which.
#[must_use]
pub fn clip(set: &Set, name: &str) -> (Set, Vec<usize>) {
    let indices: Vec<usize> = set
        .lanes
        .iter()
        .enumerate()
        .filter(|(_, lane)| lane.clip == name)
        .map(|(index, _)| index)
        .collect();

    let mut alone = Set {
        bpm: set.bpm,
        lanes: indices.iter().map(|&i| set.lanes[i].clone()).collect(),
        reverb: set.reverb,
        macros: Vec::new(),
        length_bars: 1.0,
    };
    for lane in &mut alone.lanes {
        lane.spans.clear();
        // Muting is a performance, and a performance does not follow you into an
        // editor: you came here to hear this.
        lane.muted = false;
    }
    alone.length_bars = bars(&alone);
    (alone, indices)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_plays_alone_from_the_top() {
        let piece = crate::sets::by_name("intro").expect("the intro exists");
        let name = piece
            .lanes
            .iter()
            .find(|lane| !lane.spans.is_empty())
            .map(|lane| lane.clip.clone())
            .expect("some lane arrives late");

        let (alone, from) = clip(&piece, &name);
        assert!(!alone.lanes.is_empty(), "the clip has lanes");
        assert_eq!(alone.lanes.len(), from.len());
        for (lane, &index) in alone.lanes.iter().zip(&from) {
            assert_eq!(lane.name, piece.lanes[index].name);
            assert_eq!(lane.clip, name);
            // No spans, so it sounds from bar zero however late it arrives in
            // the piece — which is the whole point.
            assert!(lane.spans.is_empty());
            assert!(!lane.muted);
        }
        // Nothing else comes along.
        assert!(alone.lanes.len() < piece.lanes.len());
        assert!(alone.macros.is_empty(), "conducting is a piece-level act");
        assert_eq!(alone.bpm, piece.bpm);
        assert!(alone.length_bars >= 1.0);
        assert!(alone.length_bars < piece.length_bars);
    }

    #[test]
    fn length_covers_the_longest_pattern() {
        let piece = crate::sets::by_name("rolling").expect("rolling exists");
        for name in piece.lanes.iter().map(|lane| lane.clip.clone()) {
            let (alone, _) = clip(&piece, &name);
            let longest = alone
                .lanes
                .iter()
                .map(|lane| lane.pattern.all().len())
                .max()
                .unwrap_or(0);
            let bars = alone.length_bars as usize * STEPS_PER_BAR;
            assert!(bars >= longest, "{name}: {bars} < {longest}");
            assert!(bars - longest < STEPS_PER_BAR, "{name}: a whole spare bar");
        }
    }
}
