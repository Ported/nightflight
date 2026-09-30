# nightflight

A real-time instrument for electronic music, written in Rust. Parts are launched
and dropped, the energy and the space are ridden on faders, and sources are
thrown around your head — heard binaurally on headphones.

Everything is synthesised. There are no samples in this repository.

```
cargo run -p app --release intro
```

Headphones. On speakers you lose the whole point of it.

## What it plays

Four sets ship with it, and they build on each other:

| set | what it is |
|---|---|
| `rolling` | Four on the floor at 126 BPM, hats orbiting your head, a bass rolling on the off-sixteenths, sidechained to the kick. |
| `prelude` | Bach's Prelude in C (BWV 846) transposed to G, played on two-operator FM, five chord voices on a slowly turning ring. |
| `pad` | A string machine through a phaser — the *Oxygène* sound — five voices spread around you. |
| `intro` | All of it as a piece: the Prelude with the pad under it, a helicopter flying the beat in from 40 m, one beat of held breath, and the landing. |

Pieces live in `pieces/` as JSON and are the source of truth. The Rust in
`crates/engine/src/sets.rs` is how they were first derived — Bach's figure over
his chords is eleven lines of code and a thousand steps of data — and
`cargo run -p engine --example export` writes out what it makes. Edit a piece
file and you edit the music.

## How it is put together

Three crates, and the boundaries are the design:

```
dsp      no dependencies at all. Oscillators, one filter, envelopes,
         instruments, binaural placement, a reverb. f32 blocks, no I/O,
         no allocation in any process path.

engine   depends only on dsp. Clock, sequencer, lanes, spans, curves,
         macros, voice pool, and the tab format. `Engine::process(&mut
         [f32])` is the only entry point — it knows nothing about audio
         devices, windows or files.
host     cpal owns the audio thread. Two lock-free ring buffers and two
         Copy structs are the whole of what crosses into it.
server   the engine over a WebSocket; the browser is the conductor.
nf-engine the format and the renderer as one binary, under `nf`.
```

Two consequences worth knowing, because most of the design follows from them:

**An offline render is the same code as the live stream.** `process()` in a loop
as fast as the machine allows, instead of once per audio callback. A test asserts
the output is bit-identical at block sizes of 1, 7, 64, 128, 137, 480 and 1024
frames, which is what makes that claim true rather than hopeful.

**The audio thread never waits.** No allocation, no locks, no I/O, no printing.
In debug builds `assert_no_alloc` aborts the program if the callback so much as
touches the allocator, which is how that rule is enforced rather than remembered.

## Working on it

```
cargo run -p app --release intro              # play it, with the window
cargo run -p engine --release --example wav -- 40 renders/intro.wav --set intro
cargo run -p engine --release --example macros # what the automation curves are doing
cargo run -p dsp   --release --example ir      # dump the reverb's impulse response
cargo test --release
```

### `nf`

Music is written as **tab** — a text grid, one line per lane, bars divided by
`|` — and `nf` is how you work with it. It needs `numpy` and nothing else.

```
nf ls                          what the library holds
nf show intro                  the piece as tab
nf new piece groove --bars 2   a file to start from
nf measure groove.tab          render it, then say what came out
nf measure intro --hits 0,7.6  and whether the music lands on those seconds
```

A piece is a name in `pieces/`, a path to a `.json`, or a path to a `.tab`;
all three work anywhere one is asked for. `nf measure` on a piece renders it
first, which is the loop: edit the grid, run one command, read the numbers.

**Rust owns the format and the sound; Python owns the surface.** The tab parser
is in `crates/engine/src/tab.rs` and is round-trip tested, so `nf` never reads a
tab file itself — it asks `nf-engine`, which is the same code the audio thread
runs. A second parser would drift from the first, and the half that changes
every session should be the half that is ten lines to extend.

Neither the author nor the machine writing most of this code can hear the
output, so **everything is verified by measurement**. `tools/analyse.py` is the
library underneath `nf measure` and is worth knowing directly for the things
the CLI does not surface — per-band reverberation time, note onsets, and how
much of a spun tone is not the tone:

```
python3 tools/analyse.py renders/intro.wav --lufs --rt60
```

The test suite is the same discipline in a form that fails a build. It checks
things like: a whole-number FM ratio keeps its pitch and a fractional one does
not; four-point Lagrange interpolation in the delay line beats linear by more
than a dB at 10 kHz; the reverb's decay time is what was asked for; a seek does
not click; a gate stays locked to the grid when the tempo changes.

## Where it came from

This grew out of a Python studio that renders the same music offline, and the
instrument recipes were ported from it — the 808's six inharmonic squares, the
kick's three layers, Woodworth's interaural delay fitted to a KU100 dummy head,
the reverb's damping curve. Two things were deliberately *not* carried over:
sample-for-sample parity with those renders, and Python itself. Comments that
refer to a preset "chosen by ear from four variants" are referring to that
lineage.

The offline renderer there still has things this does not: a measured
head-related transfer function, so front and back are distinguishable. Here they
are not, and there is a test asserting so, which will fail when that lands.

## Licence

Dual licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your
option — the usual arrangement for Rust projects.

That covers the code. The compositions in `crates/engine/src/sets.rs` are the
author's, except Bach's, who has been out of copyright for some time.
