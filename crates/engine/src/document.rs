//! Pieces on disk.
//!
//! **Who owns what.** The editor owns the document and is the only thing that
//! changes it; the engine only ever reads one and plays it. Which means the
//! engine never serialises, never writes, never answers a question about its own
//! contents, and never needs a path back out across the ring buffers — the three
//! awkward things that would otherwise have to exist.
//!
//! The editor is a browser page and cannot write files, so it is the server that
//! writes on its behalf. The order is: the page sends an edit, the server applies
//! it to the document *and* forwards a live command so it is heard immediately,
//! and a save writes the document down. A level moved during a performance is not
//! an edit and does not touch the document, which is the same distinction every
//! desk makes between playing and authoring.
//!
//! **Why files and not a database.** These are small — a piece is tens of
//! kilobytes and a clip is one — and nothing here queries, joins or writes
//! concurrently. What files give that a database does not: they diff, so a change
//! to a loop is legible in version control; they can be opened in an editor when
//! something is wrong; and they are obvious to anyone who clones this. A database
//! would buy transactions across many objects and indexes over thousands of them,
//! and we have neither problem. If the capture log ever grows large it will be an
//! append-only file, which is also not a database problem.
//!
//! The one thing files do badly is a half-finished write, so every save goes to a
//! temporary file and is renamed over the target, which is atomic.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::seq::Set;

/// The format. Adding a field with a default does not need a new number;
/// changing what a field means does.
pub const VERSION: u32 = 1;

/// A piece as it sits on disk: a name, a format version, and the set itself.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Piece {
    pub version: u32,
    pub name: String,
    #[serde(flatten)]
    pub set: Set,
}

/// The workspace, so saved things land next to the code rather than wherever the
/// program happened to be started from.
#[must_use]
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Where pieces live. Patches and clips are saved separately, under `library`.
#[must_use]
pub fn directory() -> PathBuf {
    root().join("pieces")
}

/// Write any serialisable thing to `path`, atomically.
///
/// Beside the target and renamed over it: a save interrupted halfway leaves the
/// previous file intact rather than half of the new one. Indented, because the
/// whole reason for a file is that a person can read the change it makes.
///
/// # Errors
/// If the value cannot be serialised, or the file cannot be written.
pub fn write_atomically<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.writing");
    std::fs::write(&temporary, text.as_bytes())?;
    std::fs::rename(&temporary, path)
}

/// # Errors
/// If the file cannot be read, is not valid JSON, or was written by a later
/// version of the format.
pub fn load(path: &Path) -> io::Result<Piece> {
    let text = std::fs::read_to_string(path)?;
    let piece: Piece = serde_json::from_str(&text)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if piece.version > VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} was written by version {} of the format and this reads {VERSION}",
                path.display(),
                piece.version
            ),
        ));
    }
    Ok(piece)
}

/// Find a piece by whatever you have: a name, a path, or neither.
///
/// One function because there were three, in the server, the renderer and the
/// library listing, and they had already drifted on which extensions they
/// tried and in what order. What you are iterating on is a file and what you
/// saved is a name; nothing downstream should care which you are holding.
///
/// The order is deliberate. A `.tab` beside a `.json` of the same name wins,
/// because tab is what a person edits and JSON is what a program wrote — if
/// both exist, the one someone typed is the one they meant. A built-in is the
/// last resort, so saving a piece over a generator's name shadows it, which is
/// the point of the generators being provenance rather than sources.
///
/// # Errors
/// If nothing of that name exists anywhere, or what does cannot be read.
pub fn find(what: &str) -> io::Result<(String, Set)> {
    let given = Path::new(what);
    let candidates: Vec<PathBuf> = if given.extension().is_some() {
        vec![given.to_path_buf()]
    } else {
        let here = directory();
        vec![here.join(format!("{what}.tab")), here.join(format!("{what}.json"))]
    };

    for path in &candidates {
        if !path.is_file() {
            continue;
        }
        if path.extension().is_some_and(|e| e == "tab") {
            let text = std::fs::read_to_string(path)?;
            return crate::tab::read(&text)
                .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, why));
        }
        let piece = load(path)?;
        return Ok((piece.name, piece.set));
    }

    if given.extension().is_none()
        && let Some(set) = crate::sets::by_name(what)
    {
        return Ok((what.to_string(), set));
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "no piece called {what:?}. Tried {}, and the built-ins {:?}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            crate::sets::NAMES
        ),
    ))
}

/// Write a piece, atomically.
///
/// # Errors
/// If the directory cannot be created or the file cannot be written.
pub fn save(path: &Path, name: &str, set: &Set) -> io::Result<()> {
    write_atomically(
        path,
        &Piece {
            version: VERSION,
            name: name.to_string(),
            set: set.clone(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name is not a path and a path is not a name, and `find` is the one
    /// place that has to know. It used to be three places, which had already
    /// drifted on which extensions they tried and in what order.
    #[test]
    fn a_piece_is_found_by_name_or_by_path() {
        let (name, set) = find("rolling").expect("rolling is in pieces/");
        assert_eq!(name, "rolling");
        assert!(!set.lanes.is_empty());

        let path = directory().join("rolling.json");
        let (_, same) = find(path.to_str().expect("utf-8")).expect("by path too");
        assert_eq!(same.lanes.len(), set.lanes.len());
    }

    #[test]
    fn a_tab_beside_a_json_wins() {
        // Tab is what a person edits and JSON is what a program wrote; if both
        // exist, the one someone typed is the one they meant.
        let stem = "__find_precedence_test__";
        let json = directory().join(format!("{stem}.json"));
        let tab = directory().join(format!("{stem}.tab"));
        let (_, mut set) = find("rolling").expect("something to copy");
        set.bpm = 126.0;
        save(&json, stem, &set).expect("write the json");
        set.bpm = 100.0;
        std::fs::write(&tab, crate::tab::write(stem, &set)).expect("write the tab");

        let found = find(stem).map(|(_, s)| s.bpm);
        // Clean up before asserting, so a failure does not leave litter in
        // pieces/ for the next run to trip over.
        let _ = std::fs::remove_file(&tab);
        let json_only = find(stem).map(|(_, s)| s.bpm);
        let _ = std::fs::remove_file(&json);

        assert_eq!(found.expect("found with both"), 100.0, "the tab should win");
        assert_eq!(json_only.expect("found with one"), 126.0, "then the json");
    }

    #[test]
    fn a_name_that_is_nothing_says_where_it_looked() {
        let why = find("no-such-piece").expect_err("nothing of that name").to_string();
        assert!(why.contains("no-such-piece"), "{why}");
        assert!(why.contains(".tab"), "{why}");
        assert!(why.contains("rolling"), "should list the built-ins: {why}");
    }
}
