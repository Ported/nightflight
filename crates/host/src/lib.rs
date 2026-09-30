//! The audio device, and the only thread with a deadline.
//!
//! Shared by every front end: the window and the server both start a stream this
//! way and then talk to it through the two queues. Nothing here knows which one
//! it is serving, which is the point.
//!
//! cpal hands us Core Audio's callback: a function the system calls every few
//! milliseconds asking for the next block of samples. It runs on a thread that
//! must finish inside the time the block it is filling will take to play — miss
//! that and the speaker gets silence, which is the click you hear as a dropout.
//!
//! So this is the whole of the audio side: drain the command queue, fill the
//! buffer, and occasionally push a telemetry frame. No allocation, no locks, no
//! file access, no printing. Everything it touches was built before the stream
//! started.

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, StreamConfig, SupportedBufferSize};
use engine::Engine;
use engine::seq::Set;
use engine::telemetry::{Command, Telemetry};

/// 128 samples is 2.7 ms at 48 kHz: low enough that a knob feels immediate, high
/// enough that we are not fighting the scheduler.
const WANTED_BUFFER: u32 = 128;
/// Telemetry frames a second. Faster than a screen refresh is wasted work.
const TELEMETRY_HZ: f32 = 60.0;
/// Samples the output takes to close and open around an engine being replaced.
/// Ten milliseconds: long enough not to click, short enough that switching to an
/// editor feels immediate.
const SWAP_FADE: f32 = 480.0;

/// The fade either side of an engine swap.
///
/// A piece being exchanged for a clip's audition is a discontinuity in every
/// sample of the output — different notes, different reverb tails, different
/// everything — so it has to happen in silence. This closes the output, reports
/// when it is safe to exchange, and opens it again.
///
/// It is a type rather than four variables in the callback because it is the one
/// part of the swap that fails quietly: get it wrong and you do not crash, you
/// get a click, or silence for ever. As a type it can be tested without an audio
/// device.
#[derive(Debug)]
struct Fade {
    /// 1 is open, 0 is closed.
    gain: f32,
    /// Per sample; 0 when it has arrived.
    step: f32,
}

impl Fade {
    const fn open() -> Self {
        Self {
            gain: 1.0,
            step: 0.0,
        }
    }

    /// Closed and holding: the moment to exchange engines.
    const fn silent(&self) -> bool {
        self.gain <= 0.0
    }

    /// Fully open and not moving, so the ramp can be skipped entirely.
    const fn resting(&self) -> bool {
        self.step == 0.0 && self.gain >= 1.0
    }

    fn start_closing(&mut self) {
        self.step = -1.0 / SWAP_FADE;
    }

    fn start_opening(&mut self) {
        self.step = 1.0 / SWAP_FADE;
    }

    /// Ramp across one buffer of interleaved stereo.
    fn apply(&mut self, out: &mut [f32]) {
        for frame in out.chunks_mut(2) {
            if self.step != 0.0 {
                self.gain = (self.gain + self.step).clamp(0.0, 1.0);
                // Stop at either end. Closing stops *at* zero and waits there
                // to be reopened; nothing restarts it on its own, which is what
                // keeps the exchange from happening halfway through a buffer.
                if (self.step > 0.0 && self.gain >= 1.0) || (self.step < 0.0 && self.gain <= 0.0) {
                    self.step = 0.0;
                }
            }
            for sample in frame {
                *sample *= self.gain;
            }
        }
    }
}

/// Whatever the audio thread is playing, as the caller sees it: `assert_no_alloc`
/// has to be installed by the binary, since a program may only have one
/// allocator. One line in `main`:
///
/// ```ignore
/// #[cfg(debug_assertions)]
/// #[global_allocator]
/// static ALLOCATOR: host::AllocDisabler = host::AllocDisabler;
/// ```
/// Only exists in debug builds: `assert_no_alloc` compiles its allocator out of
/// release builds, which is why the binaries gate the declaration the same way.
#[cfg(debug_assertions)]
pub use assert_no_alloc::AllocDisabler;

/// Run `f` under the audio thread's rules: in a debug build, any trip to the
/// allocator aborts the program rather than quietly stealing time.
#[inline]
pub fn realtime<T>(f: impl FnOnce() -> T) -> T {
    #[cfg(debug_assertions)]
    {
        assert_no_alloc::assert_no_alloc(f)
    }
    #[cfg(not(debug_assertions))]
    {
        f()
    }
}

