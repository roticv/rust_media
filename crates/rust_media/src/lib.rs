//! rust_media - A Rust-based media processing framework
//!
//! This is the main library crate that re-exports all components from the rust_media ecosystem.
//! For most users, this is the only crate you need to depend on.
//!
//! # Crate Organization
//!
//! - **rust_media_core**: Core types (Packet, Frame) and traits (Demuxer, Decoder, Encoder, Muxer)
//! - **rust_media_format**: Container format implementations (MP4, MKV, WebM)
//! - **rust_media_codec**: Codec implementations (H.264, VP9, AV1, AAC, Opus)
//! - **rust_media**: This crate - convenience re-exports
//! - **rust_media_cli**: Command-line tool (binary)
//!
//! # Streaming Architecture
//!
//! All APIs are designed for streaming operation with bounded memory usage:
//! - **Demuxer**: Streams packets one at a time from containers
//! - **Decoder**: send/receive pattern for incremental decoding
//! - **Encoder**: send/receive pattern for incremental encoding
//! - **Muxer**: Writes packets one at a time to containers
//!
//! This enables processing of arbitrarily large files and real-time pipelines.
//!
//! # Example
//!
//! ```ignore
//! use rust_media::prelude::*;
//!
//! // Open input file
//! let mut demuxer = Mp4Demuxer::open("input.mp4")?;
//! let streams = demuxer.streams()?;
//!
//! // Create decoder for video stream
//! let video_stream = &streams[0];
//! let mut decoder = H264Decoder::new(video_stream)?;
//!
//! // Process streaming
//! while let Ok(packet) = demuxer.read_packet() {
//!     decoder.send_packet(&packet)?;
//!
//!     while let Ok(frame) = decoder.receive_frame() {
//!         // Process frame
//!     }
//! }
//! ```

// Re-export core types and traits
pub use rust_media_core::*;

// Re-export format implementations
pub use rust_media_format as format;

// Re-export codec implementations
pub use rust_media_codec as codec;

/// Convenience prelude for common imports
pub mod prelude {
    pub use rust_media_core::{
        Decoder, DecoderConfig, DecoderOptions, Demuxer, DemuxerBuilder, DemuxerOptions, Encoder,
        EncoderConfig, EncoderOptions, Error, Frame, Muxer, MuxerBuilder, MuxerOptions, Packet,
        Result,
    };
}
