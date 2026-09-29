//! The player: a clock, some lanes and a pool of voices, with one entry point.
//!
//! `Engine::process` fills a buffer of interleaved stereo samples. cpal calls
//! it about three hundred times a second from the audio thread; an offline
//! render calls it in a loop as fast as the machine can go. Same code, so what
//! you hear live and what lands in a file cannot drift apart.
//!
//! It knows nothing about audio devices, windows or files, which is also what
//! makes it testable on a machine with no sound card.

pub mod audition;
pub mod automation;
pub mod clock;
pub mod document;
pub mod library;
pub mod mix;
pub mod seq;
pub mod sets;
pub mod telemetry;
pub mod voices;

use automation::{Flight, Macro};
use clock::Clock;
use dsp::inst::{Bass, Glass, Hat, Kick, Strings};
use dsp::reverb::Reverb;
use dsp::smooth::Smoothed;
use dsp::space::{Ears, Motion, Placer, Position};
use mix::Duck;
use seq::{Home, Lane, Set, Step, Voicing};
use telemetry::{Command, LaneState, Telemetry};
use voices::{AnyVoice, Pool};

/// The control rate: how often positions, faders and filter coefficients are
/// recomputed. 64 samples is 1.33 ms, 750 times a second.
///
/// It is a **fixed grid in absolute sample time**, not merely a cap on block
/// size, and that distinction is load-bearing. Placement interpolates a
/// source's delays and gains across each block, so if block boundaries moved
/// with whatever buffer size the audio device happened to hand us, the output
/// would too — and an offline render at 137 frames a block would not match the
/// live stream at 128. Pinning the boundaries to multiples of 64 samples makes
/// the result identical whatever the device does.
pub const CONTROL_BLOCK: usize = 64;

/// Scratch buffers are sized for one control block.
pub const MAX_BLOCK: usize = CONTROL_BLOCK;

/// Samples the master takes to close or open around a scrub. 6 ms is short
/// enough to feel instant and long enough that the cut is a dip, not a click.
const SEEK_FADE: usize = 288;

/// Where a lane with no place of its own flies to: just above your head.
const OVERHEAD: Position = Position {
    x: 0.0,
    y: 1.0,
    z: 0.0,
};

pub struct Engine {
    clock: Clock,
    pool: Pool,
    lanes: Vec<Lane>,
    macros: Vec<Macro>,
    /// One smoothed fader per lane, so a mute or a level change ramps instead
    /// of stepping. Parallel to `lanes`.
    levels: Vec<Smoothed>,
    duck: Duck,
    reverb: Option<Reverb>,
    /// One placer per placed lane, holding its delay line and filters.
    /// Parallel to `lanes`; `None` for lanes that stay in the centre.
    placers: Vec<Option<Placer>>,
    /// Laps each orbiting lane has turned so far. Accumulated rather than
    /// derived, so a change of speed never moves a source.
    laps: Vec<f64>,
    /// Peak of each lane's own contribution since the last telemetry frame.
    lane_peaks: Vec<f32>,
    /// Where each lane was when it was last drawn.
    lane_positions: Vec<Position>,
    /// Master peak since the last telemetry frame.
    peak: f32,
    /// Whether the transport is running. Paused, the clock stops and no new
    /// notes start, but everything already sounding is left to finish — so
    /// pausing sounds like the room emptying rather than like a cut.
    playing: bool,
    /// Bars to loop between, while editing a clip. Kept as bars rather than
    /// samples so it survives a change of tempo.
    loop_region: Option<(f32, f32)>,
    /// A bar the transport has been asked to jump to, waiting for the fade.
    seek_target: Option<f64>,
    /// The scrub fade: 1 is open, 0 is closed.
    seek_gain: f32,
    /// Per-sample change in that fade; 0 when it has arrived.
    seek_step: f32,
    /// The next step each flying lane owes, while it is in the air. Flights fire
    /// ahead of the beat, so they cannot use the shared step counter.
    flight_cursor: Vec<Option<u64>>,
    /// Scratch buffers for one sub-block. Preallocated: the audio thread never
    /// asks for memory.
    mix: [[f32; MAX_BLOCK]; 2],
    lane_buffer: [f32; MAX_BLOCK],
    /// The shared reverb feed. Every lane contributes through its own send, and
    /// the whole bus is reverberated once — the mixing desk's send bus, and the
    /// reason a room sounds like one room rather than one per instrument.
    send_bus: [f32; MAX_BLOCK],
    duck_curve: [f32; MAX_BLOCK],
    master: f32,
    /// Every macro mapping with its lane already found, so moving a fader
    /// never searches a list of names on the audio thread.
    wires: Vec<Wire>,
}

