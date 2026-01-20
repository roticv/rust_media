//! Codec implementations (decoders and encoders)
//!
//! This crate provides implementations of media codecs:
//!
//! # Video Codecs
//! - **VP8** - ✅ Implemented (via libvpx)
//! - **VP9** - ✅ Implemented (via libvpx)
//! - **H.264/AVC** - ✅ Decoder implemented (via OpenH264, BSD-2-Clause)
//! - **H.264/AVC** - ✅ Encoder implemented (via x264, requires `gpl-x264` feature)
//! - AV1 (planned)
//! - H.265/HEVC (future)
//!
//! # Audio Codecs
//! - **PCM** (raw audio) - ✅ Implemented
//! - **Opus** - ✅ Implemented (via libopus)
//! - **AAC** - ✅ Encoder and decoder implemented (via libfdk-aac, requires `fdk-aac` feature)
//! - MP3 (future)
//! - Vorbis (future)
//! - FLAC (future)
//!
//! All implementations use streaming APIs for bounded memory usage.
//!
//! # Optional Features
//!
//! Some codecs require external libraries and are gated behind optional features:
//!
//! - `gpl-x264`: Enables H.264 encoding via x264 (GPL v2+)
//! - `fdk-aac`: Enables AAC encoding and decoding via libfdk-aac (Fraunhofer FDK AAC License)
//!
//! **Warning**: Enabling `gpl-x264` changes the license of compiled binaries to GPL.

pub mod audio;
pub mod video;

// Re-export commonly used codecs
pub use audio::opus::{OpusDecoder, OpusEncoder};
pub use audio::pcm::{PcmDecoder, PcmEncoder};
pub use video::openh264::H264Decoder;
pub use video::vp8::{Vp8Decoder, Vp8Encoder};
pub use video::vp9::{Vp9Decoder, Vp9Encoder};

// Optional feature codecs
#[cfg(feature = "gpl-x264")]
pub use video::x264::X264Encoder;

#[cfg(feature = "fdk-aac")]
pub use audio::fdk_aac::{FdkAacDecoder, FdkAacEncoder};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
