//! Notice when a file changes, and play the change.
//!
//! The loop this exists for: a tab file is written, the server picks it up,
//! and the music changes under whoever is listening without them touching
//! anything. That is the difference between a tool and the thing we are
//! actually trying to build, and it is about forty lines.
//!
//! **Polling, not filesystem events.** A `notify` dependency would report
//! changes sooner and bring platform quirks with it — editors that write via
//! rename, several events per save, directories that need re-registering. A
//! `stat` of a few dozen files every quarter second costs nothing measurable
//! and has none of that: a rename looks exactly like a write, which matters
//! because `document::save` is a temp file and a rename.
//!
//! **Settled, not just changed.** A file is reloaded only once its size and
//! mtime have held still for a poll. A shell redirect is not atomic, and
//! reading a tab halfway through being written is how you get a piece with
//! four lanes missing and no idea why.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::Session;

/// How often to look. Fast enough that saving a file and hearing it feels
/// like one action; slow enough to be free.
const EVERY: Duration = Duration::from_millis(250);

type Stamps = BTreeMap<PathBuf, (SystemTime, u64)>;

/// Watch the places music is kept, for as long as the program runs.
pub fn spawn(session: Arc<Mutex<Session>>) {
    std::thread::Builder::new()
        .name("watch".into())
        .spawn(move || {
            let mut known = scan();
            // What changed last time round but has not held still yet.
            let mut moving: Stamps = Stamps::new();
            loop {
                std::thread::sleep(EVERY);
                let now = scan();
                if now == known {
                    continue;
                }

                let settled: Vec<PathBuf> = now
                    .iter()
                    .filter(|(path, stamp)| {
                        known.get(*path) != Some(*stamp) && moving.get(*path) == Some(*stamp)
                    })
                    .map(|(path, _)| path.clone())
                    .collect();
                // Gone is gone; there is nothing half-written about a deletion.
                let removed: Vec<PathBuf> = known
                    .keys()
                    .filter(|path| !now.contains_key(*path))
                    .cloned()
                    .collect();

                moving = now
                    .iter()
                    .filter(|(path, stamp)| known.get(*path) != Some(*stamp))
                    .map(|(path, stamp)| ((*path).clone(), *stamp))
                    .collect();

                if settled.is_empty() && removed.is_empty() {
                    continue;
                }
                for path in &settled {
                    known.insert(path.clone(), now[path]);
                }
                for path in &removed {
                    known.remove(path);
                }
                react(&session, &settled, &removed);
            }
        })
        .expect("a watcher thread");
}

/// Every file that could change what is heard, with enough of it to tell
/// whether it moved.
fn scan() -> Stamps {
    let mut out = Stamps::new();
    let root = engine::document::root();
    for dir in [
        engine::document::directory(),
        root.join("library/patches"),
        root.join("library/clips"),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|e| e == "tab" || e == "json")
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(when) = meta.modified() else { continue };
            out.insert(path, (when, meta.len()));
        }
    }
    out
}

fn react(session: &Mutex<Session>, settled: &[PathBuf], removed: &[PathBuf]) {
    let mut session = session.lock().expect("no panics hold this");
    let playing = session.name.clone();

    // Was it the piece that is on? Its patches count too: a lane naming a
    // patch takes that patch's parameters, so editing the patch changes what
    // is playing just as surely as editing the piece.
    let touches_current = settled
        .iter()
        .chain(removed)
        .any(|path| is_piece(path, &playing) || is_library(path));

    if touches_current {
        match session.reload() {
            Ok(()) => println!("reloaded {playing}"),
            // A typo in a tab file must not stop the music. The engine keeps
            // playing what it had, and the reason goes to the page.
            Err(why) => {
                eprintln!("{playing} did not reload: {why}");
                session.trouble = Some(why);
            }
        }
    }
    // Either way the list of what exists may have moved, and the page redraws
    // from a generation counter rather than from a message per file.
    session.generation += 1;
}

fn is_piece(path: &Path, name: &str) -> bool {
    path.parent() == Some(engine::document::directory().as_path())
        && path.file_stem().and_then(|s| s.to_str()) == Some(name)
}

fn is_library(path: &Path) -> bool {
    path.parent()
        .and_then(|p| p.parent())
        .is_some_and(|p| p.ends_with("library"))
}
