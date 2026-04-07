//! Filter implementations for the rust_media ecosystem
//!
//! Provides frame-level processing filters that sit between decoder and encoder
//! in the media pipeline:
//!
//! ```text
//! Demuxer → Decoder → [Filters] → Encoder → Muxer
//! ```
//!
//! # Available Filters
//!
//! ## Audio
//! - **aresample** - Resample audio to a different sample rate
//!
//! ## Video
//! - **ssim** - Compute SSIM quality metric between two video streams
//!
//! # Filter Graph Parsing
//!
//! Filters can be specified using FFmpeg-like syntax:
//! ```text
//! filter_name=param1=value1:param2=value2
//! ```
//! Multiple filters chain with commas: `filter1,filter2=param=value`

pub mod audio;
pub mod graph;
pub mod video;

pub use graph::{Filter, FilterGraph};