impl Engine {
    #[must_use]
    pub fn new(sr: f32, bpm: f32, set: Set) -> Self {
        let lanes = set.lanes;
        let levels = lanes
            .iter()
            .map(|p| Smoothed::new(sr, 0.02, p.gain))
            .collect();
        // Delay lines are 32 KB each, so they are built here, before the
        // stream starts, and only for lanes that actually move.
        let placers = lanes
            .iter()
            .map(|p| {
                // A lane that only ever sits in the centre needs no delay line;
                // one that flies does, even if it lands in the centre.
                if matches!(p.home, Home::Centre) && !p.ever_flies() {
                    None
                } else {
                    Some(Placer::new())
                }
            })
            .collect();
        // Resolved once: a name is a convenience for whoever writes a piece, and
        // has no business being looked up seven hundred and fifty times a second.
        let wires = set
            .macros
            .iter()
            .enumerate()
            .flat_map(|(macro_index, m)| {
                let lanes = &lanes;
                m.mappings.iter().filter_map(move |mapping| {
                    lanes
                        .iter()
                        .position(|lane| lane.name == mapping.lane)
                        .map(|lane| Wire {
                            macro_index,
                            lane,
                            target: mapping.target,
                            from: mapping.from,
                            to: mapping.to,
                            curve: mapping.curve,
                        })
                })
            })
            .collect();

        Self {
            clock: Clock::new(sr, bpm),
            pool: Pool::default(),
            laps: vec![0.0; lanes.len()],
            flight_cursor: vec![None; lanes.len()],
            lane_peaks: vec![0.0; lanes.len()],
            lane_positions: vec![Position::default(); lanes.len()],
            peak: 0.0,
            playing: true,
            loop_region: None,
            seek_target: None,
            seek_gain: 1.0,
            seek_step: 0.0,
            lanes,
            macros: set.macros,
            levels,
            placers,
            // Rolling's pump: 6 dB down in 5 ms, back up over 200 ms.
            duck: Duck::new(sr, 6.0, 0.005, 0.2),
            reverb: set
                .reverb
                .map(|r| Reverb::new(sr, r.rt60, r.damping, r.predelay, r.lowcut)),
            mix: [[0.0; MAX_BLOCK]; 2],
            lane_buffer: [0.0; MAX_BLOCK],
            send_bus: [0.0; MAX_BLOCK],
            duck_curve: [0.0; MAX_BLOCK],
            master: 0.5,
            wires,
        }
    }

