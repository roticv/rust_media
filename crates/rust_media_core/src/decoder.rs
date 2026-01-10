//! Decoder trait and related types for decoding compressed media

use crate::error::Result;
use crate::frame::Frame;
use crate::packet::Packet;
use crate::stream::StreamInfo;
use std::str::FromStr;

/// Trait for decoding compressed media data with streaming API
///
/// A decoder takes compressed packets and produces uncompressed frames incrementally.
/// It handles codec-specific decompression (H.264, VP9, AV1, AAC, Opus, etc.)
/// and may buffer packets internally for reordering or reference frames.
///
/// # Streaming Architecture
///
/// The decoder provides a **streaming API** using the send/receive pattern:
/// - `send_packet()`: Feed packets incrementally to the decoder
/// - `receive_frame()`: Pull decoded frames one at a time
/// - Does NOT require all packets to be loaded before decoding can start
/// - Bounded memory usage - only buffers what's needed for decoding (e.g., reference frames)
/// - Enables real-time decoding and low-latency pipelines
///
/// # Example
///
/// ```ignore
/// let mut decoder = H264Decoder::new(stream_info)?;
///
/// // Stream packets to decoder incrementally
/// decoder.send_packet(&packet)?;
///
/// // Stream frames from decoder one at a time
/// while let Ok(frame) = decoder.receive_frame() {
///     // Process frame as soon as it's available
///     // No need to wait for entire file to be decoded
/// }
///
/// // Flush remaining frames at end of stream
/// decoder.flush()?;
/// while let Ok(frame) = decoder.receive_frame() {
///     // Process remaining buffered frames
/// }
/// ```
pub trait Decoder {
    /// Returns the codec name (e.g., "h264", "vp9", "aac")
    fn codec(&self) -> &str;

    /// Returns the stream information this decoder was configured with
    fn stream_info(&self) -> &StreamInfo;

    /// Sends a packet to the decoder for processing (streaming API)
    ///
    /// This is part of the streaming send/receive pattern. The decoder processes
    /// packets incrementally without requiring the entire stream to be loaded.
    /// The decoder may buffer packets internally for reordering (B-frames) or
    /// reference frame storage.
    ///
    /// Call `receive_frame()` to retrieve decoded frames as they become available.
    ///
    /// Returns `Error::NeedMoreData` if the decoder needs more packets
    /// before it can produce a frame.
    fn send_packet(&mut self, packet: &Packet) -> Result<()>;

    /// Receives a decoded frame from the decoder (streaming API)
    ///
    /// This is part of the streaming send/receive pattern. Returns frames one at
    /// a time as soon as they're available, enabling incremental processing.
    ///
    /// Returns `Error::NeedMoreData` if the decoder needs more packets
    /// before it can produce another frame. Call `send_packet()` to provide more data.
    ///
    /// Returns `Error::EndOfStream` if all frames have been retrieved
    /// after flushing.
    fn receive_frame(&mut self) -> Result<Frame>;

    /// Flushes the decoder, signaling end of input
    ///
    /// After calling flush, continue calling `receive_frame()` until
    /// it returns `Error::EndOfStream` to retrieve all remaining frames.
    fn flush(&mut self) -> Result<()>;

    /// Resets the decoder to its initial state
    ///
    /// Clears all internal buffers and state. Use this when seeking
    /// or starting a new decode session.
    fn reset(&mut self) -> Result<()>;

    /// Returns whether the decoder is in a flushed state
    fn is_flushed(&self) -> bool;

    /// Returns decoder capabilities
    fn capabilities(&self) -> DecoderCapabilities {
        DecoderCapabilities::default()
    }
}

/// Decoder capabilities and features
#[derive(Debug, Clone, Default)]
pub struct DecoderCapabilities {
    /// Whether the decoder supports hardware acceleration
    pub hardware_acceleration: bool,

    /// Whether the decoder can decode frames out of order
    pub reordering: bool,

    /// Maximum number of reference frames needed
    pub max_ref_frames: Option<usize>,

    /// Supported pixel/sample formats for output
    pub output_formats: Vec<String>,

    /// Whether the decoder requires extra data (e.g., SPS/PPS)
    pub requires_extra_data: bool,
}

/// Configuration for creating decoders
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    /// Stream information
    pub stream_info: StreamInfo,

    /// Decoder-specific options
    pub options: DecoderOptions,
}

impl DecoderConfig {
    /// Creates a new decoder configuration
    pub fn new(stream_info: StreamInfo) -> Self {
        Self {
            stream_info,
            options: DecoderOptions::default(),
        }
    }

    /// Builder method to set options
    pub fn with_options(mut self, options: DecoderOptions) -> Self {
        self.options = options;
        self
    }
}

/// Options for configuring decoders
#[derive(Debug, Clone, Default)]
pub struct DecoderOptions {
    /// Whether to use hardware acceleration if available
    pub hardware_acceleration: bool,

    /// Number of threads to use (0 = auto)
    pub threads: usize,

    /// Whether to enable low-latency mode
    pub low_latency: bool,

    /// Output pixel/sample format preference
    pub output_format: Option<String>,

    /// Custom decoder-specific options
    pub custom: Vec<(String, String)>,
}

impl DecoderOptions {
    /// Creates new decoder options
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder method to enable hardware acceleration
    pub fn with_hardware_acceleration(mut self, enabled: bool) -> Self {
        self.hardware_acceleration = enabled;
        self
    }

