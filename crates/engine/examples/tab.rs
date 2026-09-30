//! Print a piece as tab.
//!
//! ```text
//! cargo run -p engine --example tab -- intro
//! ```
//!
//! A stop-gap until `nf describe` exists, and the reason the format was
//! written: `intro.json` is 252 kB and costs about sixty-three thousand tokens
//! to read. The same piece as tab is fourteen.

fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "rolling".into());
    let path = engine::document::directory().join(format!("{name}.json"));
    let mut set = match engine::document::load(&path) {
        Ok(piece) => piece.set,
        Err(_) => match engine::sets::by_name(&name) {
            Some(set) => set,
            None => {
                eprintln!("no piece named {name:?}; have {:?}", engine::sets::NAMES);
                std::process::exit(1);
            }
        },
    };
    for missing in engine::library::resolve(&mut set) {
        eprintln!("patch not found: {missing}");
    }
    print!("{}", engine::tab::write(&name, &set));
}