    /// Fill `out` with interleaved stereo samples.
    ///
    /// The loop splits the buffer at every step boundary, so a note starts on
    /// its exact sample rather than being nudged to the start of a block. That
    /// is what keeps the groove tight at 126 BPM, where a sixteenth is
    /// 5714.28 samples and never lands on a block edge.
    pub fn process(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames {
            // A scrub waits here until the fade has closed, then jumps between
            // sub-blocks, so nothing is ever half-rendered across the cut.
            if self.seek_gain <= 0.0
                && let Some(bar) = self.seek_target.take()
            {
                self.jump_to(bar);
                self.seek_step = 1.0 / SEEK_FADE as f32;
            }

            // Wrap before working out what is due, so the step at the loop's
            // start is the next one to fire rather than one already past.
            if let Some((from, to)) = self.loop_region
                && self.playing
                && self.clock.bar() >= f64::from(to)
            {
                self.loop_back(f64::from(from));
            }

            // Stopped, the clock does not move, so no step is ever due. The
            // guard matters: with a frozen clock the next step is permanently
            // overdue, and firing it would loop forever inside one callback.
            let control = CONTROL_BLOCK as u64;
            let until_control = (control - self.clock.sample % control) as usize;
            let n = if self.playing {
                let next = self.clock.next_step_sample();
                if next <= self.clock.sample {
                    self.fire_step();
                    self.clock.step += 1;
                    continue;
                }
                let until_step = (next - self.clock.sample) as usize;
                until_step.min(until_control).min(frames - done)
            } else {
                until_control.min(frames - done)
            };

            // Positions are taken at the ends of the control period this
            // sub-block sits inside, never at the sub-block's own edges.
            let base = self.clock.sample - self.clock.sample % control;
            self.render(n, base, (self.clock.sample % control) as usize);

            let master = self.master;
            for (i, frame) in out[done * 2..(done + n) * 2].chunks_mut(2).enumerate() {
                // The seek fade rides on top of the master, per sample, so a
                // jump is a six-millisecond dip rather than a step.
                if self.seek_step != 0.0 {
                    self.seek_gain = (self.seek_gain + self.seek_step).clamp(0.0, 1.0);
                    if self.seek_gain >= 1.0 || (self.seek_gain <= 0.0 && self.seek_step < 0.0) {
                        self.seek_step = 0.0;
                    }
                }
                let gain = master * self.seek_gain;
                frame[0] = self.mix[0][i] * gain;
                frame[1] = self.mix[1][i] * gain;
                self.peak = self.peak.max(frame[0].abs()).max(frame[1].abs());
            }

            if self.playing {
                self.clock.advance(n);
            }
            done += n;
        }
    }

