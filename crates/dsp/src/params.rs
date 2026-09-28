//! Instrument parameters, declared once.
//!
//! A parameter has to be three things at once: a field to compute with, a
//! default, and a description an interface can draw a fader from — a name, a
//! range, a unit, and whether it wants a logarithmic scale. Written by hand that
//! is the same knowledge in three places, and the third copy is the one that
//! silently goes stale, because nothing fails to compile when a range is wrong.
//!
//! So it is written once and the three are generated. The declaration reads as
//! close to a table as Rust allows:
//!
//! ```ignore
//! parameters! {
//!     /// What the instrument is.
//!     pub struct Params {
//!         /// Seconds: how fast the pitch falls. The "doom".
//!         pitch_decay: log 0.005..=0.3 = 0.03, "s";
//!         /// Level of the beater transient.
//!         click: lin 0.0..=1.0 = 0.35, "";
//!     }
//! }
//! ```
//!
//! Logarithmic is not decoration. A cutoff between 20 Hz and 8 kHz on a linear
//! fader spends nine tenths of its travel above 800 Hz, where the ear hears
//! almost no change; on a logarithmic one every octave gets equal room, which is
//! how pitch actually works. The rule is simple: anything measured in hertz or
//! seconds is logarithmic, anything that is a depth or a mix is not.

use serde::{Deserialize, Serialize};

/// What an interface needs to draw one fader.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ParamSpec {
    /// The field's own name, which is also its name on the wire.
    pub name: &'static str,
    /// The doc comment from the declaration, shown as a tooltip.
    pub doc: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// Whether the fader should move in octaves rather than in steps.
    pub logarithmic: bool,
    /// "s", "Hz", "cents", or empty for a bare number.
    pub unit: &'static str,
}

/// An instrument's parameters, addressable by index.
///
/// By index rather than by name because a command crosses to the audio thread,
/// where a string would mean an allocation. The server turns a name into an
/// index; the index is what travels.
pub trait Parameters: Copy + Default {
    const SPEC: &'static [ParamSpec];

    fn get(&self, index: usize) -> f32;
    fn set(&mut self, index: usize, value: f32);
}

#[doc(hidden)]
#[macro_export]
macro_rules! __parameter_scale {
    (lin) => {
        false
    };
    (log) => {
        true
    };
}

/// Declare an instrument's parameters. See the module documentation.
#[macro_export]
macro_rules! parameters {
    (
        $(#[$struct_doc:meta])*
        pub struct $name:ident {
            $(
                $(#[doc = $doc:literal])*
                $field:ident : $scale:ident $min:literal ..= $max:literal = $default:literal, $unit:literal;
            )*
        }
    ) => {
        $(#[$struct_doc])*
        #[derive(Clone, Copy, Debug, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(default)]
        pub struct $name {
            $(
                $(#[doc = $doc])*
                pub $field: f32,
            )*
        }

        impl Default for $name {
            fn default() -> Self {
                Self { $($field: $default,)* }
            }
        }

        impl $crate::params::Parameters for $name {
            const SPEC: &'static [$crate::params::ParamSpec] = &[
                $($crate::params::ParamSpec {
                    name: stringify!($field),
                    doc: concat!($($doc),*),
                    min: $min,
                    max: $max,
                    default: $default,
                    logarithmic: $crate::__parameter_scale!($scale),
                    unit: $unit,
                },)*
            ];

            fn get(&self, index: usize) -> f32 {
                [$(self.$field),*].get(index).copied().unwrap_or(0.0)
            }

            fn set(&mut self, index: usize, value: f32) {
                let mut at = 0;
                $(
                    if at == index {
                        // Clamped to the declared range, so nothing an interface
                        // can send makes an instrument misbehave.
                        self.$field = value.clamp($min, $max);
                    }
                    at += 1;
                )*
                let _ = at;
            }
        }
    };
}
