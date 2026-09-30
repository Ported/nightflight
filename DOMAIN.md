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

### Writing the notes

A drum lane is a row of boxes: the only question per step is *whether*. A
pitched lane is a **piano roll** — rows are semitones from the lane's root, high
at the top, columns are steps — because the question is also *which*.

Which one you get is the instrument's own answer. A kick has a pitch and it is
not a note: it is a sweep from high to low that *is* the sound, and moving the
whole thing up a tone is a patch edit. A hat has no pitch worth naming.
Everything else plays what the steps say.

A column holds at most one note, because a lane holds one offset per step — so
clicking a cell *moves* the note there rather than adding one, and chords are
lanes. The rows span what the lane actually plays, padded out to an octave: a
hundred and twenty-eight rows of nothing would be honest and useless.

**Note length is a lane property, not a note's.** Bach's bass holds a half bar
and his top voice an eighth, and in this model that is two lanes rather than two
note lengths. It is the real limit of a step being `(velocity, offset)` and
nothing else. The control sits by the patch because a roll is unjudgeable
without it: the same notes at one step and at sixteen are two different pieces
of music.

**You would not hand-enter Bach.** The prelude is 352 steps a voice, derived by
eleven lines of Rust. Generators stay in `sets.rs` as provenance, the export
writes what they make into the library, and the roll is for writing a bass line
by hand and for *editing* what a generator produced.

### Where variation lives

Three places, and it is worth knowing which is which.

**Within a step**: velocity and a semitone offset. A kick a touch flatter on the
off-beats is an offset, not a second sound.

**Within a lane**: length. A line of steps that does not divide the bar drifts
against everything else, which is most of what makes hypnotic music hypnotic.

**Across lanes**: a different patch. Two kicks alternating in a beat is *two
lanes*, because a lane is one patch playing one line of steps. That is also why
a step does not carry a patch reference of its own: it would buy the same thing
while hiding the variation from the grid, and the second kick would have no
level, no placement, no send and no mute of its own.

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

## The library

Patches and clips are **material**: worth more than the piece that first needed
them, and worth editing on their own. So they are files with names of their own,
and a piece refers to them.

```text
library/patches/punch.json   an instrument with its parameters
library/clips/beat.json      lanes of steps, each on a patch
pieces/intro.json            tracks placing clips, macros, a room
```

A lane names its patch **and** keeps the values inline. That looks redundant and
is deliberate: a piece file stays whole and plays with no library present, which
matters because a piece is the thing you would send someone. What keeps the two
from drifting is a rule — **the library wins on load**, and the copy in the piece
is rewritten every time the piece is saved. So the copy is a cache, and editing a
patch changes every lane playing it.

**Save, and save as new.** Saving writes back to the name the thing already has,
and everything on that name moves with it. Saving as new writes a different name
and repoints only what you were editing. That pair is the whole of how a library
grows: fork a kick, push it around, and the beat that had the old one still has
the old one. It is also what makes "a different kick on this beat" mean anything
— two clips naming two patches.

A name is checked rather than escaped. Patch names come from a text box and a
text box can contain a slash; telling someone to pick another name is kinder than
quietly saving `drums/punch` as `drums_punch` and then failing to find it.

## What exists today, and what does not

Lanes, steps, clips, pieces and macros are real. A lane names the clip it belongs
to, the timeline draws one row per clip, and each clip has an editor tab.

**Tracks are not yet.** Spans still live on the lane rather than on a placement,
so a clip cannot appear twice in a piece with different timing. Nothing needs it
yet and the shape is there when it does.

Each clip editor plays its clip alone, looping, and leaving it puts the piece
back where it was.

**Patches are real.** Every parameter is declared once — field, default, range,
unit and scale together — so an interface can draw a fader for any instrument
without knowing which one it is looking at. A patch is saved under its own name,
shared between lanes, swapped in from a list, forked with *save as*, and created
from nothing by picking an instrument. The index tab lists everything saved and
plays a patch on a plain test line so you can hear what a name means.

**Clips are saved but not yet reused.** A clip is written to the library and can
be forked, but a piece still cannot pull one *in* from the library — only edit
and save the ones it already has, which is why the index shows the others as "not
in this piece". Adding a clip to a running piece means adding lanes to a running
engine, and that waits on the same swap this release built.

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