    /// One sub-block: every lane into its own buffer, then summed.
    ///
    /// Per-lane buffers are not tidiness — the duck has to apply to the bass
    /// and not to the kick that triggers it, so they cannot share a bus.
    ///
    /// A lane with live voices is always advanced, muted or not. Skipping it
    /// would leak: `add_lane` is what retires a finished voice, so voices in a
    /// skipped lane never die, the pool fills, and the lanes you *can* hear
    /// start losing their slots.
    fn render(&mut self, n: usize, control_start: u64, offset: usize) {
        let base_sample = self.clock.sample;
        let bar = self.clock.bar_of(control_start);
        self.apply_macros(bar);
        if offset == 0 && self.playing {
            self.fire_flights();
        }
        let bar_end = self.clock.bar_of(control_start + CONTROL_BLOCK as u64);
        for duck in self.duck_curve[..n].iter_mut() {
            *duck = self.duck.tick();
        }

        self.mix[0][..n].fill(0.0);
        self.mix[1][..n].fill(0.0);
        self.send_bus[..n].fill(0.0);
        for (index, lane) in self.lanes.iter().enumerate() {
            let level = &mut self.levels[index];
            // Muted, or the score has it silent: either way the fader goes to
            // zero and gets there smoothly.
            let scored = lane.scored(bar).unwrap_or(0.0);
            // A lane in the air is quieter than the same lane landed: the
            // intro's balance depends on the landing being a clear step up
            // rather than a jolt.
            let airborne = lane.flying(bar).map_or(1.0, |(f, _)| f.gain);
            level.set(if lane.muted {
                0.0
            } else {
                lane.gain * scored * airborne
            });
            let sounding = self.pool.playing(index);
            let placed = self.placers[index].is_some();

            // A placed lane is processed even when it is silent, because its
            // delay line holds sound still travelling to the ears — up to
            // 4.4 ms at 1.5 m. Freezing the line between hits would smear the
            // tail of one hat onto the attack of the next.
            if !sounding && !placed {
                // Nothing ringing and nothing in flight: keep the fader moving
                // so it is where it should be when the lane comes back.
                for _ in 0..n {
                    level.tick();
                }
                continue;
            }

            self.lane_buffer[..n].fill(0.0);
            if sounding {
                self.pool.add_lane(index, &mut self.lane_buffer[..n]);
            }
            // The chop, if this lane has one. It comes before the fader so
            // that muting a gated lane still fades smoothly.
            if let Some(gate) = &lane.gate {
                let spss = self.clock.samples_per_step();
                for (i, s) in self.lane_buffer[..n].iter_mut().enumerate() {
                    *s *= gate.gain(base_sample + i as u64, spss, dsp::SR);
                }
            }

            // The fader is ticked for every sample either way, so it tracks
            // real time rather than how much the lane happened to play.
            if lane.ducked {
                for (s, &d) in self.lane_buffer[..n]
                    .iter_mut()
                    .zip(self.duck_curve[..n].iter())
                {
                    *s *= level.tick() * d;
                }
            } else {
                for s in self.lane_buffer[..n].iter_mut() {
                    *s *= level.tick();
                }
            }

            // Then either straight up the middle, or placed around the head —
            // and if it is in the air, on the aircraft instead.
            let turned = self.laps[index];
            let turning = lane.home.laps_per_bar() * (bar_end - bar);
            let (start, finish, send) = match lane.flying(bar) {
                Some((flight, lands)) => {
                    let home = |laps, at| lane.home.at(laps, at).unwrap_or(OVERHEAD);
                    (
                        Some(home(turned, bar) + flight.offset(flight.progress(lands, bar))),
                        Some(
                            home(turned + turning, bar_end)
                                + flight.offset(flight.progress(lands, bar_end)),
                        ),
                        flight.send,
                    )
                }
                None => (
                    lane.home.at(turned, bar),
                    lane.home.at(turned + turning, bar_end),
                    lane.send,
                ),
            };
            // For the window: how loud this lane is and where it is.
            let lane_peak = self.lane_buffer[..n]
                .iter()
                .fold(0.0f32, |m, s| m.max(s.abs()));
            self.lane_peaks[index] = self.lane_peaks[index].max(lane_peak);
            if let Some(at) = start {
                self.lane_positions[index] = at;
            }

            match (&mut self.placers[index], start, finish) {
                (Some(placer), Some(from), Some(to)) => {
                    let (left, right) = self.mix.split_at_mut(1);
                    let mut ears = Ears {
                        left: &mut left[0][..n],
                        right: &mut right[0][..n],
                        send: &mut self.send_bus[..n],
                        send_level: send,
                    };
                    let motion = Motion {
                        from,
                        to,
                        offset,
                        period: CONTROL_BLOCK,
                    };
                    placer.place(&self.lane_buffer[..n], motion, &mut ears, dsp::SR);
                }
                _ => {
                    for ear in 0..2 {
                        for (m, &s) in self.mix[ear][..n]
                            .iter_mut()
                            .zip(self.lane_buffer[..n].iter())
                        {
                            *m += s;
                        }
                    }
                    if send > 0.0 {
                        for (bus, &s) in self.send_bus[..n]
                            .iter_mut()
                            .zip(self.lane_buffer[..n].iter())
                        {
                            *bus += s * send;
                        }
                    }
                }
            }
        }

        // The orbits advance exactly once per control period, however that
        // period was carved up, so the result does not depend on the buffer
        // size. Stopped, they hold: a source's place is musical time, not
        // wall-clock time.
        if offset + n == CONTROL_BLOCK && self.playing {
            let bars = CONTROL_BLOCK as f64 / (self.clock.samples_per_step() * seq::STEPS_PER_BAR as f64);
            for (laps, lane) in self.laps.iter_mut().zip(&self.lanes) {
                *laps += lane.home.laps_per_bar() * bars;
            }
        }

        // One room for everything, mixed in around the listener rather than
        // from any source's direction.
        if let Some(reverb) = &mut self.reverb {
            let (left, right) = self.mix.split_at_mut(1);
            reverb.process(
                &self.send_bus[..n],
                &mut left[0][..n],
                &mut right[0][..n],
                1.0,
            );
        }
    }

