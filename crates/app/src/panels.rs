//! The window: lane strips, macro faders, and a health readout.
//!
//! Every control here holds its own value and sends a command when it moves. It
//! deliberately does *not* read the value back from telemetry each frame — a
//! fader that follows what the engine reports would jump under your finger every
//! time a frame arrived late. Telemetry drives the meters and the picture; the
//! faders drive the engine.
//!
//! The one exception is a macro under automation: there the curve owns the value
//! and the fader follows it, which is what "auto" means. Touching it takes over.

use egui::{Color32, RichText, Slider};
use engine::telemetry::{Command, Telemetry};

use crate::plan;
use host::Link;

const GOOD: Color32 = Color32::from_rgb(120, 200, 150);
const WARN: Color32 = Color32::from_rgb(230, 190, 110);
const BAD: Color32 = Color32::from_rgb(230, 120, 120);

pub struct App {
    link: Link,
    set: String,
    /// How far the scrub bar reaches, in bars.
    length: f32,
    latest: Telemetry,
    /// Where the hand is holding the playhead, while it is holding it. The
    /// handle has to stop following telemetry during a drag or it fights the
    /// finger dragging it.
    scrub: Option<f32>,
    /// What the window believes, which is what it has sent.
    levels: Vec<f32>,
    mutes: Vec<bool>,
    /// `None` means the curve is driving it.
    macros: Vec<Option<f32>>,
    bpm: f32,
    master: f32,
}

impl App {
    #[must_use]
    pub fn new(link: Link, set: String, length: f32) -> Self {
        let lanes = link.lane_names.len();
        let macros = link.macro_names.len();
        Self {
            link,
            set,
            length: length.max(1.0),
            latest: Telemetry::default(),
            scrub: None,
            levels: vec![f32::NAN; lanes],
            mutes: vec![false; lanes],
            macros: vec![None; macros],
            bpm: 126.0,
            master: 0.5,
        }
    }

    /// Take the newest telemetry frame and throw away any backlog: an old frame
    /// is of no interest to a meter.
    fn drain(&mut self) {
        while let Ok(frame) = self.link.telemetry.pop() {
            self.latest = frame;
        }
        // The first frame is where the faders learn the set's own levels.
        for (i, level) in self.levels.iter_mut().enumerate() {
            if level.is_nan()
                && let Some(lane) = self.latest.lanes.get(i)
            {
                *level = lane.gain;
            }
        }
    }

