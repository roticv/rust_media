//! Video codec implementations

pub mod av1;
pub mod h264;
pub mod vp8;
pub mod vp9;

// GPL-licensed codecs (optional features)
#[cfg(feature = "gpl-x264")]
pub mod x264;

// VideoToolbox hardware acceleration (macOS only)
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub mod videotoolbox;
