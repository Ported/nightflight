# nightflight

A real-time instrument for electronic music in Rust. Read `README.md` for what it
is and `PLAN.md` for where it is going; both are kept current and neither
duplicates the other.

## How to work here

**Nobody involved can hear the output.** Verify by measurement, always, and report
the numbers. Never say something "sounds good".

- `tools/analyse.py <wav>` with `--lufs`, `--rt60`, `--hits`, `--tone=1000`:
  level, peak, largest sample-to-sample jump, K-weighted loudness, per-band
  reverberation time, onsets and their spacing, and how much of a spun tone is not
  the tone. Needs `numpy`, nothing else.
- Rust makes sound; Python looks at it. There is no contract between them beyond a
  file on disk.
- **A measurement that surprises you is more often the measurement's fault than the
  code's.** This has been true most times it has come up: a brick-wall band filter
  read a 7 s reverb tail as 17 s; an energy comparison could not see a kick's punch
  because saturation had flattened it; a mono sum cancelled exactly the decorrelated
  content that placement creates; a test tone generated from absolute time carried
  −50 dB of its own f32 phase noise. Check the instrument before the subject.
- When a measurement was wrong, **write why into the test comment**, not just the
  fix. Those comments are the most valuable prose in the repository.

**Teach the music, not the programming.** Explain what a musical idea is, where it
comes from and what to listen for, before building it. Use the real vocabulary and
define each term once. Say what to listen for and when.

**Small steps.** Each one ends with something that can be run and heard.

**Refactors that should not change the sound get proven.** Render before and after
and diff the samples; bit-identical or explain why not.

## Architecture

```
dsp      no dependencies. f32 blocks, no I/O, no allocation in process paths.
engine   depends only on dsp. `Engine::process(&mut [f32])` is the only entry
         point: no audio device, no window, no files.
app      cpal owns the audio thread, egui draws. Two lock-free ring buffers and
         two Copy structs between them.
```

The audio thread never waits: no allocation, no locks, no I/O, no printing.
`assert_no_alloc` aborts a debug build if it does. Anything built at run time is
built off that thread and handed over as a prepared object.

Both of those boundaries are load-bearing and neither is decoration. The first is
why an offline render is the same code as the live stream; the second is why the
front end can be replaced without touching a line of DSP.

## Commands

```
cargo run -p app --release intro          # play, with the window: rolling | prelude | pad | intro
cargo run -p engine --release --example wav -- 40 renders/intro.wav --set intro
cargo run -p engine --release --example macros   # what the automation is doing, bar by bar
cargo run -p dsp --release --example ir          # the reverb's impulse response
cargo test --release && cargo clippy --all-targets && cargo fmt --all
```

A debug build has `assert_no_alloc` armed and DSP too slow to keep up. Run
`--release` to listen; run debug to check the rules.

## Traps this project has already fallen into

- **An edit that silently does not apply.** A string replacement whose anchor
  `cargo fmt` had already rewritten did nothing; the build passed because nothing
  changed and the tests passed because they cover the engine, not the window. A
  button was reported as shipped and did not exist. **Verify an edit landed.**
- **A stale binary.** A failed `cargo build` left the previous binary in place, so
  measurements described code that no longer compiled. Read the compiler output
  before the numbers.
- **Missing glyphs.** egui's default fonts do not carry U+23F8 and friends, and a
  button whose label is a missing glyph is an invisible button. Use words.
- **zsh does not word-split unquoted parameters.** `$args` reaches a command as one
  argument, so a sweep silently ran the same case several times. Use arrays or
  explicit arguments.
