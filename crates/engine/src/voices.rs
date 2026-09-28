//! The voice pool: fixed size, so playing a note never allocates.
//!
//! Two things worth knowing about the shape of this. `AnyVoice` is an enum
//! rather than a `Box<dyn Voice>` because a box is a heap allocation, and the
//! audio thread is not allowed one — an enum is the same polymorphism with the
//! size known up front. And a finished voice is cleared by writing `None` over
//! it, which frees nothing, because there was never anything on the heap.

use dsp::Voice;
use dsp::inst::{Bass, Glass, Hat, Kick, Strings};

/// Enough for a busy bar with long tails. Raising it costs memory, not time.
pub const MAX_VOICES: usize = 48;

pub enum AnyVoice {
    Kick(Kick),
    Hat(Hat),
    Bass(Bass),
    Glass(Glass),
    Strings(Strings),
}

impl Voice for AnyVoice {
    fn add(&mut self, out: &mut [f32]) {
        match self {
            Self::Kick(v) => v.add(out),
            Self::Hat(v) => v.add(out),
            Self::Bass(v) => v.add(out),
            Self::Glass(v) => v.add(out),
            Self::Strings(v) => v.add(out),
        }
    }

    fn finished(&self) -> bool {
        match self {
            Self::Kick(v) => v.finished(),
            Self::Hat(v) => v.finished(),
            Self::Bass(v) => v.finished(),
            Self::Glass(v) => v.finished(),
            Self::Strings(v) => v.finished(),
        }
    }
}

struct Slot {
    part: usize,
    voice: AnyVoice,
}

pub struct Pool {
    slots: [Option<Slot>; MAX_VOICES],
    /// Voices dropped because every slot was busy. Shown in the UI: if it
    /// moves, MAX_VOICES is too low.
    pub dropped: u32,
}

impl Default for Pool {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            dropped: 0,
        }
    }
}

impl Pool {
    /// Start a voice for `part` in the first free slot, or count a drop.
    pub fn start(&mut self, part: usize, voice: AnyVoice) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.is_none()) {
            *slot = Some(Slot { part, voice });
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    #[must_use]
    pub fn playing(&self, part: usize) -> bool {
        self.slots
            .iter()
            .any(|s| s.as_ref().is_some_and(|s| s.part == part))
    }

    /// Mix one part's live voices into `out`, retiring the ones that rang out.
    pub fn add_part(&mut self, part: usize, out: &mut [f32]) {
        for slot in &mut self.slots {
            if let Some(s) = slot {
                if s.part != part {
                    continue;
                }
                s.voice.add(out);
                if s.voice.finished() {
                    *slot = None;
                }
            }
        }
    }

    /// Stop everything. Used when the transport jumps: a pad note with a
    /// four-second release would otherwise ring on over the new position.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
    }

    #[must_use]
    pub fn active(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}
