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

## Editing something in isolation

Every layer needs the same thing from the tool: to be **heard by itself**, and to
be **saved**. You judge a kick by hearing the kick, and a beat by hearing the
beat — not by hearing them inside four minutes of arrangement, and not only where
the piece happens to put them.

So an editor is not a view onto the piece. Opening one **replaces what the engine
is playing**: the server builds a small piece from the clip alone — its lanes,
from bar zero, looping, no spans, no macros — and hands it over. Going back to
the conductor hands the real piece back, at the bar it had reached.

Two things are deliberately left out of an audition. **Spans**, so a clip that
does not arrive until bar 26 is still audible the moment you open it. **Macros**,
because conducting is something done to a piece, and while you are judging a
sound it should hold still at the parameters the faders are showing.

This is why the engine can be replaced while it runs. It is one capability with
one purpose, and the same one that will later open a different piece, audition a
patch on a test pattern, and preview a clip from the index.

The wire stays in **document coordinates** throughout. The page says "lane 7" and
means the seventh lane of the piece, whether or not it is the seventh thing
currently sounding; the server translates in the two places that need it, and
telemetry comes back permuted into document order. A lane the audition is not
playing simply reports as silent. Editing one still changes the document — you
can turn a knob on something you cannot hear, and it is written down.

## What exists today, and what does not

Lanes, steps, clips, pieces and macros are real. A lane names the clip it belongs
to, the timeline draws one row per clip, and each clip has an editor tab.

**Tracks are not yet.** Spans still live on the lane rather than on a placement,
so a clip cannot appear twice in a piece with different timing. Nothing needs it
yet and the shape is there when it does.

Each clip editor plays its clip alone, looping, and leaving it puts the piece
back where it was.

**Patches are two thirds real.** Every parameter is declared once — field,
default, range, unit and scale together — so an interface can draw a fader for any
instrument without knowing which one it is looking at, and a turned fader is saved
with the piece. What is still missing is the *naming*: a patch cannot yet be saved
under its own name and shared between lanes, so changing it in one place changes
it in one place. Until then they are settings that travel with a lane.

**There is no library yet.** Patches and clips exist only inside the piece that
uses them, so a patch cannot be reused across pieces and a clip cannot be
auditioned before some piece contains it. The index of everything saved, and
*save as new* to fork one, come with that.

**Pieces are saved; edits are not yet.** A piece is a JSON file in `pieces/`,
the engine plays what it is given, and the built-in Rust generators are now only
provenance — `cargo run -p engine --example export` writes what they make. Hand
edit a piece file, restart, and you hear the change.

The loop is closed: the server holds the document, an edit is applied to it *and*
forwarded to the engine so it is heard at once, and a save is a file write. The
engine holds no document at all — it is handed a piece and plays it.

Which makes the distinction between the two kinds of message worth stating.
**Authoring** changes the document: a step placed, later a parameter turned.
**Performance** does not: a lane muted, a fader moved, a macro taken off its
curve, the transport scrubbed. Both reach the engine; only the first is written
down. It is the same distinction a mixing desk makes between playing and
authoring, and it is why moving a fader for an hour leaves a piece unmodified.
