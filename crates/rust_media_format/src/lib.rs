//! Container format implementations (demuxers and muxers)
//!
//! This crate provides implementations of container format demuxers and muxers:
//! - **WAV** (RIFF WAVE) - ✅ Demuxer + Muxer
//! - **WebM** (Matroska subset) - ✅ Demuxer (Muxer planned)
//! - MP4/MOV (ISO Base Media File Format) - Planned
//! - MKV (Matroska) - Planned
//!
//! All implementations use streaming APIs for bounded memory usage.

pub mod wav;
pub mod webm;

// Placeholder modules for future implementations
// pub mod mp4;
// pub mod mkv;

// Re-export commonly used demuxers and muxers
pub use wav::{WavDemuxer, WavMuxer};
pub use webm::WebmDemuxer;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
