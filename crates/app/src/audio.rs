//! The audio device, and the only thread with a deadline.
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

/// The window's end of the connection.
pub struct Link {
    pub commands: rtrb::Producer<Command>,
    pub telemetry: rtrb::Consumer<Telemetry>,
    /// Read once, before the engine was handed to the audio thread. Commands
    /// carry indices into these.
    pub part_names: Vec<&'static str>,
    pub macro_names: Vec<&'static str>,
    pub device: String,
    pub buffer_frames: u32,
    /// Dropping this stops the stream, so the window has to hold it.
    _stream: cpal::Stream,
}

impl Link {
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

    // Built here, on this thread, and then moved into the callback: every
    // allocation the engine will ever need has happened by the time it plays.
    let mut engine = Box::new(Engine::new(dsp::SR, 126.0, set));
    let part_names = engine.part_names();
    let macro_names = engine.macro_names();

    let (command_tx, mut command_rx) = rtrb::RingBuffer::new(256);
    let (mut telemetry_tx, telemetry_rx) = rtrb::RingBuffer::new(8);

    let xruns = Arc::new(AtomicU32::new(0));
    let reported = Arc::clone(&xruns);

    let report_every = (dsp::SR / buffer_frames as f32 / TELEMETRY_HZ).max(1.0) as u32;
    let mut blocks = 0u32;
    let mut load = 0.0f32;

    let stream = device.build_output_stream(
        config,
        move |out: &mut [f32], _| {
            let started = Instant::now();
            crate::realtime(|| {
                while let Ok(command) = command_rx.pop() {
                    engine.apply(command);
                }
                engine.process(out);
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
        part_names,
        macro_names,
        device: name,
        buffer_frames,
        _stream: stream,
    })
}
