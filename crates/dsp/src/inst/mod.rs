//! Instruments: one file each, each a `Voice` you construct for a note.

pub mod bass;
pub mod glass;
pub mod hat;
pub mod kick;
pub mod snare;
pub mod strings;

pub use bass::Bass;
pub use glass::Glass;
pub use hat::Hat;
pub use kick::Kick;
pub use snare::Snare;
pub use strings::Strings;
