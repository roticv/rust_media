//! Video codec implementations

pub mod vp8;
pub mod vp9;
pub mod openh264;

// GPL-licensed codecs (optional features)
#[cfg(feature = "gpl-x264")]
pub mod x264;

// VideoToolbox hardware acceleration (macOS only)
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub mod videotoolbox;
