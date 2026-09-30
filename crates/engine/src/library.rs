//! Patches and clips, saved under their own names.
//!
//! A piece is a whole arrangement and there is one of it. A patch and a clip are
//! *material*: worth more than the piece that first needed them, and worth
//! editing on their own. So they live in their own files, and a piece refers to
//! them by name.
//!
//! ```text
//! library/patches/punch.json   an instrument with its parameters
//! library/clips/beat.json      lanes of steps, each on a patch
//! pieces/intro.json            tracks placing clips, macros, a room
//! ```
//!
//! Pieces sit outside the library because a piece is not material: it is the one
//! thing that uses the material.
//!
//! **A reference, and a copy.** A lane names its patch *and* keeps the values
//! inline. That looks redundant and is deliberate: a piece file stays whole and
//! plays with no library present, which matters because a piece is the thing you
//! would send someone. The rule that keeps them from drifting is that **the
//! library wins on load** — a lane naming a patch takes that patch's parameters,
//! whatever the piece remembers — and the copy in the piece is rewritten every
//! time the piece is saved. So the copy is a cache, and changing a patch changes
//! every lane using it.
//!
//! **Save, and save as new.** Saving writes back to the name the thing already
//! has, and everything using that name changes with it. Saving as new writes a
//! different name and repoints only what you were editing. That pair is the
//! whole of how a library grows: you fork a kick, push it around, and the beat
//! that had the old one still has the old one.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::{VERSION, root, write_atomically};
use crate::seq::{Lane, Set, Voicing};

/// A saved instrument: a name and the parameters it was left at.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Patch {
    pub version: u32,
    pub name: String,
    /// Carries the instrument too — `Voicing` is tagged by it.
    #[serde(flatten)]
    pub voicing: Voicing,
}

/// Saved material: the lanes of one clip, without their placement.
///
/// Spans say when a clip plays, which belongs to the piece that places it and
/// not to the clip — the same clip at bar 4 and at bar 26 is one clip. So they
/// are dropped on the way in here, and a lane read back out always plays.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub version: u32,
    pub name: String,
    pub lanes: Vec<Lane>,
}

/// What the library holds, for an index to draw.
#[derive(Clone, Debug, Serialize)]
pub struct Index {
    pub patches: Vec<PatchEntry>,
    pub clips: Vec<ClipEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PatchEntry {
    pub name: String,
    pub instrument: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClipEntry {
    pub name: String,
    pub lanes: usize,
    pub steps: usize,
}

fn patches() -> PathBuf {
    root().join("library/patches")
}

fn clips() -> PathBuf {
    root().join("library/clips")
}

/// A name that is safe as a file name and readable as a title.
///
/// Patch names come from a text box, and a text box can contain a slash. Rather
/// than escape, reject: the caller is a person naming a kick, and telling them
/// to pick another name is kinder than silently saving "drums/punch" as
/// "drums_punch" and then failing to find it.
///
/// # Errors
/// If the name is empty, over 64 characters, or holds anything but letters,
/// digits, spaces, hyphens and underscores.
pub fn check_name(name: &str) -> io::Result<()> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidInput, why.to_string());
    if name.trim().is_empty() {
        return Err(bad("a name is needed"));
    }
    if name.len() > 64 {
        return Err(bad("that name is too long"));
    }
    if name.trim() != name {
        return Err(bad("names cannot start or end with a space"));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || " -_".contains(c))
    {
        return Err(bad("letters, digits, spaces, hyphens and underscores only"));
    }
    Ok(())
}

