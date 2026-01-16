//! Video codec implementations

pub mod vp8;
pub mod vp9;

// GPL-licensed codecs (optional features)
#[cfg(feature = "gpl-x264")]
pub mod x264;
