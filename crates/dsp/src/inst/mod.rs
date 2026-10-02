//! Instruments: one file each, each a `Voice` you construct for a note.

pub mod bass;
pub mod clap;
pub mod cowbell;
pub mod cymbal;
pub mod glass;
pub mod hat;
pub mod kick;
pub mod rim;
pub mod shaker;
pub mod snare;
pub mod strings;

pub use bass::Bass;
pub use clap::Clap;
pub use cowbell::Cowbell;
pub use cymbal::Cymbal;
pub use glass::Glass;
pub use hat::Hat;
pub use kick::Kick;
pub use rim::Rim;
pub use shaker::Shaker;
pub use snare::Snare;
pub use strings::Strings;
