//! Codec implementations (decoders and encoders)
//!
//! This crate provides implementations of media codecs:
//!
//! # Video Codecs
//! - H.264/AVC (planned)
//! - VP9 (planned)
//! - AV1 (planned)
//! - H.265/HEVC (future)
//! - VP8 (future)
//!
//! # Audio Codecs
//! - **PCM** (raw audio) - ✅ Implemented
//! - **Opus** - ✅ Implemented
//! - AAC (LC, HE, HEv2) (planned)
//! - MP3 (future)
//! - Vorbis (future)
//! - FLAC (future)
//!
//! All implementations use streaming APIs for bounded memory usage.

pub mod audio;

// Placeholder for video codecs
// pub mod video;

// Re-export commonly used codecs
pub use audio::opus::{OpusDecoder, OpusEncoder};
pub use audio::pcm::{PcmDecoder, PcmEncoder};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
