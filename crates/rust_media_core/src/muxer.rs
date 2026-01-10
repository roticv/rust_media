//! Muxer trait and related types for writing container formats

use crate::error::Result;
use crate::packet::Packet;
use crate::stream::StreamInfo;
use std::io::Write;

/// Trait for muxing container formats with streaming API
///
/// A muxer writes compressed packets to a container format (MP4, MKV, WebM, etc.)
/// incrementally. It handles creating the container structure, interleaving streams,
/// and writing timing information.
///
/// # Streaming Architecture
///
/// The muxer provides a **streaming API** that writes data incrementally:
/// - Does NOT buffer the entire output in memory
/// - Writes packets one at a time via `write_packet()`
/// - Supports arbitrarily large output files (multi-GB videos)
/// - Enables real-time muxing and low-latency pipelines
///
/// # Example
///
/// ```ignore
/// let mut muxer = Mp4Muxer::create("output.mp4")?;
///
/// // Add streams
/// muxer.add_stream(video_stream_info)?;
/// muxer.add_stream(audio_stream_info)?;
///
/// // Write header
/// muxer.write_header()?;
///
/// // Stream packets one at a time - no buffering of entire file
/// for packet in packets {
///     muxer.write_packet(&packet)?;
///     // Packet is written immediately to disk
/// }
///
/// // Write trailer and close
/// muxer.write_trailer()?;
/// ```
pub trait Muxer {
    /// Adds a stream to the container
    ///
    /// Must be called before `write_header()`.
    /// Returns the stream index.
    fn add_stream(&mut self, stream_info: StreamInfo) -> Result<usize>;

    /// Returns information about all streams
    fn streams(&self) -> &[StreamInfo];

    /// Writes the container header
    ///
    /// Must be called after all streams are added and before writing packets.
    fn write_header(&mut self) -> Result<()>;

    /// Writes a packet to the container (streaming API)
    ///
    /// This is a streaming operation that writes the packet immediately without
    /// buffering the entire output. The muxer may interleave packets from different
    /// streams to maintain proper timing.
    ///
    /// Must be called after `write_header()` and before `write_trailer()`.
    fn write_packet(&mut self, packet: &Packet) -> Result<()>;

    /// Writes the container trailer and finalizes the file
    ///
    /// Must be called after all packets are written. After this call, no more
    /// packets can be written.
    fn write_trailer(&mut self) -> Result<()>;

    /// Flushes any buffered data to the underlying writer
    fn flush(&mut self) -> Result<()>;

    /// Returns the current position in bytes
    fn position(&self) -> u64;
}

/// Builder for creating muxers
pub struct MuxerBuilder<W> {
    writer: W,
    options: MuxerOptions,
}

impl<W: Write> MuxerBuilder<W> {
    /// Creates a new muxer builder with the given writer
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            options: MuxerOptions::default(),
        }
    }

    /// Sets whether to write timestamps in absolute time
    pub fn with_absolute_timestamps(mut self, enabled: bool) -> Self {
        self.options.absolute_timestamps = enabled;
        self
    }

    /// Sets the maximum interleave duration
    pub fn with_max_interleave_delta(mut self, delta_us: i64) -> Self {
        self.options.max_interleave_delta = Some(delta_us);
        self
    }

    /// Returns the writer (consuming the builder)
    pub fn into_writer(self) -> W {
        self.writer
    }

    /// Returns the options
    pub fn options(&self) -> &MuxerOptions {
        &self.options
    }

    /// Returns a reference to the writer
    pub fn writer(&self) -> &W {
        &self.writer
    }

    /// Returns a mutable reference to the writer
    pub fn writer_mut(&mut self) -> &mut W {
        &mut self.writer
    }
}

/// Options for configuring muxers
#[derive(Debug, Clone)]
pub struct MuxerOptions {
    /// Whether to use absolute timestamps
    pub absolute_timestamps: bool,

    /// Maximum interleave delta in microseconds
    pub max_interleave_delta: Option<i64>,

    /// Whether to write seeking information
    pub write_seeking_info: bool,

    /// Custom format-specific options
    pub custom: Vec<(String, String)>,
}

impl Default for MuxerOptions {
    fn default() -> Self {
        Self {
            absolute_timestamps: false,
            max_interleave_delta: None,
            write_seeking_info: true,
            custom: Vec::new(),
        }
    }
}

impl MuxerOptions {
    /// Creates new muxer options
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder method to add custom option
    pub fn with_custom_option(mut self, key: String, value: String) -> Self {
        self.custom.push((key, value));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_muxer_options_default() {
        let options = MuxerOptions::default();
        assert!(!options.absolute_timestamps);
        assert!(options.write_seeking_info);
        assert_eq!(options.max_interleave_delta, None);
    }

    #[test]
    fn test_muxer_builder() {
        let data = Vec::new();
        let cursor = Cursor::new(data);
        let builder = MuxerBuilder::new(cursor)
            .with_absolute_timestamps(true)
            .with_max_interleave_delta(1000000);

        assert!(builder.options().absolute_timestamps);
        assert_eq!(builder.options().max_interleave_delta, Some(1000000));
    }
}
