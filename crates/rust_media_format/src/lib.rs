//! Container format implementations (demuxers and muxers)
//!
//! This crate provides implementations of container format demuxers and muxers:
//! - **WAV** (RIFF WAVE) - ✅ Demuxer + Muxer
//! - **WebM** (Matroska subset) - ✅ Demuxer + Muxer
//! - **MP4** (ISO Base Media File Format) - ✅ Muxer
//! - MKV (Matroska) - Planned
//!
//! All implementations use streaming APIs for bounded memory usage.

pub mod mp4;
pub mod wav;
pub mod webm;

// Placeholder modules for future implementations
// pub mod mkv;

// Re-export commonly used demuxers and muxers
pub use mp4::Mp4Muxer;
pub use wav::{WavDemuxer, WavMuxer};
pub use webm::{WebmDemuxer, WebmMuxer};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
