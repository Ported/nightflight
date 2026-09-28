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

## How it is put together

Three crates, and the boundaries are the design:

```
dsp      no dependencies at all. Oscillators, one filter, envelopes,
         instruments, binaural placement, a reverb. f32 blocks, no I/O,
         no allocation in any process path.

engine   depends only on dsp. Clock, sequencer, parts, spans, curves,
         macros, voice pool. `Engine::process(&mut [f32])` is the only
         entry point — it knows nothing about audio devices, windows
         or files.

app      the window. cpal owns the audio thread; egui draws. They talk
         through two lock-free ring buffers and two `Copy` structs.
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

Neither the author nor the machine writing most of this code can hear the output,
so **everything is verified by measurement**. `tools/analyse.py` reads a rendered
WAV and prints level, peak, the largest sample-to-sample jump, K-weighted
loudness, per-band reverberation time, and how much of a spun tone is not the
tone. It needs `numpy` and nothing else:

```
python3 tools/analyse.py renders/intro.wav --lufs --hits
```

That division is deliberate: **Rust makes sound, Python looks at it.** There is no
contract between them beyond a file on disk.

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
