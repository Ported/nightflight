//! Write the built-in pieces out as documents.
//!
//! The pieces are written in Rust because that is a good way to *derive*
//! material — Bach's figure over his chords is eleven lines of code and a
//! thousand steps of data. Once derived, the data is the thing that gets edited,
//! so this exports it and the generators stay in the repository as the provenance
//! of what they made.
//!
//! It also seeds the library. A clip and a patch are reusable material, and a
//! fresh clone should not have to reverse-engineer its first kick out of a piece
//! before it can save a second one — so every clip in a built-in piece is written
//! to `library/clips`, and every lane's instrument to `library/patches` under the
//! lane's own name. Nothing is overwritten: a patch already saved is left as
//! whoever saved it left it.
//!
//! `cargo run -p engine --example export`

fn main() {
    let directory = engine::document::directory();
    seed_library();
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

/// Write out the material the built-in pieces are made of, without disturbing
/// anything already there.
fn seed_library() {
    let existing = engine::library::index();
    // Taken once and then added to as we go, so a name used by two pieces — the
    // prelude and the intro both have a "voice 1" — is written by the first and
    // reported for the rest. Overwriting would make the seeding order decide
    // what a patch sounds like, which is no way to build a library.
    let mut patches: std::collections::HashSet<_> =
        existing.patches.iter().map(|p| p.name.clone()).collect();
    let mut clips: std::collections::HashSet<_> =
        existing.clips.iter().map(|c| c.name.clone()).collect();
    // Which instruments the library already shows. The library lists patch
    // files, nothing else — so an instrument with no patch is invisible: not in
    // `nf ls`, not in the index, nowhere to audition it from. Tracked here so
    // that after seeding from the pieces, any instrument still uncovered gets a
    // defaults patch below.
    let mut instruments: std::collections::HashSet<&'static str> =
        existing.patches.iter().map(|p| p.instrument).collect();

    for name in engine::sets::NAMES {
        let Some(set) = engine::sets::by_name(name) else {
            continue;
        };
        // One patch per lane, named after the lane. Two lanes of the same name
        // in different pieces would collide, so the first one wins and the rest
        // keep their inline copy — which still plays.
        for lane in &set.lanes {
            if patches.contains(&lane.name) {
                continue;
            }
            match engine::library::save_patch(&lane.name, lane.voicing) {
                Ok(_) => println!("  patch {:>12}  from {name}", lane.name),
                Err(err) => eprintln!("  could not save patch {}: {err}", lane.name),
            }
            patches.insert(lane.name.clone());
            instruments.insert(lane.voicing.instrument());
        }

        for lane in &set.lanes {
            if clips.contains(&lane.clip) {
                continue;
            }
            clips.insert(lane.clip.clone());
            let lanes: Vec<_> = set
                .lanes
                .iter()
                .filter(|other| other.clip == lane.clip)
                .cloned()
                .collect();
            match engine::library::save_clip(&lane.clip, &lanes) {
                Ok(_) => println!(
                    "   clip {:>12}  from {name}, {} lane{}",
                    lane.clip,
                    lanes.len(),
                    if lanes.len() == 1 { "" } else { "s" }
                ),
                Err(err) => eprintln!("  could not save clip {}: {err}", lane.clip),
            }
        }
    }

    // Every instrument the pieces did not cover gets a defaults patch under its
    // own name. The defaults are not a placeholder — they are each instrument's
    // chosen sound (the kick's are its "punch" preset) — and without this a new
    // instrument that no built-in piece plays yet would not exist anywhere a
    // person can see.
    for instrument in engine::seq::Voicing::INSTRUMENTS {
        if instruments.contains(instrument) || patches.contains(*instrument) {
            continue;
        }
        let Some(voicing) = engine::seq::Voicing::fresh(instrument) else {
            continue;
        };
        match engine::library::save_patch(instrument, voicing) {
            Ok(_) => println!("  patch {instrument:>12}  defaults"),
            Err(err) => eprintln!("  could not save patch {instrument}: {err}"),
        }
    }
}