    /// Start whatever this step asks of every lane that is on the ground.
    fn fire_step(&mut self) {
        let bar = self.clock.bar();
        let step = self.clock.step;
        for index in 0..self.lanes.len() {
            let lane = &self.lanes[index];
            // A lane outside its span starts no new notes; anything already
            // ringing is left to finish.
            if lane.muted || lane.scored(bar).is_none() {
                continue;
            }
            // A lane in the air fires ahead of the beat instead: see `fire_flights`.
            if lane.flying(bar).is_some() {
                continue;
            }
            self.fire_lane(index, step);
        }
    }

    /// Start the notes a flying lane owes, early.
    ///
    /// This is the one place the sequencer does not fire on the beat. A source
    /// 40 m away is heard 116 ms after it sounds — a quarter of a beat at
    /// 126 BPM — so a helicopter that played on the beat would arrive audibly
    /// late against everything else. Instead each note is emitted early by its
    /// own travel time, so it *lands* on the beat. A PA at the back of a field
    /// is delayed for the same reason, in the other direction.
    ///
    /// Doppler survives this, because the correction is fixed for the length of
    /// a note while the delay keeps changing underneath it.
    ///
    /// Called once per control period, so a flown note starts within 1.33 ms of
    /// where it should — against a sixteenth of 119 ms.
    fn fire_flights(&mut self) {
        let now = self.clock.sample;
        let bar = self.clock.bar();
        let spss = self.clock.samples_per_step();
        for index in 0..self.lanes.len() {
            let Some((flight, lands)) = self.lanes[index].flying(bar) else {
                self.flight_cursor[index] = None;
                continue;
            };
            if self.lanes[index].muted {
                continue;
            }

            let home = self.lanes[index]
                .home
                .at(self.laps[index], bar)
                .unwrap_or(OVERHEAD);
            let position = home + flight.offset(flight.progress(lands, bar));
            let travel = (Flight::travel(position) * dsp::SR) as u64;
            let due = self.clock.step_of(now + travel);

            // The breath: the last beats before the landing are a hole.
            let breath = (f64::from(flight.breath_beats) * 4.0 * spss) as u64;
            let cutoff = self
                .clock
                .sample_of_bar(f64::from(lands))
                .saturating_sub(breath);

            let mut cursor = self.flight_cursor[index].unwrap_or_else(|| self.clock.step_of(now));
            while cursor <= due {
                if self.clock.step_sample(cursor) < cutoff {
                    self.fire_lane(index, cursor);
                }
                cursor += 1;
            }
            self.flight_cursor[index] = Some(cursor);
        }
    }

    /// One lane, one step.
    fn fire_lane(&mut self, index: usize, step: u64) {
        let sr = dsp::SR;
        let spss = self.clock.samples_per_step();
        let lane = &self.lanes[index];
        let hit = lane.pattern.at(step);
        if hit.velocity <= 0.0 {
            return;
        }
        let length = lane.length.seconds(spss, sr);
        let pitch = lane.root + f32::from(hit.offset);
        let velocity = (hit.velocity * lane.velocity_scale).clamp(0.0, 4.0);
        let voice = match lane.voicing {
            Voicing::Kick(p) => {
                self.duck.trigger();
                AnyVoice::Kick(Kick::new(sr, pitch, velocity, length, p))
            }
            // The step number seeds the phases, so every hit differs and
            // the render still repeats exactly.
            Voicing::Hat(p) => AnyVoice::Hat(Hat::new(
                sr,
                velocity,
                length,
                (step as u32).wrapping_mul(2_654_435_761),
                p,
            )),
            Voicing::Bass(p) => AnyVoice::Bass(Bass::new(sr, pitch, velocity, length, p)),
            Voicing::Strings(p) => AnyVoice::Strings(Strings::new(
                sr,
                pitch,
                velocity,
                length,
                self.clock.sample as f32 / sr,
                p,
            )),
            // Glass needs the time since the transport started, not since
            // the note started: its wow bends every note together.
            Voicing::Glass(p) => AnyVoice::Glass(Glass::new(
                sr,
                pitch,
                velocity,
                length,
                self.clock.sample as f32 / sr,
                p,
            )),
        };
        self.pool.start(index, voice);
    }

