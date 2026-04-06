//! rust_media - A Rust-based media processing framework
//!
//! This library provides core abstractions for media processing with **streaming APIs**:
//! - Container-level (Packet) and bit-level (Frame) data structures
//! - Demuxing and muxing for container formats (MP4, MKV, WebM)
//! - Decoding and encoding for codecs (H.264, VP9, AV1, AAC, Opus)
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

pub mod types;
pub mod packet;
pub mod frame;
pub mod error;
pub mod stream;
pub mod demuxer;
pub mod decoder;
pub mod encoder;
pub mod muxer;
pub mod reorder;

// Re-export commonly used types
pub use packet::Packet;
pub use frame::Frame;
pub use types::{MediaType, PixelFormat, SampleFormat, ColorSpace, ColorRange};
pub use error::{Error, Result};
pub use stream::{StreamInfo, StreamParams, VideoStreamParams, AudioStreamParams, ContainerInfo};
pub use demuxer::{Demuxer, DemuxerBuilder, DemuxerOptions, FormatDetection};
pub use decoder::{Decoder, DecoderConfig, DecoderOptions, DecoderCapabilities, CodecId};
pub use encoder::{Encoder, EncoderConfig, EncoderOptions, EncoderCapabilities};
pub use muxer::{Muxer, MuxerBuilder, MuxerOptions};
pub use reorder::FrameReorderBuffer;