    /// Builder method to set thread count
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = threads;
        self
    }

    /// Builder method to enable low-latency mode
    pub fn with_low_latency(mut self, enabled: bool) -> Self {
        self.low_latency = enabled;
        self
    }

    /// Builder method to set output format
    pub fn with_output_format(mut self, format: String) -> Self {
        self.output_format = Some(format);
        self
    }

    /// Builder method to add custom option
    pub fn with_custom_option(mut self, key: String, value: String) -> Self {
        self.custom.push((key, value));
        self
    }
}

/// Codec identification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecId {
    // Video codecs
    H264,
    H265,
    VP8,
    VP9,
    AV1,
    MPEG4,
    MPEG2,
    JPEG,
    PNG,
    HEVC,

    // Audio codecs
    AAC,
    MP3,
    Opus,
    Vorbis,
    FLAC,
    PCM,

    // Unknown
    Unknown,
}

impl CodecId {
    /// Returns the codec name as a string
    pub fn as_str(&self) -> &str {
        match self {
            CodecId::H264 => "h264",
            CodecId::H265 => "h265",
            CodecId::VP8 => "vp8",
            CodecId::VP9 => "vp9",
            CodecId::AV1 => "av1",
            CodecId::MPEG4 => "mpeg4",
            CodecId::MPEG2 => "mpeg2",
            CodecId::JPEG => "jpeg",
            CodecId::PNG => "png",
            CodecId::HEVC => "hevc",
            CodecId::AAC => "aac",
            CodecId::MP3 => "mp3",
            CodecId::Opus => "opus",
            CodecId::Vorbis => "vorbis",
            CodecId::FLAC => "flac",
            CodecId::PCM => "pcm",
            CodecId::Unknown => "unknown",
        }
    }

    /// Parses a codec ID from a string
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "h264" | "avc" | "avc1" => CodecId::H264,
            "h265" | "hevc" | "hev1" | "hvc1" => CodecId::H265,
            "vp8" => CodecId::VP8,
            "vp9" | "vp09" => CodecId::VP9,
            "av1" | "av01" => CodecId::AV1,
            "mpeg4" | "mp4v" => CodecId::MPEG4,
            "mpeg2" | "mp2v" => CodecId::MPEG2,
            "jpeg" | "mjpeg" => CodecId::JPEG,
            "png" => CodecId::PNG,
            "aac" | "mp4a" => CodecId::AAC,
            "mp3" => CodecId::MP3,
            "opus" => CodecId::Opus,
            "vorbis" => CodecId::Vorbis,
            "flac" => CodecId::FLAC,
            "pcm" => CodecId::PCM,
            _ => CodecId::Unknown,
        }
    }

    /// Returns whether this is a video codec
    pub fn is_video(&self) -> bool {
        matches!(
            self,
            CodecId::H264
                | CodecId::H265
                | CodecId::VP8
                | CodecId::VP9
                | CodecId::AV1
                | CodecId::MPEG4
                | CodecId::MPEG2
                | CodecId::JPEG
                | CodecId::PNG
                | CodecId::HEVC
        )
    }

    /// Returns whether this is an audio codec
    pub fn is_audio(&self) -> bool {
        matches!(
            self,
            CodecId::AAC
                | CodecId::MP3
                | CodecId::Opus
                | CodecId::Vorbis
                | CodecId::FLAC
                | CodecId::PCM
        )
    }
}

impl FromStr for CodecId {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(CodecId::parse(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_id_from_str() {
        assert_eq!(CodecId::parse("h264"), CodecId::H264);
        assert_eq!(CodecId::parse("avc1"), CodecId::H264);
        assert_eq!(CodecId::parse("vp9"), CodecId::VP9);
        assert_eq!(CodecId::parse("av01"), CodecId::AV1);
        assert_eq!(CodecId::parse("aac"), CodecId::AAC);
        assert_eq!(CodecId::parse("opus"), CodecId::Opus);

        // Test FromStr trait
        assert_eq!("h264".parse::<CodecId>().unwrap(), CodecId::H264);
        assert_eq!("vp9".parse::<CodecId>().unwrap(), CodecId::VP9);
    }

    #[test]
    fn test_codec_id_as_str() {
        assert_eq!(CodecId::H264.as_str(), "h264");
        assert_eq!(CodecId::VP9.as_str(), "vp9");
        assert_eq!(CodecId::AAC.as_str(), "aac");
    }

    #[test]
    fn test_codec_id_is_video() {
        assert!(CodecId::H264.is_video());
        assert!(CodecId::VP9.is_video());
        assert!(!CodecId::AAC.is_video());
        assert!(!CodecId::Opus.is_video());
    }

    #[test]
    fn test_codec_id_is_audio() {
        assert!(CodecId::AAC.is_audio());
        assert!(CodecId::Opus.is_audio());
        assert!(!CodecId::H264.is_audio());
        assert!(!CodecId::VP9.is_audio());
    }

    #[test]
    fn test_decoder_options_builder() {
        let options = DecoderOptions::new()
            .with_hardware_acceleration(true)
            .with_threads(4)
            .with_low_latency(true)
            .with_output_format("yuv420p".to_string());

        assert!(options.hardware_acceleration);
        assert_eq!(options.threads, 4);
        assert!(options.low_latency);
        assert_eq!(options.output_format, Some("yuv420p".to_string()));
    }
}
