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

/// Where pieces live, next to the workspace rather than wherever the program was
/// started from.
#[must_use]
pub fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../pieces")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("pieces"))
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

/// Write a piece, atomically.
///
/// # Errors
/// If the directory cannot be created or the file cannot be written.
pub fn save(path: &Path, name: &str, set: &Set) -> io::Result<()> {
    let piece = Piece {
        version: VERSION,
        name: name.to_string(),
        set: set.clone(),
    };
    // Indented, because the whole reason for a file is that a person can read
    // the change it makes.
    let text = serde_json::to_string_pretty(&piece)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Written beside the target and renamed over it: a save interrupted halfway
    // leaves the previous piece intact rather than half of the new one.
    let temporary = path.with_extension("json.writing");
    std::fs::write(&temporary, text.as_bytes())?;
    std::fs::rename(&temporary, path)
}