    /// Move every parameter each macro is wired to. Once per control block:
    /// 750 times a second is far finer than a hand, and the parameters that feed
    /// continuous DSP are smoothed downstream anyway.
    fn apply_macros(&mut self, bar: f64) {
        for wire in &self.wires {
            let value = self.macros[wire.macro_index].at(bar);
            let shaped = value.clamp(0.0, 1.0).powf(wire.curve);
            let target = wire.from + (wire.to - wire.from) * shaped;
            if let Some(lane) = self.lanes.get_mut(wire.lane) {
                automation::apply(lane, wire.target, target);
            }
        }
    }

    /// Put a hand on a macro, overriding any automation. `None` gives it back.
    pub fn set_macro(&mut self, name: &str, value: Option<f32>) -> bool {
        match self.macros.iter_mut().find(|m| m.name == name) {
            Some(m) => {
                m.manual = value.map(|v| v.clamp(0.0, 1.0));
                true
            }
            None => false,
        }
    }

    #[must_use]
    pub fn macro_names(&self) -> Vec<String> {
        self.macros.iter().map(|m| m.name.clone()).collect()
    }

    /// Where each macro currently sits.
    #[must_use]
    pub fn macro_values(&self) -> Vec<(String, f32)> {
        let bar = self.clock.bar();
        self.macros
            .iter()
            .map(|m| (m.name.clone(), m.at(bar)))
            .collect()
    }

    /// A one-line dump of what the macros have written into the lanes. For
    /// diagnosing automation; the window will show this properly.
    #[must_use]
    pub fn debug_lanes(&self) -> String {
        self.lanes
            .iter()
            .map(|p| {
                let gate = p.gate.as_ref().map_or(0.0, |g| g.depth);
                let lap = match p.home {
                    Home::Orbit { bars_per_lap, .. } => bars_per_lap,
                    Home::Centre => 0.0,
                };
                format!(
                    "{}: v{:.2} g{:.2} l{:.1}",
                    p.name, p.velocity_scale, gate, lap
                )
            })
            .collect::<Vec<_>>()
            .join("  ")
    }

    /// The master level, applied after everything. A limiter belongs here
    /// eventually; until then this is how a set is kept under full scale.
    pub fn set_master(&mut self, gain: f32) {
        self.master = gain.clamp(0.0, 4.0);
    }

    #[must_use]
    pub fn master(&self) -> f32 {
        self.master
    }

    /// How hard a lane's gate chops, 0 to 1. Returns false if the lane has no
    /// gate. This is the control the intro's build rides on.
    pub fn set_gate_depth(&mut self, name: &str, depth: f32) -> bool {
        match self.lanes.iter_mut().find(|p| p.name == name) {
            Some(lane) => match &mut lane.gate {
                Some(gate) => {
                    gate.depth = depth.clamp(0.0, 1.0);
                    true
                }
                None => false,
            },
            None => false,
        }
    }

    /// Change tempo from here on.
    pub fn set_bpm(&mut self, bpm: f32) {
        self.clock.set_bpm(bpm);
    }

    /// Mute or unmute a lane by name. Returns false if there is no such lane.
    pub fn set_muted(&mut self, name: &str, muted: bool) -> bool {
        match self.lanes.iter_mut().find(|p| p.name == name) {
            Some(lane) => {
                lane.muted = muted;
                true
            }
            None => false,
        }
    }

