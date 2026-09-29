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

use crate::seq::{Home, Lane, Length, Pattern, ReverbSettings, Set, Voicing, STEPS_PER_BAR};
use crate::sets::ROOT;

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


/// One lane, alone, looping — the lane a patch is being edited through.
///
/// Editing a patch from inside a clip auditions it through the lane that plays
/// it, so you hear it at the notes, the length and the placement it will
/// actually have. That is more useful than a neutral test bench, and it is free:
/// the lane is already there.
#[must_use]
pub fn lane(set: &Set, index: usize) -> Option<(Set, Vec<usize>)> {
    let source = set.lanes.get(index)?;
    let mut one = source.clone();
    one.spans.clear();
    one.muted = false;

    let mut alone = Set {
        bpm: set.bpm,
        lanes: vec![one],
        reverb: set.reverb,
        macros: Vec::new(),
        length_bars: 1.0,
    };
    alone.length_bars = bars(&alone);
    Some((alone, vec![index]))
}

/// A patch with no lane behind it: one opened from the index, before anything in
/// the piece plays it.
///
/// The test line is per instrument and deliberately plain — four to the bar for
/// a drum, eighths for a hat, a held note for anything pitched. It exists to let
/// you hear the patch, not to be musical; the moment a patch is in a clip you
/// audition it through that lane instead.
#[must_use]
pub fn patch(voicing: Voicing, bpm: f32, room: Option<ReverbSettings>) -> Set {
    let (steps, length, root, send) = match voicing {
        Voicing::Kick(_) => ("X...X...X...X...", Length::Seconds(0.4), ROOT, 0.0),
        Voicing::Hat(_) => ("X.X.X.X.X.X.X.X.", Length::Seconds(0.1), 0.0, 0.15),
        Voicing::Bass(_) => ("X...X...X...X...", Length::Steps(3.5), ROOT, 0.1),
        // Long notes: one a bar, so its whole shape is audible before the next.
        Voicing::Glass(_) => ("X...............", Length::Steps(16.0), ROOT + 24.0, 0.5),
        Voicing::Strings(_) => ("X...............", Length::Steps(16.0), ROOT + 12.0, 0.4),
    };
    Set {
        bpm,
        lanes: vec![Lane {
            name: voicing.instrument().to_string(),
            clip: "audition".to_string(),
            voicing,
            patch: None,
            pattern: Pattern::grid(steps),
            gain: 1.0,
            length,
            root,
            home: Home::Centre,
            send,
            gate: None,
            spans: Vec::new(),
            velocity_scale: 1.0,
            ducked: false,
            muted: false,
        }],
        reverb: room,
        macros: Vec::new(),
        length_bars: 1.0,
    }
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
    fn growing_a_loop_adds_rests_and_shrinking_drops_the_tail() {
        let mut pattern = crate::seq::Pattern::grid("X...X...X...X...");
        let before: Vec<_> = pattern.all().iter().map(|s| (s.velocity, s.offset)).collect();
        let now = |p: &crate::seq::Pattern| -> Vec<(f32, i8)> {
            p.all().iter().map(|s| (s.velocity, s.offset)).collect()
        };

        pattern.resize(32);
        assert_eq!(pattern.all().len(), 32);
        assert_eq!(now(&pattern)[..16], before[..], "the first bar is untouched");
        assert!(
            pattern.all()[16..].iter().all(|step| step.velocity == 0.0),
            "a bar you just added should be empty"
        );

        pattern.resize(8);
        assert_eq!(pattern.all().len(), 8);
        assert_eq!(now(&pattern), before[..8]);
    }

    #[test]
    fn a_resized_clip_reports_its_new_length() {
        let mut piece = crate::sets::by_name("rolling").expect("rolling exists");
        let (before, _) = clip(&piece, "beat");
        assert_eq!(before.length_bars, 1.0);

        for lane in &mut piece.lanes {
            if lane.clip == "beat" {
                lane.pattern.resize(STEPS_PER_BAR * 2);
            }
        }
        let (after, _) = clip(&piece, "beat");
        assert_eq!(after.length_bars, 2.0, "the audition loops over the new length");
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
