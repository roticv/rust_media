//! Demuxer trait and related types for reading container formats

use crate::error::Result;
use crate::packet::Packet;
use crate::stream::{ContainerInfo, StreamInfo};
use std::io::{Read, Seek};

/// Trait for demuxing container formats with streaming API
///
/// A demuxer reads a container format (MP4, MKV, WebM, etc.) and extracts
/// compressed packets of media data incrementally. It handles parsing the container
/// structure, stream metadata, and timing information.
///
/// # Streaming Architecture
///
/// The demuxer provides a **streaming API** that processes data incrementally:
/// - Does NOT load the entire file into memory
/// - Returns one packet at a time via `read_packet()`
/// - Supports arbitrarily large media files (multi-GB videos)
/// - Enables real-time processing and low-latency pipelines
///
/// # Example
///
/// ```ignore
/// let mut demuxer = Mp4Demuxer::open("video.mp4")?;
/// let container_info = demuxer.container_info()?;
/// let streams = demuxer.streams()?;
///
/// // Stream packets one at a time - no buffering of entire file
/// while let Ok(packet) = demuxer.read_packet() {
///     // Process packet incrementally
///     // Memory usage is bounded regardless of file size
/// }
/// ```
pub trait Demuxer {
    /// Returns information about the container format
    fn container_info(&self) -> Result<ContainerInfo>;

    /// Returns information about all streams in the container
    fn streams(&self) -> Result<Vec<StreamInfo>>;

    /// Returns information about a specific stream
    fn stream_info(&self, stream_index: usize) -> Result<StreamInfo>;

    /// Reads the next packet from any stream (streaming API)
    ///
    /// This is a streaming operation that returns one packet at a time without
    /// buffering the entire stream. Call repeatedly to process the file incrementally.
    ///
    /// Returns `Error::EndOfStream` when there are no more packets.
    fn read_packet(&mut self) -> Result<Packet>;

    /// Seeks to a specific timestamp (in microseconds)
    ///
    /// The demuxer will seek to the nearest keyframe at or before the
    /// specified timestamp.
    fn seek(&mut self, timestamp_us: i64) -> Result<()>;

    /// Seeks to a specific timestamp in a particular stream
    ///
    /// The timestamp is in the stream's time_base units.
    fn seek_stream(&mut self, stream_index: usize, timestamp: i64) -> Result<()>;

    /// Returns the current position in bytes
    fn position(&self) -> u64;

    /// Returns the total size in bytes (if known)
    fn size(&self) -> Option<u64>;
}

/// Builder for creating demuxers
pub struct DemuxerBuilder<R> {
    reader: R,
    options: DemuxerOptions,
}

impl<R: Read + Seek> DemuxerBuilder<R> {
    /// Creates a new demuxer builder with the given reader
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            options: DemuxerOptions::default(),
        }
    }

    /// Sets whether to analyze the full stream (slower but more accurate)
    pub fn with_full_analysis(mut self, enabled: bool) -> Self {
        self.options.full_analysis = enabled;
        self
    }

    /// Sets the maximum number of bytes to probe
    pub fn with_probe_size(mut self, size: usize) -> Self {
        self.options.probe_size = size;
        self
    }

    /// Returns the reader (consuming the builder)
    pub fn into_reader(self) -> R {
        self.reader
    }

    /// Returns the options
    pub fn options(&self) -> &DemuxerOptions {
        &self.options
    }

    /// Returns a reference to the reader
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// Returns a mutable reference to the reader
    pub fn reader_mut(&mut self) -> &mut R {
        &mut self.reader
    }
}

/// Options for configuring demuxers
#[derive(Debug, Clone)]
pub struct DemuxerOptions {
    /// Whether to perform full stream analysis
    pub full_analysis: bool,

    /// Maximum bytes to probe for format detection
    pub probe_size: usize,

    /// Maximum duration to analyze (in microseconds)
    pub max_analyze_duration: Option<i64>,
}

impl Default for DemuxerOptions {
    fn default() -> Self {
        Self {
            full_analysis: false,
            probe_size: 5 * 1024 * 1024, // 5 MB
            max_analyze_duration: None,
        }
    }
}

/// Format detection result
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatDetection {
    /// MP4/MOV container
    Mp4,
    /// Matroska/WebM container
    Matroska,
    /// Unknown or unsupported format
    Unknown,
}

/// Detects the container format by reading magic bytes
pub fn detect_format<R: Read>(reader: &mut R) -> Result<FormatDetection> {
    let mut magic = [0u8; 12];
    reader.read_exact(&mut magic)?;

    // MP4/MOV: starts with ftyp box
    if magic[4..8] == *b"ftyp" {
        return Ok(FormatDetection::Mp4);
    }

    // Matroska/WebM: starts with EBML signature
    if magic[0..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return Ok(FormatDetection::Matroska);
    }

    Ok(FormatDetection::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_detect_mp4_format() {
        let mp4_header = [
            0x00, 0x00, 0x00, 0x20, // size
            b'f', b't', b'y', b'p', // ftyp
            b'i', b's', b'o', b'm', // brand
        ];
        let mut cursor = Cursor::new(mp4_header);
        let format = detect_format(&mut cursor).unwrap();
        assert_eq!(format, FormatDetection::Mp4);
    }

    #[test]
    fn test_detect_matroska_format() {
        let mkv_header = [
            0x1A, 0x45, 0xDF, 0xA3, // EBML signature
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let mut cursor = Cursor::new(mkv_header);
        let format = detect_format(&mut cursor).unwrap();
        assert_eq!(format, FormatDetection::Matroska);
    }

    #[test]
    fn test_demuxer_options_default() {
        let options = DemuxerOptions::default();
        assert!(!options.full_analysis);
        assert_eq!(options.probe_size, 5 * 1024 * 1024);
    }

    #[test]
    fn test_demuxer_builder() {
        let data = vec![0u8; 100];
        let cursor = Cursor::new(data);
        let builder = DemuxerBuilder::new(cursor)
            .with_full_analysis(true)
            .with_probe_size(1024);

        assert!(builder.options().full_analysis);
        assert_eq!(builder.options().probe_size, 1024);
    }
}