    /// Set how much of a lane goes to the reverb. Returns false if there is no
    /// such lane.
    pub fn set_send(&mut self, name: &str, send: f32) -> bool {
        match self.lanes.iter_mut().find(|p| p.name == name) {
            Some(lane) => {
                lane.send = send.clamp(0.0, 1.0);
                true
            }
            None => false,
        }
    }

    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        dsp::SR
    }

    /// Mute everything but one lane.
    pub fn solo(&mut self, name: &str) -> bool {
        let found = self.lanes.iter().any(|p| p.name == name);
        for lane in &mut self.lanes {
            lane.muted = lane.name != name;
        }
        found
    }

    /// Each lane's name and current reverb send.
    #[must_use]
    pub fn sends(&self) -> Vec<(String, f32)> {
        self.lanes
            .iter()
            .map(|lane| (lane.name.clone(), lane.send))
            .collect()
    }

    #[must_use]
    pub fn lane_names(&self) -> Vec<String> {
        self.lanes.iter().map(|lane| lane.name.clone()).collect()
    }

    /// Bar position, voice count and dropped-voice count, for the UI.
    /// Everything the window draws. Reading it clears the peak meters, so each
    /// frame reports the loudest moment since the last one — which is what a
    /// meter should show, rather than whatever happened to be true at the
    /// instant it was asked.
    pub fn telemetry(&mut self) -> Telemetry {
        let bar = self.clock.bar();
        let mut frame = Telemetry {
            bar: bar as f32,
            bpm: self.clock.bpm(),
            voices: self.pool.active() as u16,
            dropped: self.pool.dropped,
            peak: std::mem::take(&mut self.peak),
            playing: self.playing,
            loop_from: self.loop_region.map_or(0.0, |(from, _)| from),
            loop_to: self.loop_region.map_or(0.0, |(_, to)| to),
            lane_count: self.lanes.len().min(telemetry::MAX_PARTS) as u8,
            ..Telemetry::default()
        };
        for (i, lane) in self.lanes.iter().enumerate().take(telemetry::MAX_PARTS) {
            let at = self.lane_positions[i];
            frame.lanes[i] = LaneState {
                level: std::mem::take(&mut self.lane_peaks[i]),
                position: [at.x, at.y, at.z],
                placed: self.placers[i].is_some(),
                sounding: self.pool.playing(i),
                muted: lane.muted,
                gain: lane.gain,
            };
        }
        frame.macro_count = self.macros.len().min(telemetry::MAX_MACROS) as u8;
        for (slot, m) in frame.macros.iter_mut().zip(&self.macros) {
            *slot = m.at(bar);
        }
        frame
    }

    /// Start or stop the transport.
    ///
    /// Stopping is deliberately not a cut. The clock freezes and no new note
    /// starts, but every voice already ringing runs to its end and the reverb
    /// tail decays, so pressing stop sounds like a room emptying. It also means
    /// stopping never clicks, because nothing is interrupted.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    #[must_use]
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Put the transport at `bar` before the engine is playing.
    ///
    /// Not a scrub: there is nothing to fade, because nothing is sounding yet.
    /// Used when an engine is built to replace another one — whoever does the
    /// swap is responsible for the fade across it.
    pub fn start_at(&mut self, bar: f32) {
        self.jump_to(f64::from(bar.max(0.0)));
        self.seek_gain = 1.0;
        self.seek_step = 0.0;
    }

    /// Loop between two bars, or stop looping.
    pub fn set_loop(&mut self, region: Option<(f32, f32)>) {
        // A loop that does not move forwards would spin inside one callback.
        self.loop_region = region.filter(|(from, to)| to > from);
    }

    #[must_use]
    pub fn loop_region(&self) -> Option<(f32, f32)> {
        self.loop_region
    }

    /// Wrap the transport back to `bar`, seamlessly.
    ///
    /// Nothing like a scrub: a loop point is a musical edge, not a cut, so
    /// nothing is silenced and nothing is faded. Notes ringing at the end of the
    /// bar continue over the seam — which is what makes a hat's tail carry into
    /// the next pass — the room keeps its tail, and the orbits keep turning,
    /// since a source circling every four bars should not jump back each time a
    /// one-bar loop comes round.
    ///
    /// The flight cursors do reset: they remember which steps they have already
    /// fired ahead of the beat, and after a wrap those steps are due again.
    fn loop_back(&mut self, bar: f64) {
        self.clock.seek(bar);
        self.flight_cursor.fill(None);
    }

    /// Move the transport to `bar`, fading out and back in around the cut.
    ///
    /// A scrub is not a seek in a video player: everything sounding has to stop,
    /// or a pad note with a four-second release rings on over the new position,
    /// and every delay line has to forget, or 170 ms of the place we just left
    /// arrives at the new one. What is *not* reset is the macros — a hand on a
    /// fader stays where the hand left it.
    pub fn seek(&mut self, bar: f32) {
        self.seek_target = Some(f64::from(bar.max(0.0)));
        self.seek_step = -1.0 / SEEK_FADE as f32;
    }

    fn jump_to(&mut self, bar: f64) {
        self.clock.seek(bar);
        self.pool.clear();
        for placer in self.placers.iter_mut().flatten() {
            placer.reset();
        }
        if let Some(reverb) = &mut self.reverb {
            reverb.reset();
        }
        // The orbits are put where they would be if they had been turning at
        // their current rate all along. Not quite what an unbroken playthrough
        // would give when the rate has been changing — the honest answer would
        // need the integral of the whole curve — but a source's place in its own
        // lap is not something an ear can be wrong about.
        for (laps, lane) in self.laps.iter_mut().zip(&self.lanes) {
            *laps = bar * lane.home.laps_per_bar();
        }
        self.flight_cursor.fill(None);
        self.lane_peaks.fill(0.0);
        self.peak = 0.0;
        self.seek_gain = 0.0;
    }

    /// Apply one command from the window. Nothing here allocates, waits or
    /// fails: a command that names something that does not exist is dropped.
    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Macro { index, value } => {
                if let Some(m) = self.macros.get_mut(index as usize) {
                    m.manual = value.map(|v| v.clamp(0.0, 1.0));
                }
            }
            Command::Mute { index, muted } => {
                if let Some(lane) = self.lanes.get_mut(index as usize) {
                    lane.muted = muted;
                }
            }
            Command::Level { index, gain } => {
                if let Some(lane) = self.lanes.get_mut(index as usize) {
                    lane.gain = gain.clamp(0.0, 4.0);
                }
            }
            Command::Bpm { value } => self.clock.set_bpm(value),
            Command::Seek { bar } => self.seek(bar),
            Command::Playing { value } => self.playing = value,
            Command::Loop { from, to, on } => {
                self.set_loop(on.then_some((from, to)));
            }
            Command::SetParam { lane, param, value } => {
                if let Some(lane) = self.lanes.get_mut(lane as usize) {
                    lane.voicing.set_param(param as usize, value);
                }
            }
            Command::SetStep {
                lane,
                step,
                velocity,
                offset,
            } => {
                if let Some(lane) = self.lanes.get_mut(lane as usize) {
                    lane.pattern
                        .set(step as usize, Step::new(velocity.clamp(0.0, 2.0), offset));
                }
            }
            Command::Master { value } => self.set_master(value),
        }
    }

    #[must_use]
    pub fn state(&self) -> State {
        State {
            bar: self.clock.bar(),
            bpm: self.clock.bpm(),
            voices: self.pool.active(),
            dropped: self.pool.dropped,
        }
    }
}

/// One macro mapping, with the lane it moves already found.
#[derive(Clone, Copy, Debug)]
struct Wire {
    macro_index: usize,
    lane: usize,
    target: automation::Target,
    from: f32,
    to: f32,
    curve: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct State {
    pub bar: f64,
    pub bpm: f32,
    pub voices: usize,
    pub dropped: u32,
}
