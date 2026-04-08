//! MKV (Matroska) container format support
//!
//! Matroska is a flexible, extensible container format that supports:
//! - **Video**: H.264, H.265/HEVC, VP8, VP9, AV1, MPEG-4, and more
//! - **Audio**: AAC, MP3, FLAC, AC3, Opus, Vorbis, PCM, and more
//! - Subtitles, chapters, attachments, metadata
//!
//! WebM is a subset of Matroska designed for web use. Both formats share
//! the same EBML (Extensible Binary Meta Language) container structure,
//! so the underlying parser is reused.
//!
//! # Supported Codecs (decode)
//!
//! - **Video**: H.264 (V_MPEG4/ISO/AVC), H.265/HEVC (V_MPEGH/ISO/HEVC),
//!   VP8 (V_VP8), VP9 (V_VP9), AV1 (V_AV1)
//! - **Audio**: Opus (A_OPUS), Vorbis (A_VORBIS), AAC (A_AAC), MP3 (A_MPEG/L3),
//!   FLAC (A_FLAC), AC3 (A_AC3), PCM (A_PCM/INT/LIT, A_PCM/INT/BIG)

use crate::webm::WebmDemuxer;

/// MKV (Matroska) demuxer
///
/// MKV and WebM share the same EBML/Matroska container structure, so this is
/// a type alias for `WebmDemuxer` which now supports both formats and a wider
/// range of codec IDs.
///
/// # Example
///
/// ```rust,ignore
/// use rust_media_format::mkv::MkvDemuxer;
/// use std::fs::File;
/// use std::io::BufReader;
///
/// let file = File::open("video.mkv")?;
/// let mut demuxer = MkvDemuxer::open(BufReader::new(file))?;
/// let streams = demuxer.streams()?;
/// ```
pub type MkvDemuxer<R> = WebmDemuxer<R>;
