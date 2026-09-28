//! Wave shaping.

/// Soft clipping, normalised so a full-scale peak stays full scale.
///
/// On headphones this matters more than it looks: they can barely move air at
/// 49 Hz, but the harmonics that distortion adds at 100-300 Hz let the ear
/// rebuild the missing fundamental, so a driven kick sounds bigger.
#[must_use]
pub fn saturate(x: f32, drive: f32) -> f32 {
    (drive * x).tanh() / drive.tanh()
}
