# Plan

Where this is going, and the decisions it already rests on. The README says what
it is and how to run it; this is the part that changes.

## Decisions already made

**Rust for the engine, and only Rust.** Audio is computed in blocks of a few
milliseconds, hundreds of times a second, and one late block is an audible click.
A language with a garbage collector cannot promise that.

**No sample-for-sample parity with the Python studio it came from.** Matching
numpy's random generator and scipy's exact filter coefficients would have meant
f64 through the whole signal path and a comparison harness to maintain forever,
and none of it makes a sound. The recipes were ported; the numbers were not.

**The engine knows nothing about audio devices.** `Engine::process(&mut [f32])`
is its only entry point, so an offline render is the same code as the live stream
rather than a parallel implementation. A test holds the output bit-identical
across block sizes, which is what makes that true.

**The engine knows nothing about the user interface.** Two lock-free ring buffers
and two `Copy` structs: commands in, telemetry out. No locks, no shared state.
This is what makes the front end replaceable, and it is about to be replaced.

**The interface moves to the web.** egui got the instrument playable in an
afternoon and will not get much further. What this wants to become is a DAW —
a stack of tracks, automation curves you can grab and bend, a drum grid, a voice
designer — and every one of those components exists as a mature web library and
none of them exist for egui. The loop time matters more than the frame rate:
sub-second hot reload with devtools against an eight-second rebuild.

The shape is a local WebSocket rather than an embedded webview, at least first.
The engine becomes a server, the interface a browser tab. That keeps the interface
in a separate process where it cannot starve or crash the audio thread, needs no
bundler to get started, and the front end carries over unchanged if it is later
wrapped in Tauri.

## The real-time rules

- **Never wait** in the audio callback: no allocation, no locks, no file or
  network I/O, no printing. Everything is preallocated before the stream starts,
  and `assert_no_alloc` aborts a debug build if that is ever untrue.
- **Anything crossing to the audio thread is `Copy` and fixed-size.** A telemetry
  frame is 448 bytes and a command is 12; both are pinned by a test, because a
  `Vec` sneaking into either would not fail to compile, it would fail to be
  real-time.
- **Every parameter is smoothed.** A one-pole slew over about 20 ms, so nothing
  a hand touches can step.
- **A dropout counter is on screen.** If it moves, something broke a rule.
- **Anything built at run time is built off the audio thread** and handed over as
  a prepared object, with the old one sent back the other way to be dropped.

## What is next

**1. The socket, and a drum machine.** The first thing worth having is a grid you
can edit while it plays: sixteen steps per part, click a cell, hear it. The bass
is the same editor with pitch showing, which is a tab. Both are direct edits to
`Pattern`, which is already a list of velocity-and-pitch steps, so the engine
needs a command for one step rather than a way to swap whole patterns.

**2. Parameters and presets.** Every instrument parameter addressable, with a
range and a scale so an interface can draw a fader for it without knowing what
instrument it is looking at. Saving a set of them is a new preset of that
instrument. One declaration per parameter should generate the struct, the
defaults, and that description — anything less and the three drift apart.

**3. Tracks, curves, and capture.** A stack of tracks with the arrangement drawn
on it, automation lanes you can bend, and every command logged with the bar it
happened on. Then a performance is a score: play it live, edit what you played,
render it offline. The data model for this already exists — a macro reads a curve
every control block and a hand can override it — so most of the work is an editor,
not an engine.

**4. Voices at run time.** Adding and removing parts while it plays, which means
building the part off the audio thread and swapping it in. A modular graph where
oscillators and filters are wired together by hand is a much larger thing and a
much later one; presets over fixed instruments come first.

**5. Space, properly.** A measured head-related transfer function, so front and
back and height become distinguishable. Today there is interaural time and level
difference only, and a test asserts the front/back ambiguity so that it fails when
this lands.

**6. Hardware.** A MIDI controller, which produces the same commands a fader
does. Head tracking from AirPods, counter-rotating the scene so sounds stay put
in the room when you turn your head — the largest single gain available in
binaural realism.

**7. A limiter.** There is no master stage, so each set carries its own balance and
a set that is mixed too hot clips. This is the reason the arrival's gains are what
they are.

## Conventions

- 48 kHz, fixed. A device that cannot do it is an error, not something to resample
  around.
- Metres, listener at the origin, +x right, +y up, −z ahead. An angle of 90°
  clockwise from ahead is the right ear's side.
- The transport is an integer sample count. Where a step falls is derived from it.
- Control rate is a fixed grid of 64 samples in absolute time, not a cap on block
  size — placement interpolates across it, so a moving boundary would move the
  audio.
- Note lengths distinguish seconds from steps. A drum's ring is a physical fact;
  "a sixteenth" follows the tempo.
- Nothing starts or stops without a ramp.
