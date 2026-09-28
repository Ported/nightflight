# The words

Six nouns, and everything in the code and the file formats uses them. They are
written down because a project that calls the same thing three names ends up with
three half-implementations of it.

```
piece      the whole arrangement: tracks, macros, a room, a tempo, a length
 └── track     one row on the timeline: a clip, placed, with spans
      └── clip      named, reusable material — what an editor makes and saves
           └── lane      one patch playing one line of steps
                ├── patch     an instrument with saved parameters
                └── steps     velocity and pitch offset, one per step
```

**piece** — a whole arrangement. What `sets.rs` builds today. `intro` is a piece.

**track** — a clip placed in a piece: when it plays, at what level. One row on
the timeline. The same clip can appear on two tracks, which is the point of
keeping them separate: material is reusable, a placement is not.

**clip** — named, reusable material, and the thing an editor edits. A drum
machine makes a clip with a lane per drum; a bass editor makes a clip with one
lane of pitches. A clip has a length in steps and knows nothing about when it
plays.

**lane** — one patch playing one line of steps, inside a clip. "kick" is a lane
of the "beat" clip. A lane also carries where it sits in space, its level, its
send to the room, and whether the kick ducks it — everything about *this* sound
that is not the notes.

**patch** — an instrument with a set of parameters, saved under a name. "punch"
is a patch of the kick instrument. Changing a patch changes every lane using it;
that is what makes it a patch rather than a copy.

**steps** — the notes. A velocity and a semitone offset per step, velocity 0
being a rest. Sixteen steps is a bar of sixteenths, but a lane's steps can be any
length, and one that does not divide the bar drifts against it — which is most
of what makes hypnotic music hypnotic.

## Words deliberately not used

**voice** means one note currently sounding, and nothing else. It is `dsp::Voice`
and it is what the voice pool counts. Bach's chord voices in the Prelude are a
third meaning and they keep their name only because that is what Bach called
them. A configured instrument is a **patch**.

**pattern** is avoided on its own, because a drum machine's pattern holds every
instrument while a sequencer's pattern holds one. Say **steps** for one lane's
notes and **clip** for the set of them.

**part** was the old word for a lane. It is gone.

**scene** belongs to the Python studio this grew out of, where it means a whole
rendered piece. Not used here.

## What exists today, and what does not

Lanes, steps, clips, pieces and macros are real. A lane names the clip it belongs
to, the timeline draws one row per clip, and each clip has an editor tab.

**Tracks are not yet.** Spans still live on the lane rather than on a placement,
so a clip cannot appear twice in a piece with different timing. Nothing needs it
yet and the shape is there when it does.

**Patches are only half real.** An instrument's parameters exist as a Rust struct
and can be set per lane, but they cannot be named, saved or shared between lanes,
which is what would make them patches rather than settings.

**Pieces are saved; edits are not yet.** A piece is a JSON file in `pieces/`,
the engine plays what it is given, and the built-in Rust generators are now only
provenance — `cargo run -p engine --example export` writes what they make. Hand
edit a piece file, restart, and you hear the change.

What is still missing is the loop back: an edit made in an editor reaches the
running engine but nothing writes it down, because the server does not yet hold
the document. When it does, a save is a file write and a reload shows what you
edited rather than what the piece started as.
