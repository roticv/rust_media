//! Codec implementations (decoders and encoders)
//!
//! This crate provides implementations of media codecs:
//!
//! # Video Codecs
//! - **VP8** - ✅ Implemented (via libvpx)
//! - **VP9** - ✅ Implemented (via libvpx)
//! - **H.264/AVC** - ✅ Encoder implemented (via x264, requires `gpl-x264` feature)
//! - AV1 (planned)
//! - H.265/HEVC (future)
//!
//! # Audio Codecs
//! - **PCM** (raw audio) - ✅ Implemented
//! - **Opus** - ✅ Implemented (via libopus)
//! - AAC (LC, HE, HEv2) (planned)
//! - MP3 (future)
//! - Vorbis (future)
//! - FLAC (future)
//!
//! All implementations use streaming APIs for bounded memory usage.
//!
//! # GPL-Licensed Features
//!
//! Some codecs require GPL-licensed libraries and are gated behind optional features:
//!
//! - `gpl-x264`: Enables H.264 encoding via x264 (GPL v2+)
//!
//! **Warning**: Enabling GPL features changes the license of compiled binaries to GPL.

pub mod audio;
pub mod video;

// Re-export commonly used codecs
pub use audio::opus::{OpusDecoder, OpusEncoder};
pub use audio::pcm::{PcmDecoder, PcmEncoder};
pub use video::vp8::{Vp8Decoder, Vp8Encoder};
pub use video::vp9::{Vp9Decoder, Vp9Encoder};

// GPL-licensed codecs (optional features)
#[cfg(feature = "gpl-x264")]
pub use video::x264::X264Encoder;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
