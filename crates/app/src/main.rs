//! Night Flight Live: one window, one audio thread.
//!
//! The audio thread owns the engine and has a hard deadline. The window owns
//! nothing but its own widgets and talks to the engine through two lock-free
//! queues. `assert_no_alloc` enforces the first half of that in debug builds, by
//! making any trip to the allocator inside the callback abort the program
//! instead of quietly stealing time from it.

mod panels;
mod plan;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: host::AllocDisabler = host::AllocDisabler;

fn main() -> eframe::Result {
    let name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "intro".to_string());
    let Some(set) = engine::sets::by_name(&name) else {
        eprintln!("no set named {name:?}; have {:?}", engine::sets::NAMES);
        std::process::exit(1);
    };

    // Read before the set is handed over: the window needs to know how far the
    // scrub bar reaches.
    let length = set.length_bars;

    let link = match host::start(set) {
        Ok(link) => link,
        Err(err) => {
            eprintln!("could not open the audio device: {err}");
            std::process::exit(1);
        }
    };
    println!("{} · {} · {} frames", name, link.device, link.buffer_frames);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Night Flight Live")
            .with_inner_size([1040.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Night Flight Live",
        options,
        Box::new(move |_| Ok(Box::new(panels::App::new(link, name, length)))),
    )
}
