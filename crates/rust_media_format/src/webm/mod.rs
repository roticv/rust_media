//! WebM container format support
//!
//! WebM is a subset of Matroska designed for web use, typically containing:
//! - Video: VP8, VP9, AV1
//! - Audio: Vorbis, Opus

pub mod demuxer;
pub mod ebml;

pub use demuxer::WebmDemuxer;
