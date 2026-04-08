//! Container format implementations (demuxers and muxers)
//!
//! This crate provides implementations of container format demuxers and muxers:
//! - **WAV** (RIFF WAVE) - ✅ Demuxer + Muxer
//! - **WebM** (Matroska subset) - ✅ Demuxer + Muxer
//! - **MKV** (Matroska) - ✅ Demuxer (shares EBML parser with WebM)
//! - **MP4** (ISO Base Media File Format) - ✅ Demuxer + Muxer
//!
//! All implementations use streaming APIs for bounded memory usage.
//!
//! # Format Detection
//!
//! The `detect` module provides format identification from magic bytes and/or
//! file extensions. Use `detect::detect()` for combined detection.

pub mod detect;
pub mod mkv;
pub mod mp4;
pub mod wav;
pub mod webm;

// Re-export commonly used types
pub use detect::{detect_format, detect_format_by_extension, detect_format_by_path, ContainerFormat};
pub use mkv::MkvDemuxer;
pub use mp4::{Mp4Demuxer, Mp4Muxer};
pub use wav::{WavDemuxer, WavMuxer};
pub use webm::{WebmDemuxer, WebmMuxer};

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