/// A front end's end of the connection.
pub struct Link {
    pub commands: rtrb::Producer<Command>,
    pub telemetry: rtrb::Consumer<Telemetry>,
    /// Engines built off the audio thread, waiting to take over.
    swap: rtrb::Producer<Box<Engine>>,
    /// The ones they replaced, coming back to be dropped somewhere a deadline
    /// does not apply. Freeing an engine means freeing a reverb and several
    /// delay lines, which is exactly the kind of work the audio thread must
    /// never be asked to do.
    retired: rtrb::Consumer<Box<Engine>>,
    /// Read once, before the engine was handed to the audio thread. Commands
    /// carry indices into these.
    pub lane_names: Vec<String>,
    pub macro_names: Vec<String>,
    /// Everything about the set that does not change while it plays. Read before
    /// the engine was handed over, which is also why editing it later means the
    /// front end keeping its own copy.
    pub description: engine::telemetry::Description,
    pub device: String,
    pub buffer_frames: u32,
    /// Dropping this stops the stream, so the window has to hold it.
    _stream: cpal::Stream,
}

impl Link {
    /// Hand the audio thread a different engine.
    ///
    /// Built here, where allocating is allowed, and swapped there by moving one
    /// pointer. The output fades down before the swap and back up after, so a
    /// piece can be exchanged for a clip's audition without a click.
    ///
    /// Returns false if a swap is already waiting, in which case this one is
    /// dropped: the newest intent wins, and an editor clicked through quickly
    /// should not queue up five pieces to play in turn.
    pub fn load(&mut self, engine: Box<Engine>) -> bool {
        self.collect();
        self.swap.push(engine).is_ok()
    }

    /// Drop any engines the audio thread has finished with.
    pub fn collect(&mut self) {
        while self.retired.pop().is_ok() {}
    }

    pub fn send(&mut self, command: Command) {
        // If the queue is full the audio thread is not keeping up with us, which
        // cannot really happen at hand speed. Dropping is the right failure:
        // never block the UI, and never grow a queue the audio thread must drain.
        let _ = self.commands.push(command);
    }
}

/// Open the default output at 48 kHz and start playing `set`.
pub fn start(set: Set) -> Result<Link, Box<dyn Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default output device")?;
    let name = device.id()?.to_string();

    // The engine assumes 48 kHz throughout, so a device that cannot do it is an
    // error rather than something to paper over with resampling — on a Mac it is
    // one setting in Audio MIDI Setup.
    let supported = device
        .supported_output_configs()?
        .find(|c| {
            c.sample_format() == SampleFormat::F32
                && c.channels() >= 2
                && c.min_sample_rate() <= dsp::SAMPLE_RATE
                && c.max_sample_rate() >= dsp::SAMPLE_RATE
        })
        .ok_or("device has no 48 kHz stereo f32 output")?
        .with_sample_rate(dsp::SAMPLE_RATE);

    let buffer_size = match supported.buffer_size() {
        SupportedBufferSize::Range { min, max } => {
            BufferSize::Fixed(WANTED_BUFFER.clamp(*min, *max))
        }
        SupportedBufferSize::Unknown => BufferSize::Default,
    };
    let buffer_frames = match buffer_size {
        BufferSize::Fixed(frames) => frames,
        BufferSize::Default => WANTED_BUFFER,
    };
    let config = StreamConfig {
        channels: 2,
        sample_rate: dsp::SAMPLE_RATE,
        buffer_size,
    };

    // The piece is described before it is handed over. The engine holds no
    // document — it is given one and plays it — so this is the last moment
    // anyone can ask the piece about itself.
    let description = set.describe();

    // Built here, on this thread, and then moved into the callback: every
    // allocation the engine will ever need has happened by the time it plays.
    let mut engine = Box::new(Engine::new(dsp::SR, 126.0, set));
    let lane_names = engine.lane_names();
    let macro_names = engine.macro_names();

    let (command_tx, mut command_rx) = rtrb::RingBuffer::new(256);
    let (mut telemetry_tx, telemetry_rx) = rtrb::RingBuffer::new(8);
    // One at a time in each direction: there is never a reason to have two
    // pieces queued, and a second slot would only let them pile up.
    let (swap_tx, mut swap_rx) = rtrb::RingBuffer::<Box<Engine>>::new(1);
    let (mut retired_tx, retired_rx) = rtrb::RingBuffer::<Box<Engine>>::new(2);

    let xruns = Arc::new(AtomicU32::new(0));
    let reported = Arc::clone(&xruns);

    let report_every = (dsp::SR / buffer_frames as f32 / TELEMETRY_HZ).max(1.0) as u32;
    let mut blocks = 0u32;
    let mut load = 0.0f32;
    let mut fade = Fade::open();
    let mut waiting: Option<Box<Engine>> = None;

    let stream = device.build_output_stream(
        config,
        move |out: &mut [f32], _| {
            let started = Instant::now();
            realtime(|| {
                // Closed and something waiting: take over. Moving two boxes,
                // which is two pointers, and the old one goes back to be freed
                // where freeing is allowed.
                if fade.silent()
                    && let Some(next) = waiting.take()
                {
                    let previous = std::mem::replace(&mut engine, next);
                    let _ = retired_tx.push(previous);
                    fade.start_opening();
                }
                if waiting.is_none()
                    && let Ok(next) = swap_rx.pop()
                {
                    waiting = Some(next);
                    fade.start_closing();
                }

                while let Ok(command) = command_rx.pop() {
                    engine.apply(command);
                }
                engine.process(out);

                if !fade.resting() {
                    fade.apply(out);
                }
            });

            // How much of the deadline that took. Kept as the worst of the
            // frames since the last report, because the average tells you
            // nothing about whether you are about to glitch.
            let budget = (out.len() / 2) as f32 / dsp::SR;
            load = load.max(started.elapsed().as_secs_f32() / budget);

            blocks += 1;
            if blocks >= report_every {
                blocks = 0;
                let mut frame = engine.telemetry();
                frame.load = std::mem::take(&mut load);
                frame.xruns = reported.load(Ordering::Relaxed);
                // If the window is behind, drop the frame rather than wait.
                let _ = telemetry_tx.push(frame);
            }
        },
        {
            let xruns = Arc::clone(&xruns);
            move |err: cpal::Error| {
                if err.kind() == cpal::ErrorKind::Xrun {
                    xruns.fetch_add(1, Ordering::Relaxed);
                } else {
                    eprintln!("stream error: {err}");
                }
            }
        },
        None,
    )?;
    stream.play()?;

    Ok(Link {
        commands: command_tx,
        telemetry: telemetry_rx,
        swap: swap_tx,
        retired: retired_rx,
        lane_names,
        macro_names,
        description,
        device: name,
        buffer_frames,
        _stream: stream,
    })
}

