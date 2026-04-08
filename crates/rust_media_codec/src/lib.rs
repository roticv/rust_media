//! Codec implementations (decoders and encoders)
//!
//! This crate provides implementations of media codecs:
//!
//! # Video Codecs
//! - **VP8** - ✅ Implemented (via libvpx)
//! - **VP9** - ✅ Implemented (via libvpx)
//! - **H.264/AVC** - ✅ Decoder implemented (via rust_h264, pure Rust, MIT/Apache-2.0)
//! - **H.264/AVC** - ✅ Decoder implemented (via VideoToolbox, macOS only) - All profiles
//! - **H.264/AVC** - ✅ Encoder implemented (via x264, requires `gpl-x264` feature)
//! - **H.265/HEVC** - ✅ Decoder implemented (via VideoToolbox, requires `videotoolbox` feature)
//! - **AV1** - ✅ Decoder implemented (via dav1d, requires libdav1d)
//!
//! # Audio Codecs
//! - **PCM** (raw audio) - ✅ Implemented
//! - **Opus** - ✅ Implemented (via libopus)
//! - **AAC** - ✅ Encoder and decoder implemented (via libfdk-aac, requires `fdk-aac` feature)
//! - **MP3** - ✅ Decoder implemented (via minimp3)
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
//! - `videotoolbox`: Enables hardware-accelerated H.264 decoding on macOS (all profiles)
//!
//! **Warning**: Enabling `gpl-x264` changes the license of compiled binaries to GPL.

pub mod audio;
pub mod video;

// Re-export commonly used codecs
pub use audio::mp3::Mp3Decoder;
pub use audio::opus::{OpusDecoder, OpusEncoder};
pub use audio::pcm::{PcmDecoder, PcmEncoder};
pub use video::av1::Av1Decoder;
pub use video::h264::H264Decoder;
pub use video::vp8::{Vp8Decoder, Vp8Encoder};
pub use video::vp9::{Vp9Decoder, Vp9Encoder};

// Optional feature codecs
#[cfg(feature = "gpl-x264")]
pub use video::x264::X264Encoder;

#[cfg(feature = "fdk-aac")]
pub use audio::fdk_aac::{FdkAacDecoder, FdkAacEncoder};

// VideoToolbox hardware acceleration (macOS only)
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use video::videotoolbox::VideoToolboxH264Decoder;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use video::videotoolbox::VideoToolboxHevcDecoder;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
