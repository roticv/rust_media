//! Encoder trait and related types for encoding media data

use crate::error::Result;
use crate::frame::Frame;
use crate::packet::Packet;
use crate::stream::StreamInfo;

/// Trait for encoding uncompressed media data with streaming API
///
/// An encoder takes uncompressed frames and produces compressed packets incrementally.
/// It handles codec-specific compression (H.264, VP9, AV1, AAC, Opus, etc.)
/// and may buffer frames internally for GOP (Group of Pictures) structure.
///
/// # Streaming Architecture
///
/// The encoder provides a **streaming API** using the send/receive pattern
/// (symmetric to the decoder):
/// - `send_frame()`: Feed frames incrementally to the encoder
/// - `receive_packet()`: Pull encoded packets one at a time
/// - Does NOT require all frames to be loaded before encoding can start
/// - Bounded memory usage - only buffers what's needed (e.g., B-frame lookahead)
/// - Enables real-time encoding and low-latency pipelines
///
/// # Example
///
/// ```ignore
/// let mut encoder = H264Encoder::new(config)?;
///
/// // Stream frames to encoder incrementally
/// encoder.send_frame(&frame)?;
///
/// // Stream packets from encoder one at a time
/// while let Ok(packet) = encoder.receive_packet() {
///     // Write packet as soon as it's available
///     // No need to wait for entire stream to be encoded
/// }
///
/// // Flush remaining packets at end of stream
/// encoder.flush()?;
/// while let Ok(packet) = encoder.receive_packet() {
///     // Process remaining buffered packets
/// }
/// ```
pub trait Encoder {
    /// Returns the codec name (e.g., "h264", "vp9", "aac")
    fn codec(&self) -> &str;

    /// Returns the stream information this encoder will produce
    fn stream_info(&self) -> &StreamInfo;

    /// Sends a frame to the encoder for processing (streaming API)
    ///
    /// This is part of the streaming send/receive pattern. The encoder processes
    /// frames incrementally without requiring the entire stream to be loaded.
    /// The encoder may buffer frames internally for GOP structure or lookahead.
    ///
    /// Call `receive_packet()` to retrieve encoded packets as they become available.
    ///
    /// Returns `Error::NeedMoreData` if the encoder needs more frames
    /// before it can produce a packet.
    fn send_frame(&mut self, frame: &Frame) -> Result<()>;

    /// Receives an encoded packet from the encoder (streaming API)
    ///
    /// This is part of the streaming send/receive pattern. Returns packets one at
    /// a time as soon as they're available, enabling incremental processing.
    ///
    /// Returns `Error::NeedMoreData` if the encoder needs more frames
    /// before it can produce another packet. Call `send_frame()` to provide more data.
    ///
    /// Returns `Error::EndOfStream` if all packets have been retrieved
    /// after flushing.
    fn receive_packet(&mut self) -> Result<Packet>;

    /// Flushes the encoder, signaling end of input
    ///
    /// After calling flush, continue calling `receive_packet()` until
    /// it returns `Error::EndOfStream` to retrieve all remaining packets.
    fn flush(&mut self) -> Result<()>;

    /// Resets the encoder to its initial state
    ///
    /// Clears all internal buffers and state. Use this when starting a new
    /// encode session.
    fn reset(&mut self) -> Result<()>;

    /// Returns whether the encoder is in a flushed state
    fn is_flushed(&self) -> bool;

    /// Returns encoder capabilities
    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities::default()
    }
}

/// Encoder capabilities and features
#[derive(Debug, Clone, Default)]
pub struct EncoderCapabilities {
    /// Whether the encoder supports hardware acceleration
    pub hardware_acceleration: bool,

    /// Whether the encoder supports B-frames
    pub b_frames: bool,

    /// Maximum lookahead frames
    pub max_lookahead: Option<usize>,

    /// Supported pixel/sample formats for input
    pub input_formats: Vec<String>,

    /// Whether the encoder requires specific frame alignment
    pub requires_alignment: bool,
}

/// Configuration for creating encoders
#[derive(Debug, Clone)]
pub struct EncoderConfig {
    /// Target codec
    pub codec: String,

    /// Target bitrate (bits per second)
    pub bitrate: Option<u64>,

    /// Encoder-specific options
    pub options: EncoderOptions,
}

impl EncoderConfig {
    /// Creates a new encoder configuration
    pub fn new(codec: String) -> Self {
        Self {
            codec,
            bitrate: None,
            options: EncoderOptions::default(),
        }
    }

    /// Builder method to set bitrate
    pub fn with_bitrate(mut self, bitrate: u64) -> Self {
        self.bitrate = Some(bitrate);
        self
    }

    /// Builder method to set options
    pub fn with_options(mut self, options: EncoderOptions) -> Self {
        self.options = options;
        self
    }
}

/// Options for configuring encoders
#[derive(Debug, Clone, Default)]
pub struct EncoderOptions {
    /// Whether to use hardware acceleration if available
    pub hardware_acceleration: bool,

    /// Number of threads to use (0 = auto)
    pub threads: usize,

    /// Quality/CRF value (codec-dependent)
    pub quality: Option<f32>,

    /// Preset (e.g., "ultrafast", "medium", "slow")
    pub preset: Option<String>,

    /// GOP size (distance between keyframes)
    pub gop_size: Option<usize>,

    /// Maximum B-frames
    pub max_b_frames: Option<usize>,

    /// Custom encoder-specific options
    pub custom: Vec<(String, String)>,
}

impl EncoderOptions {
    /// Creates new encoder options
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

    /// Builder method to set quality
    pub fn with_quality(mut self, quality: f32) -> Self {
        self.quality = Some(quality);
        self
    }

    /// Builder method to set preset
    pub fn with_preset(mut self, preset: String) -> Self {
        self.preset = Some(preset);
        self
    }

    /// Builder method to set GOP size
    pub fn with_gop_size(mut self, gop_size: usize) -> Self {
        self.gop_size = Some(gop_size);
        self
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

    #[test]
    fn test_encoder_config() {
        let config = EncoderConfig::new("h264".to_string())
            .with_bitrate(5_000_000);

        assert_eq!(config.codec, "h264");
        assert_eq!(config.bitrate, Some(5_000_000));
    }

    #[test]
    fn test_encoder_options_builder() {
        let options = EncoderOptions::new()
            .with_hardware_acceleration(true)
            .with_threads(4)
            .with_quality(23.0)
            .with_preset("medium".to_string())
            .with_gop_size(250);

        assert!(options.hardware_acceleration);
        assert_eq!(options.threads, 4);
        assert_eq!(options.quality, Some(23.0));
        assert_eq!(options.preset, Some("medium".to_string()));
        assert_eq!(options.gop_size, Some(250));
    }
}