#[cfg(test)]
mod tests {
    use super::{Fade, SWAP_FADE};

    /// One buffer of ones, ramped, returned.
    fn run(fade: &mut Fade, frames: usize) -> Vec<f32> {
        let mut out = vec![1.0f32; frames * 2];
        fade.apply(&mut out);
        out
    }

    #[test]
    fn closing_reaches_silence_and_waits_there() {
        let mut fade = Fade::open();
        assert!(fade.resting());
        fade.start_closing();

        // Buffer by buffer, as the callback would.
        let mut all = Vec::new();
        for _ in 0..8 {
            all.extend(run(&mut fade, 128));
        }
        assert!(fade.silent(), "480 samples fit in 1024");

        // Monotone down, and no step bigger than one sample's worth of ramp —
        // which is what "no click" means.
        let mut previous = 1.0;
        for &sample in &all {
            assert!(
                sample <= previous + 1e-6,
                "went back up: {sample} > {previous}"
            );
            assert!(
                (previous - sample) <= 1.0 / SWAP_FADE + 1e-6,
                "a jump of {}",
                previous - sample
            );
            previous = sample;
        }
        assert_eq!(all[all.len() - 1], 0.0);

        // It holds at zero: nothing but an exchange reopens it, so the swap can
        // never land halfway through a buffer.
        for _ in 0..4 {
            assert!(run(&mut fade, 128).iter().all(|&s| s == 0.0));
            assert!(fade.silent());
        }
    }

    #[test]
    fn opening_returns_to_full() {
        let mut fade = Fade::open();
        fade.start_closing();
        while !fade.silent() {
            run(&mut fade, 128);
        }
        fade.start_opening();

        let mut all = Vec::new();
        for _ in 0..8 {
            all.extend(run(&mut fade, 128));
        }
        assert!(fade.resting(), "open again, and not moving");

        let mut previous = 0.0;
        for &sample in &all {
            assert!(sample >= previous - 1e-6, "went back down");
            assert!((sample - previous) <= 1.0 / SWAP_FADE + 1e-6);
            previous = sample;
        }
        assert_eq!(all[all.len() - 1], 1.0);
    }

    #[test]
    fn the_ramp_is_the_same_whatever_the_buffer_size() {
        // The audio device picks the buffer size; the fade must not depend on it.
        let reference = {
            let mut fade = Fade::open();
            fade.start_closing();
            run(&mut fade, 2048)
        };
        for frames in [1, 7, 64, 128, 480, 1024] {
            let mut fade = Fade::open();
            fade.start_closing();
            let mut all = Vec::new();
            while all.len() < reference.len() {
                all.extend(run(&mut fade, frames));
            }
            assert_eq!(
                all[..reference.len()],
                reference[..],
                "buffer of {frames} frames ramps differently"
            );
        }
    }

    #[test]
    fn a_resting_fade_changes_nothing() {
        let mut fade = Fade::open();
        assert!(fade.resting());
        assert!(run(&mut fade, 64).iter().all(|&s| s == 1.0));
    }
}