/// Every `.json` in a directory, by stem, sorted. A missing directory is empty
/// rather than an error: a fresh clone has no library and that is not a problem.
fn names(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

/// Everything saved, for the index.
#[must_use]
pub fn index() -> Index {
    Index {
        patches: names(&patches())
            .into_iter()
            .filter_map(|name| {
                let patch = load_patch(&name).ok()?;
                Some(PatchEntry {
                    name,
                    instrument: patch.voicing.instrument(),
                })
            })
            .collect(),
        clips: names(&clips())
            .into_iter()
            .filter_map(|name| {
                let clip = load_clip(&name).ok()?;
                Some(ClipEntry {
                    name,
                    lanes: clip.lanes.len(),
                    steps: clip
                        .lanes
                        .iter()
                        .map(|lane| lane.pattern.all().len())
                        .max()
                        .unwrap_or(0),
                })
            })
            .collect(),
    }
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<T> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// # Errors
/// If the patch does not exist or cannot be read.
pub fn load_patch(name: &str) -> io::Result<Patch> {
    read(&patches().join(format!("{name}.json")))
}

/// # Errors
/// If the clip does not exist or cannot be read.
pub fn load_clip(name: &str) -> io::Result<Clip> {
    read(&clips().join(format!("{name}.json")))
}

/// # Errors
/// If the name is not usable, or the file cannot be written.
pub fn save_patch(name: &str, voicing: Voicing) -> io::Result<PathBuf> {
    check_name(name)?;
    let path = patches().join(format!("{name}.json"));
    write_atomically(
        &path,
        &Patch {
            version: VERSION,
            name: name.to_string(),
            voicing,
        },
    )?;
    Ok(path)
}

/// # Errors
/// If the name is not usable, or the file cannot be written.
pub fn save_clip(name: &str, lanes: &[Lane]) -> io::Result<PathBuf> {
    check_name(name)?;
    let path = clips().join(format!("{name}.json"));
    let lanes = lanes
        .iter()
        .map(|lane| {
            let mut lane = lane.clone();
            lane.clip = name.to_string();
            // Placement belongs to the piece, not the material.
            lane.spans.clear();
            // Nor does a performance: muting is something you do while playing.
            lane.muted = false;
            lane.velocity_scale = 1.0;
            lane
        })
        .collect();
    write_atomically(
        &path,
        &Clip {
            version: VERSION,
            name: name.to_string(),
            lanes,
        },
    )?;
    Ok(path)
}

/// Give every lane that names a saved patch that patch's parameters.
///
/// This is where "the library wins" actually happens, and it runs once when a
/// piece is loaded. A name with no file behind it is left alone rather than
/// refused: a piece that arrived without its library still plays, from the copy
/// it carries.
///
/// Returns the names it could not find, for whoever wants to say so.
pub fn resolve(set: &mut Set) -> Vec<String> {
    let mut missing = Vec::new();
    for lane in &mut set.lanes {
        let Some(name) = lane.patch.clone() else {
            continue;
        };
        match load_patch(&name) {
            Ok(patch) if patch.voicing.instrument() == lane.voicing.instrument() => {
                lane.voicing = patch.voicing;
            }
            // A patch of the wrong instrument is a name collision, not a patch.
            // Leaving the lane's own voicing alone keeps it playing the thing it
            // was written to play.
            Ok(patch) => missing.push(format!(
                "{name} is a {} patch, but {} plays {}",
                patch.voicing.instrument(),
                lane.name,
                lane.voicing.instrument()
            )),
            Err(_) => missing.push(name),
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_that_would_escape_the_directory_are_refused() {
        for name in [
            "",
            " ",
            "  x",
            "x ",
            "../../etc/passwd",
            "a/b",
            "a.b",
            "a\0b",
        ] {
            assert!(check_name(name).is_err(), "{name:?} should be refused");
        }
        for name in ["punch", "808 tight", "kick-2", "sub_bass", "Öresund"] {
            assert!(check_name(name).is_ok(), "{name:?} should be allowed");
        }
    }

    #[test]
    fn a_missing_library_is_empty_rather_than_an_error() {
        assert!(names(Path::new("/nowhere/at/all")).is_empty());
    }

    #[test]
    fn resolving_leaves_an_unknown_patch_playing() {
        let mut set = crate::sets::by_name("rolling").expect("rolling exists");
        let before = set.lanes[0].voicing;
        set.lanes[0].patch = Some("no such patch".into());

        let missing = resolve(&mut set);
        assert_eq!(missing, vec!["no such patch".to_string()]);
        assert_eq!(
            format!("{:?}", set.lanes[0].voicing),
            format!("{before:?}"),
            "the lane keeps the copy the piece carries"
        );
    }
}