    /// The scrub bar: drag the playhead to move through the piece.
    ///
    /// Only a change of value sends a seek. Holding the handle still sends
    /// nothing, so the music keeps playing from where you put it rather than
    /// re-cutting to the same bar sixty times a second.
    fn scrub_bar(&mut self, ui: &mut egui::Ui) {
        // Space toggles the transport, the way it does everywhere else.
        let toggled = ui.input(|i| i.key_pressed(egui::Key::Space));
        ui.horizontal(|ui| {
            // Text rather than the media-control glyphs: egui's default fonts do
            // not reliably carry U+23F8 and friends, and a button whose label is
            // a missing glyph is an invisible button.
            let playing = self.latest.playing;
            let hover = if playing {
                "stop — whatever is ringing is left to ring (space)"
            } else {
                "play (space)"
            };
            let pressed = ui
                .add_sized(
                    [46.0, 20.0],
                    egui::Button::new(if playing { "stop" } else { "play" }),
                )
                .on_hover_text(hover)
                .clicked();
            if pressed || toggled {
                self.link.send(Command::Playing { value: !playing });
            }
            if ui
                .button("start")
                .on_hover_text("back to the beginning")
                .clicked()
            {
                self.scrub = None;
                self.link.send(Command::Seek { bar: 0.0 });
            }
            let mut bar = self.scrub.unwrap_or(self.latest.bar);
            let response = ui.add_sized(
                [ui.available_width() - 150.0, 18.0],
                Slider::new(&mut bar, 0.0..=self.length).show_value(false),
            );
            if response.drag_started() {
                self.scrub = Some(bar);
            }
            if response.changed() {
                self.scrub = Some(bar);
                self.link.send(Command::Seek { bar });
            }
            if response.drag_stopped() {
                self.scrub = None;
            }
            // Bars are what the music is written in; the clock is what a set
            // feels like the length of.
            let seconds = f64::from(bar) * 4.0 * 60.0 / f64::from(self.latest.bpm.max(1.0));
            ui.label(
                RichText::new(format!(
                    "bar {bar:6.2} / {:.0}   {:.0}:{:04.1}",
                    self.length,
                    (seconds / 60.0).floor(),
                    seconds % 60.0
                ))
                .monospace(),
            );
        });
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.set).strong());
            ui.separator();
            ui.label(
                RichText::new(format!("bar {:7.2}", self.latest.bar))
                    .monospace()
                    .size(16.0),
            );
            ui.separator();
            if ui
                .add(Slider::new(&mut self.bpm, 60.0..=180.0).text("BPM"))
                .changed()
            {
                self.link.send(Command::Bpm { value: self.bpm });
            }
            if ui
                .add(Slider::new(&mut self.master, 0.0..=1.5).text("master"))
                .changed()
            {
                self.link.send(Command::Master { value: self.master });
            }
        });
    }

    /// The numbers that say whether the audio thread is keeping its promises.
    fn health(&mut self, ui: &mut egui::Ui) {
        let load = self.latest.load;
        let colour = if load > 0.7 {
            BAD
        } else if load > 0.4 {
            WARN
        } else {
            GOOD
        };
        ui.horizontal(|ui| {
            ui.label("load");
            ui.add(
                egui::ProgressBar::new(load.min(1.0))
                    .desired_width(140.0)
                    .fill(colour)
                    .text(format!("{:.0}%", load * 100.0)),
            );
            ui.separator();
            ui.label(format!("{} voices", self.latest.voices));
            ui.separator();
            let peak_db = 20.0 * self.latest.peak.max(1e-6).log10();
            ui.label(
                RichText::new(format!("peak {peak_db:6.1} dBFS"))
                    .monospace()
                    .color(if peak_db > -0.5 { BAD } else { GOOD }),
            );
            ui.separator();
            // These two are the ones that matter: either of them moving means
            // something broke a rule.
            let xruns = self.latest.xruns;
            let dropped = self.latest.dropped;
            ui.label(
                RichText::new(format!("dropouts {xruns}")).color(if xruns > 0 {
                    BAD
                } else {
                    GOOD
                }),
            );
            ui.label(
                RichText::new(format!("voices dropped {dropped}")).color(if dropped > 0 {
                    WARN
                } else {
                    GOOD
                }),
            );
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "{} · {} frames",
                    self.link.device, self.link.buffer_frames
                ))
                .weak(),
            );
        });
    }

    fn macros(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for index in 0..self.link.macro_names.len() {
                let name = self.link.macro_names[index];
                let automated = self.macros[index].is_none();
                // Under automation the curve owns the value and the fader
                // follows; touching it takes over.
                let mut value = self.macros[index]
                    .unwrap_or_else(|| self.latest.macros.get(index).copied().unwrap_or(0.0));

                ui.vertical(|ui| {
                    ui.set_width(74.0);
                    let response = ui.add(
                        Slider::new(&mut value, 0.0..=1.0)
                            .vertical()
                            .show_value(false),
                    );
                    if response.changed() {
                        self.macros[index] = Some(value);
                        self.link.send(Command::Macro {
                            index: index as u8,
                            value: Some(value),
                        });
                    }
                    ui.label(RichText::new(format!("{value:.2}")).monospace().size(11.0));
                    ui.label(RichText::new(name).size(12.0));
                    let label = if automated { "auto" } else { "held" };
                    if ui
                        .selectable_label(automated, RichText::new(label).size(10.0))
                        .clicked()
                    {
                        self.macros[index] = if automated { Some(value) } else { None };
                        self.link.send(Command::Macro {
                            index: index as u8,
                            value: self.macros[index],
                        });
                    }
                });
            }
        });
    }

    fn lanes(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("lanes")
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                for index in 0..self.link.lane_names.len() {
                    let name = self.link.lane_names[index];
                    let state = self.latest.lanes[index];

                    if ui
                        .selectable_label(!self.mutes[index], RichText::new(name).size(13.0))
                        .clicked()
                    {
                        self.mutes[index] = !self.mutes[index];
                        self.link.send(Command::Mute {
                            index: index as u8,
                            muted: self.mutes[index],
                        });
                    }

                    let mut level = self.levels[index];
                    if level.is_nan() {
                        level = state.gain;
                    }
                    if ui
                        .add(
                            Slider::new(&mut level, 0.0..=2.0)
                                .show_value(false)
                                .fixed_decimals(2),
                        )
                        .changed()
                    {
                        self.levels[index] = level;
                        self.link.send(Command::Level {
                            index: index as u8,
                            gain: level,
                        });
                    }
                    ui.label(RichText::new(format!("{level:.2}")).monospace().size(11.0));

                    // A meter, not a number: the eye reads a bar faster.
                    ui.add(
                        egui::ProgressBar::new(state.level.clamp(0.0, 1.0).sqrt())
                            .desired_width(120.0)
                            .fill(if state.sounding {
                                GOOD
                            } else {
                                Color32::DARK_GRAY
                            })
                            .text(""),
                    );
                    ui.end_row();
                }
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain();
        // Telemetry arrives whether or not anything was clicked, so the window
        // has to keep asking to be redrawn.
        ui.ctx().request_repaint();

        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            self.transport(ui);
            ui.add_space(2.0);
            self.scrub_bar(ui);
            ui.add_space(2.0);
            self.health(ui);
            ui.separator();
            ui.columns(2, |columns| {
                columns[0].heading("lanes");
                columns[0].add_space(4.0);
                self.lanes(&mut columns[0]);
                columns[0].add_space(10.0);
                columns[0].heading("macros");
                columns[0].add_space(4.0);
                self.macros(&mut columns[0]);

                columns[1].heading("where things are");
                columns[1].add_space(4.0);
                plan::show(&mut columns[1], &self.latest, &self.link.lane_names);
            });
        });
    }
}
