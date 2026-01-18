//! MP4 container format support (ISO Base Media File Format)
//!
//! MP4 is based on ISO 14496-12 (ISO Base Media File Format) and is
//! the most widely used container format for video distribution.
//!
//! Supported codecs:
//! - Video: H.264/AVC (avc1), VP9 (vp09)
//! - Audio: AAC (mp4a), Opus (Opus)

pub mod boxes;
pub mod demuxer;
pub mod muxer;
pub mod reader;
pub mod writer;

pub use demuxer::Mp4Demuxer;
pub use muxer::Mp4Muxer;
