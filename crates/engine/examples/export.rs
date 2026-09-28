//! Write the built-in pieces out as documents.
//!
//! The pieces are written in Rust because that is a good way to *derive*
//! material — Bach's figure over his chords is eleven lines of code and a
//! thousand steps of data. Once derived, the data is the thing that gets edited,
//! so this exports it and the generators stay in the repository as the provenance
//! of what they made.
//!
//! `cargo run -p engine --example export`

fn main() {
    let directory = engine::document::directory();
    for name in engine::sets::NAMES {
        let Some(set) = engine::sets::by_name(name) else {
            continue;
        };
        let path = directory.join(format!("{name}.json"));
        match engine::document::save(&path, name, &set) {
            Ok(()) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!(
                    "{:>8}  {:>7} bytes  {} lanes, {} macros",
                    name,
                    size,
                    set.lanes.len(),
                    set.macros.len()
                );
            }
            Err(err) => eprintln!("could not write {}: {err}", path.display()),
        }
    }
}
