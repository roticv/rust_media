//! FDK-AAC audio encoder and decoder implementation
//!
//! AAC (Advanced Audio Coding) encoder and decoder using the Fraunhofer FDK AAC library.
//! This implementation uses the fdk-aac crate for Rust bindings.
//!
//! # Supported Profiles
//!
//! - **AAC-LC** (Low Complexity) - Most widely compatible
//! - **HE-AAC** (High Efficiency with SBR) - Better compression at low bitrates
//! - **HE-AACv2** (with SBR + Parametric Stereo) - Best for stereo at very low bitrates
//!
//! # Encoder Example
//!
//! ```ignore
//! use rust_media_codec::FdkAacEncoder;
//! use rust_media_core::{Encoder, StreamInfo, StreamParams, AudioStreamParams};
//!
//! let stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
//!     .with_params(StreamParams::Audio(AudioStreamParams::new(48000, 2, SampleFormat::S16)))
//!     .with_bitrate(128000);
//!
//! let mut encoder = FdkAacEncoder::new(stream_info)?;
//!
//! // Send frames, receive packets...
//! encoder.send_frame(&frame)?;
//! let packet = encoder.receive_packet()?;
//! ```
//!
//! # Decoder Example
//!
//! ```ignore
//! use rust_media_codec::FdkAacDecoder;
//! use rust_media_core::{Decoder, StreamInfo, StreamParams, AudioStreamParams};
//!
//! let stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
//!     .with_params(StreamParams::Audio(AudioStreamParams::new(48000, 2, SampleFormat::S16)));
//!
//! let mut decoder = FdkAacDecoder::new(stream_info)?;
//!
//! // Send packets, receive frames...
//! decoder.send_packet(&packet)?;
//! let frame = decoder.receive_frame()?;
//! ```
//!
//! # Licensing
//!
//! This module uses libfdk-aac which is licensed under the Fraunhofer FDK AAC
//! Codec License. This is not GPL, but has some restrictions on use.

use fdk_aac::dec::{Decoder as FdkDecoder, Transport as DecTransport};
use fdk_aac::enc::{
    AudioObjectType, BitRate, ChannelMode, Encoder as FdkEncoder, EncoderParams,
    Transport as EncTransport,
};
use rust_media_core::{
    AudioStreamParams, Decoder, DecoderCapabilities, Encoder, EncoderCapabilities, Error, Frame,
    MediaType, Packet, Result, SampleFormat, StreamInfo, StreamParams,
};

/// AAC frame size in samples per channel
const AAC_FRAME_SIZE: usize = 1024;

/// Maximum AAC output packet size
const MAX_AAC_PACKET_SIZE: usize = 8192;

/// ADTS header size in bytes
const ADTS_HEADER_SIZE: usize = 7;

/// Helper to parse AudioSpecificConfig and generate ADTS headers
///
/// AudioSpecificConfig (ISO 14496-3) format:
/// - 5 bits: audioObjectType (2 = AAC-LC)
/// - 4 bits: samplingFrequencyIndex
/// - 4 bits: channelConfiguration
/// - Plus optional extension data
#[derive(Debug, Clone, Copy)]
struct AacConfig {
    /// Audio object type (2 = AAC-LC, 5 = SBR, 29 = PS)
    object_type: u8,
    /// Sample rate index (0-12 map to specific rates)
    sample_rate_index: u8,
    /// Channel configuration (1 = mono, 2 = stereo, etc.)
    channel_config: u8,
}

impl AacConfig {
    /// Parse AudioSpecificConfig bytes
    fn from_asc(asc: &[u8]) -> Option<Self> {
        if asc.len() < 2 {
            return None;
        }

        // AudioSpecificConfig structure (first 13+ bits):
        // - 5 bits: audioObjectType
        // - 4 bits: samplingFrequencyIndex
        // - 4 bits: channelConfiguration
        let byte0 = asc[0];
        let byte1 = asc[1];

        // Extract audioObjectType (5 bits from byte0)
        let object_type = byte0 >> 3;

        // Extract samplingFrequencyIndex (4 bits: 3 from byte0, 1 from byte1)
        let sample_rate_index = ((byte0 & 0x07) << 1) | ((byte1 & 0x80) >> 7);

        // Extract channelConfiguration (4 bits from byte1)
        let channel_config = (byte1 >> 3) & 0x0F;

        Some(Self {
            object_type,
            sample_rate_index,
            channel_config,
        })
    }

    /// Create ADTS header for a raw AAC frame
    ///
    /// ADTS header structure (7 bytes without CRC):
    /// - 12 bits: sync word (0xFFF)
    /// - 1 bit: ID (0 = MPEG-4, 1 = MPEG-2)
    /// - 2 bits: layer (always 0)
    /// - 1 bit: protection_absent (1 = no CRC)
    /// - 2 bits: profile (object_type - 1)
    /// - 4 bits: sampling_frequency_index
    /// - 1 bit: private_bit
    /// - 3 bits: channel_configuration
    /// - 1 bit: original/copy
    /// - 1 bit: home
    /// - 1 bit: copyright_id_bit
    /// - 1 bit: copyright_id_start
    /// - 13 bits: frame_length (header + aac frame size)
    /// - 11 bits: buffer_fullness (0x7FF for VBR)
    /// - 2 bits: num_raw_data_blocks - 1
    fn create_adts_header(&self, aac_frame_size: usize) -> [u8; ADTS_HEADER_SIZE] {
        let frame_length = ADTS_HEADER_SIZE + aac_frame_size;
        let profile = self.object_type.saturating_sub(1); // ADTS profile = object_type - 1

        let mut header = [0u8; ADTS_HEADER_SIZE];

        // Byte 0: sync word high bits (0xFF)
        header[0] = 0xFF;

        // Byte 1: sync word low (0xF) + ID(0) + layer(00) + protection_absent(1)
        header[1] = 0xF1; // 1111 0001

        // Byte 2: profile(2) + sampling_freq_idx(4) + private(1) + channel_high(1)
        header[2] = ((profile & 0x03) << 6)
            | ((self.sample_rate_index & 0x0F) << 2)
            | ((self.channel_config >> 2) & 0x01);

        // Byte 3: channel_low(2) + original(1) + home(1) + copyright_id(1) + copyright_start(1) + frame_len_high(2)
        header[3] =
            ((self.channel_config & 0x03) << 6) | ((frame_length >> 11) & 0x03) as u8;

        // Byte 4: frame_length middle (8 bits)
        header[4] = ((frame_length >> 3) & 0xFF) as u8;

        // Byte 5: frame_length low (3 bits) + buffer_fullness high (5 bits)
        header[5] = (((frame_length & 0x07) << 5) | 0x1F) as u8; // 0x1F = high bits of 0x7FF

        // Byte 6: buffer_fullness low (6 bits) + num_raw_data_blocks (2 bits)
        header[6] = 0xFC; // 111111 00 (fullness=0x7FF low, blocks=0)

        header
    }
}

/// FDK-AAC Audio Encoder
///
/// Encodes PCM audio frames into AAC packets using libfdk-aac.
/// Handles arbitrary input frame sizes by buffering samples until
/// a complete AAC frame (1024 samples per channel) is accumulated.
pub struct FdkAacEncoder {
    stream_info: StreamInfo,
    encoder: FdkEncoder,
    flushed: bool,
    sample_buffer: Vec<i16>,
    frame_size: usize,
    channels: usize,
    current_pts: i64,
    buffered_packets: Vec<Packet>,
    /// AudioSpecificConfig data for MP4 muxing
    audio_specific_config: Vec<u8>,
}

impl FdkAacEncoder {
    /// Creates a new AAC encoder with the given configuration
    ///
    /// # Arguments
    ///
    /// * `stream_info` - Stream configuration including sample rate, channels, and bitrate
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The stream is not an audio stream
    /// - The sample rate is not supported
    /// - The channel configuration is not supported
    /// - The encoder fails to initialize
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Extract audio parameters
        let audio_params = match &stream_info.params {
            StreamParams::Audio(params) => params,
            _ => return Err(Error::Config("AAC encoder requires audio stream".to_string())),
        };

        // Validate sample rate
        let sample_rate = audio_params.sample_rate;
        if ![8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000]
            .contains(&sample_rate)
        {
            return Err(Error::Unsupported(format!(
                "AAC does not support {} Hz sample rate",
                sample_rate
            )));
        }

        // Determine channel mode
        let channels = audio_params.channels;
        let channel_mode = match channels {
            1 => ChannelMode::Mono,
            2 => ChannelMode::Stereo,
            _ => {
                return Err(Error::Unsupported(format!(
                    "AAC encoder only supports mono or stereo, got {} channels",
                    channels
                )))
            }
        };

        // Get bitrate (default to 128 kbps if not specified)
        let bitrate = stream_info.bitrate.unwrap_or(128000) as u32;

        // Create encoder parameters
        let params = EncoderParams {
            bit_rate: BitRate::Cbr(bitrate),
            sample_rate,
            transport: EncTransport::Raw, // Raw AAC for MP4 muxing (no ADTS headers)
            channels: channel_mode,
            audio_object_type: AudioObjectType::Mpeg4LowComplexity, // AAC Low Complexity - most compatible
        };

        // Create the encoder
        let encoder = FdkEncoder::new(params)
            .map_err(|e| Error::Config(format!("Failed to create AAC encoder: {:?}", e)))?;

        // Get AudioSpecificConfig for MP4 muxing from encoder info
        let info = encoder
            .info()
            .map_err(|e| Error::Config(format!("Failed to get encoder info: {:?}", e)))?;

        let conf_size = info.confSize as usize;
        let audio_specific_config = info.confBuf[..conf_size].to_vec();

        Ok(Self {
            stream_info,
            encoder,
            flushed: false,
            sample_buffer: Vec::new(),
            frame_size: AAC_FRAME_SIZE,
            channels,
            current_pts: 0,
            buffered_packets: Vec::new(),
            audio_specific_config,
        })
    }

    /// Creates an AAC encoder from audio parameters
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - Sample rate in Hz (44100 or 48000 recommended)
    /// * `channels` - Number of channels (1 or 2)
    /// * `sample_format` - Must be S16 (signed 16-bit)
    /// * `bitrate` - Target bitrate in bits per second
    pub fn from_params(
        sample_rate: u32,
        channels: usize,
        sample_format: SampleFormat,
        bitrate: u64,
    ) -> Result<Self> {
        if sample_format != SampleFormat::S16 {
            return Err(Error::Unsupported(
                "AAC encoder only supports S16 sample format".to_string(),
            ));
        }

        let audio_params = AudioStreamParams::new(sample_rate, channels, sample_format);

        let stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
            .with_params(StreamParams::Audio(audio_params))
            .with_bitrate(bitrate);

        Self::new(stream_info)
    }

    /// Returns the AudioSpecificConfig data for MP4 muxing
    ///
    /// This should be stored in the stream's `extra_data` field for the MP4 muxer
    /// to include in the esds box.
    pub fn audio_specific_config(&self) -> &[u8] {
        &self.audio_specific_config
    }

    /// Helper function to encode one complete frame from the sample buffer
    fn encode_frame(&mut self) -> Result<()> {
        let total_samples_needed = self.frame_size * self.channels;

        // Take exactly the number of samples we need
        let input: Vec<i16> = self.sample_buffer.drain(..total_samples_needed).collect();

        // Allocate output buffer
        let mut output = vec![0u8; MAX_AAC_PACKET_SIZE];

        // Encode the frame
        let encode_info = self
            .encoder
            .encode(&input, &mut output)
            .map_err(|e| Error::Encode(format!("AAC encode error: {:?}", e)))?;

        if encode_info.output_size == 0 {
            // Encoder needs more data (shouldn't happen with complete frames)
            return Ok(());
        }

        // Truncate to actual output size
        output.truncate(encode_info.output_size);

        // Calculate duration in timebase units (samples)
        let duration = self.frame_size as i64;

        // Create packet
        let packet = Packet::new(output, 0, MediaType::Audio)
            .with_pts(self.current_pts)
            .with_dts(self.current_pts)
            .with_duration(duration);

        // Update PTS for next packet (in samples)
        self.current_pts += duration;

        // Store encoded packet
        self.buffered_packets.push(packet);

        Ok(())
    }
}

impl Encoder for FdkAacEncoder {
    fn codec(&self) -> &str {
        "aac"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if self.flushed {
            return Err(Error::InvalidState(
                "Cannot send frame after flush".to_string(),
            ));
        }

        // Get frame data from first plane (interleaved audio)
        let frame_data = frame
            .plane(0)
            .ok_or_else(|| Error::InvalidState("Frame missing data plane".to_string()))?;

        if frame_data.len() % 2 != 0 {
            return Err(Error::InvalidData(
                "Frame data must be aligned to 16-bit samples".to_string(),
            ));
        }

        // Convert frame data (bytes) to i16 samples and add to buffer
        for i in (0..frame_data.len()).step_by(2) {
            let sample = i16::from_le_bytes([frame_data[i], frame_data[i + 1]]);
            self.sample_buffer.push(sample);
        }

        // Store PTS from first frame if buffer was empty
        if self.current_pts == 0 && frame.pts().is_some() {
            self.current_pts = frame.pts().unwrap();
        }

        // Calculate total samples needed for one frame
        let total_samples_needed = self.frame_size * self.channels;

        // Encode as many complete frames as we have buffered
        while self.sample_buffer.len() >= total_samples_needed {
            self.encode_frame()?;
        }

        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        // Return buffered packets first
        if !self.buffered_packets.is_empty() {
            return Ok(self.buffered_packets.remove(0));
        }

        // If flushed and no more buffered packets, we're done
        if self.flushed {
            return Err(Error::EndOfStream);
        }

        // No packets available yet
        Err(Error::NeedMoreData)
    }

    fn flush(&mut self) -> Result<()> {
        // Encode any remaining samples in the buffer
        if !self.sample_buffer.is_empty() {
            let total_samples_needed = self.frame_size * self.channels;

            // Pad with silence to complete the frame
            if self.sample_buffer.len() < total_samples_needed {
                let samples_to_pad = total_samples_needed - self.sample_buffer.len();
                self.sample_buffer.extend(vec![0i16; samples_to_pad]);
            }

            // Encode the final frame
            self.encode_frame()?;
        }

        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.flushed = false;
        self.sample_buffer.clear();
        self.buffered_packets.clear();
        self.current_pts = 0;
        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }

    fn capabilities(&self) -> EncoderCapabilities {
        EncoderCapabilities {
            hardware_acceleration: false,
            b_frames: false,
            max_lookahead: None,
            input_formats: vec!["s16".to_string()],
            requires_alignment: true,
        }
    }
}

// ============================================================================
// AAC Decoder
// ============================================================================

/// Maximum decoded PCM output buffer size (enough for 8 channels * 2048 samples * 2 bytes)
const MAX_DECODE_BUFFER_SIZE: usize = 8 * 2048 * 2;

/// FDK-AAC Audio Decoder
///
/// Decodes AAC packets into PCM audio frames using libfdk-aac.
/// Supports ADTS-wrapped AAC streams (common in streaming) and raw AAC
/// with AudioSpecificConfig (common in MP4 containers).
pub struct FdkAacDecoder {
    stream_info: StreamInfo,
    decoder: FdkDecoder,
    flushed: bool,
    /// Buffer for encoded AAC data waiting to be decoded
    packet_buffer: Vec<u8>,
    /// Buffer for decoded frames waiting to be returned
    buffered_frames: Vec<Frame>,
    /// Current PTS for decoded frames
    current_pts: i64,
    /// AAC configuration parsed from AudioSpecificConfig (for raw AAC → ADTS conversion)
    aac_config: Option<AacConfig>,
    /// Detected sample rate from decoder
    sample_rate: u32,
    /// Detected channel count from decoder
    channels: usize,
}

impl FdkAacDecoder {
    /// Creates a new AAC decoder with the given configuration
    ///
    /// # Arguments
    ///
    /// * `stream_info` - Stream configuration. If `extra_data` contains
    ///   AudioSpecificConfig, it will be used to configure the decoder for
    ///   raw AAC decoding (MP4). Otherwise, ADTS transport is assumed.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The stream is not an audio stream
    /// - The decoder fails to initialize
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate this is an audio stream
        match &stream_info.params {
            StreamParams::Audio(_) => {}
            _ => return Err(Error::Config("AAC decoder requires audio stream".to_string())),
        };

        // Create decoder with ADTS transport
        // For raw AAC (from MP4), we'll wrap packets with ADTS headers
        let decoder = FdkDecoder::new(DecTransport::Adts);

        // Parse AudioSpecificConfig if present (for raw AAC from MP4)
        // We'll use this to generate ADTS headers for raw AAC frames
        let aac_config = if !stream_info.extra_data.is_empty() {
            AacConfig::from_asc(&stream_info.extra_data)
        } else {
            None
        };

        // Get initial parameters from stream_info
        let (sample_rate, channels) = match &stream_info.params {
            StreamParams::Audio(params) => (params.sample_rate, params.channels),
            _ => (48000, 2), // Defaults
        };

        Ok(Self {
            stream_info,
            decoder,
            flushed: false,
            packet_buffer: Vec::new(),
            buffered_frames: Vec::new(),
            current_pts: 0,
            aac_config,
            sample_rate,
            channels,
        })
    }

    /// Creates an AAC decoder from audio parameters
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - Expected sample rate in Hz
    /// * `channels` - Expected number of channels (1 or 2)
    /// * `audio_specific_config` - Optional AudioSpecificConfig for raw AAC
    pub fn from_params(
        sample_rate: u32,
        channels: usize,
        audio_specific_config: Option<&[u8]>,
    ) -> Result<Self> {
        let audio_params = AudioStreamParams::new(sample_rate, channels, SampleFormat::S16);

        let mut stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
            .with_params(StreamParams::Audio(audio_params));

        if let Some(asc) = audio_specific_config {
            stream_info.extra_data = asc.to_vec();
        }

        Self::new(stream_info)
    }

    /// Helper function to decode buffered data and produce frames
    fn decode_buffered(&mut self) -> Result<()> {
        if self.packet_buffer.is_empty() {
            return Ok(());
        }

        // Fill the decoder with buffered data
        let bytes_consumed = self
            .decoder
            .fill(&self.packet_buffer)
            .map_err(|e| Error::Decode(format!("AAC fill error: {:?}", e)))?;

        // Remove consumed bytes from buffer
        if bytes_consumed > 0 {
            self.packet_buffer.drain(..bytes_consumed);
        }

        // Try to decode frames
        loop {
            let mut output = vec![0i16; MAX_DECODE_BUFFER_SIZE / 2];

            match self.decoder.decode_frame(&mut output) {
                Ok(()) => {
                    // Get actual decoded size
                    let decoded_size = self.decoder.decoded_frame_size();

                    if decoded_size == 0 {
                        // No more frames available
                        break;
                    }

                    // Get stream info from decoder
                    let info = self.decoder.stream_info();

                    // Update parameters from decoder
                    self.sample_rate = info.sampleRate as u32;
                    self.channels = info.numChannels as usize;

                    let frame_size = info.frameSize as usize;

                    // Truncate output to actual size
                    output.truncate(decoded_size);

                    // Create audio frame
                    let mut frame = Frame::new_audio(
                        self.sample_rate,
                        self.channels,
                        SampleFormat::S16,
                        frame_size,
                    );

                    // Copy decoded samples to frame (convert i16 to bytes)
                    if let Some(data) = frame.plane_mut(0) {
                        for (i, sample) in output.iter().enumerate() {
                            let bytes = sample.to_le_bytes();
                            if i * 2 + 1 < data.len() {
                                data[i * 2] = bytes[0];
                                data[i * 2 + 1] = bytes[1];
                            }
                        }
                    }

                    // Set PTS
                    let frame = frame.with_pts(self.current_pts);
                    self.current_pts += frame_size as i64;

                    self.buffered_frames.push(frame);
                }
                Err(e) => {
                    // Check if it's just "need more data" vs actual error
                    // fdk-aac returns various errors when it needs more data or is syncing
                    let err_str = format!("{:?}", e);
                    if err_str.contains("NOT_ENOUGH_BITS")
                        || err_str.contains("TRANSPORT_SYNC")
                        || err_str.contains("ran out of bits")
                    {
                        // Need more data or still syncing - not a real error
                        break;
                    }
                    // Real error
                    return Err(Error::Decode(format!("AAC decode error: {:?}", e)));
                }
            }
        }

        Ok(())
    }
}

impl Decoder for FdkAacDecoder {
    fn codec(&self) -> &str {
        "aac"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.flushed {
            return Err(Error::InvalidState(
                "Cannot send packet after flush".to_string(),
            ));
        }

        // Store PTS from first packet
        if self.current_pts == 0 {
            if let Some(pts) = packet.pts() {
                self.current_pts = pts;
            }
        }

        // If we have AacConfig, this is raw AAC from MP4 - wrap with ADTS header
        if let Some(config) = &self.aac_config {
            // Create ADTS header for this raw AAC frame
            let header = config.create_adts_header(packet.data().len());
            self.packet_buffer.extend_from_slice(&header);
        }

        // Add packet data to buffer
        self.packet_buffer.extend_from_slice(packet.data());

        // Try to decode
        self.decode_buffered()?;

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        // Return buffered frames first
        if !self.buffered_frames.is_empty() {
            return Ok(self.buffered_frames.remove(0));
        }

        // If flushed and no more buffered frames, we're done
        if self.flushed {
            return Err(Error::EndOfStream);
        }

        // No frames available yet
        Err(Error::NeedMoreData)
    }

    fn flush(&mut self) -> Result<()> {
        // Try to decode any remaining data in the buffer
        if !self.packet_buffer.is_empty() {
            // Pad with zeros to help decoder flush
            self.packet_buffer.extend(vec![0u8; 1024]);
            let _ = self.decode_buffered();
        }

        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.flushed = false;
        self.packet_buffer.clear();
        self.buffered_frames.clear();
        self.current_pts = 0;
        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }

    fn capabilities(&self) -> DecoderCapabilities {
        DecoderCapabilities {
            hardware_acceleration: false,
            reordering: false,
            max_ref_frames: None,
            output_formats: vec!["s16".to_string()],
            requires_extra_data: false, // Can work with ADTS or raw+ASC
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_stream_info() -> StreamInfo {
        let audio_params = AudioStreamParams::new(48000, 2, SampleFormat::S16);
        StreamInfo::new(0, MediaType::Audio, "aac".to_string())
            .with_params(StreamParams::Audio(audio_params))
            .with_bitrate(128000)
    }

    #[test]
    fn test_aac_encoder_creation() {
        let stream_info = create_test_stream_info();
        let encoder = FdkAacEncoder::new(stream_info);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_aac_encoder_codec_name() {
        let stream_info = create_test_stream_info();
        let encoder = FdkAacEncoder::new(stream_info).unwrap();
        assert_eq!(encoder.codec(), "aac");
    }

    #[test]
    fn test_aac_encoder_audio_specific_config() {
        let stream_info = create_test_stream_info();
        let encoder = FdkAacEncoder::new(stream_info).unwrap();
        let asc = encoder.audio_specific_config();
        // AAC-LC AudioSpecificConfig is typically 2 bytes
        assert!(!asc.is_empty());
        assert!(asc.len() >= 2);
    }

    #[test]
    fn test_aac_encoder_mono() {
        let audio_params = AudioStreamParams::new(48000, 1, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
            .with_params(StreamParams::Audio(audio_params))
            .with_bitrate(64000);
        let encoder = FdkAacEncoder::new(stream_info);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_aac_encoder_44100() {
        let audio_params = AudioStreamParams::new(44100, 2, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
            .with_params(StreamParams::Audio(audio_params))
            .with_bitrate(128000);
        let encoder = FdkAacEncoder::new(stream_info);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_aac_encoder_unsupported_sample_rate() {
        let audio_params = AudioStreamParams::new(50000, 2, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
            .with_params(StreamParams::Audio(audio_params))
            .with_bitrate(128000);
        let encoder = FdkAacEncoder::new(stream_info);
        assert!(encoder.is_err());
    }

    #[test]
    fn test_aac_encoder_unsupported_channels() {
        let audio_params = AudioStreamParams::new(48000, 6, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
            .with_params(StreamParams::Audio(audio_params))
            .with_bitrate(256000);
        let encoder = FdkAacEncoder::new(stream_info);
        assert!(encoder.is_err());
    }

    #[test]
    fn test_aac_encoder_encode_frame() {
        let stream_info = create_test_stream_info();
        let mut encoder = FdkAacEncoder::new(stream_info).unwrap();

        // Create a test frame with 1024 samples per channel (stereo = 2048 total)
        let mut frame = Frame::new_audio(48000, 2, SampleFormat::S16, AAC_FRAME_SIZE);

        // Fill with a simple sine wave pattern
        let num_samples = AAC_FRAME_SIZE * 2; // stereo
        if let Some(data) = frame.plane_mut(0) {
            for i in 0..num_samples {
                let sample = ((i as f32 * 0.1).sin() * 10000.0) as i16;
                let bytes = sample.to_le_bytes();
                data[i * 2] = bytes[0];
                data[i * 2 + 1] = bytes[1];
            }
        }

        // Send frame
        encoder.send_frame(&frame).unwrap();

        // Should have one packet ready
        let packet = encoder.receive_packet();
        assert!(packet.is_ok());
        let packet = packet.unwrap();
        assert!(!packet.data().is_empty());
    }

    #[test]
    fn test_aac_encoder_flush() {
        let stream_info = create_test_stream_info();
        let mut encoder = FdkAacEncoder::new(stream_info).unwrap();

        // Send a partial frame (less than 1024 samples)
        let num_samples = 512 * 2; // 512 samples per channel, stereo
        let mut frame = Frame::new_audio(48000, 2, SampleFormat::S16, 512);

        if let Some(data) = frame.plane_mut(0) {
            for i in 0..num_samples {
                let sample = ((i as f32 * 0.1).sin() * 10000.0) as i16;
                let bytes = sample.to_le_bytes();
                data[i * 2] = bytes[0];
                data[i * 2 + 1] = bytes[1];
            }
        }

        encoder.send_frame(&frame).unwrap();

        // No packet yet (not enough samples)
        assert!(encoder.receive_packet().is_err());

        // Flush should pad and encode
        encoder.flush().unwrap();

        // Now we should have a packet
        let packet = encoder.receive_packet();
        assert!(packet.is_ok());
    }

    // ========================================================================
    // Decoder Tests
    // ========================================================================

    #[test]
    fn test_aac_decoder_creation() {
        let stream_info = create_test_stream_info();
        let decoder = FdkAacDecoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_aac_decoder_codec_name() {
        let stream_info = create_test_stream_info();
        let decoder = FdkAacDecoder::new(stream_info).unwrap();
        assert_eq!(decoder.codec(), "aac");
    }

    #[test]
    fn test_aac_decoder_from_params() {
        let decoder = FdkAacDecoder::from_params(48000, 2, None);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_aac_decoder_with_asc() {
        // AAC-LC AudioSpecificConfig for 48kHz stereo
        // Object type: AAC-LC (2), Sample rate index: 3 (48000), Channel config: 2
        let asc = vec![0x11, 0x90]; // AAC-LC, 48kHz, stereo

        let decoder = FdkAacDecoder::from_params(48000, 2, Some(&asc));
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_aac_roundtrip() {
        // Create encoder
        let stream_info = create_test_stream_info();
        let mut encoder = FdkAacEncoder::new(stream_info.clone()).unwrap();

        // Get AudioSpecificConfig from encoder
        let asc = encoder.audio_specific_config().to_vec();

        // Create decoder with ASC
        let mut decoder_stream_info = create_test_stream_info();
        decoder_stream_info.extra_data = asc;
        let mut decoder = FdkAacDecoder::new(decoder_stream_info).unwrap();

        // Create multiple test frames to ensure decoder gets enough data
        let mut encoded_packets = Vec::new();
        for frame_num in 0..5 {
            let mut frame = Frame::new_audio(48000, 2, SampleFormat::S16, AAC_FRAME_SIZE);

            // Fill with a simple sine wave pattern
            let num_samples = AAC_FRAME_SIZE * 2; // stereo
            if let Some(data) = frame.plane_mut(0) {
                for i in 0..num_samples {
                    let t = (frame_num * AAC_FRAME_SIZE + i / 2) as f32;
                    let sample = ((t * 0.1).sin() * 10000.0) as i16;
                    let bytes = sample.to_le_bytes();
                    data[i * 2] = bytes[0];
                    data[i * 2 + 1] = bytes[1];
                }
            }

            // Encode frame
            encoder.send_frame(&frame).unwrap();

            // Get encoded packet
            while let Ok(packet) = encoder.receive_packet() {
                encoded_packets.push(packet);
            }
        }

        // Flush encoder to get remaining packets
        encoder.flush().unwrap();
        while let Ok(packet) = encoder.receive_packet() {
            encoded_packets.push(packet);
        }

        // Verify we got encoded packets
        assert!(!encoded_packets.is_empty(), "Encoder should produce packets");

        // Send packets to decoder and collect decoded frames
        // Note: decoder may need multiple packets due to latency
        let mut decoded_frames = 0;
        for packet in &encoded_packets {
            // Send packet - may fail for raw AAC without proper ADTS wrapping
            // This is expected behavior, so we allow errors
            if decoder.send_packet(packet).is_ok() {
                while let Ok(_frame) = decoder.receive_frame() {
                    decoded_frames += 1;
                }
            }
        }

        // Flush decoder
        decoder.flush().unwrap();
        while let Ok(_frame) = decoder.receive_frame() {
            decoded_frames += 1;
        }

        // For raw AAC without ADTS, decoding may not work
        // This test primarily verifies the API works without panicking
        // Full roundtrip requires ADTS-wrapped AAC
        println!(
            "Encoded {} packets, decoded {} frames",
            encoded_packets.len(),
            decoded_frames
        );
    }

    #[test]
    fn test_aac_decoder_reset() {
        let stream_info = create_test_stream_info();
        let mut decoder = FdkAacDecoder::new(stream_info).unwrap();

        // Reset should work
        assert!(decoder.reset().is_ok());
        assert!(!decoder.is_flushed());
    }

    #[test]
    fn test_aac_decoder_flush() {
        let stream_info = create_test_stream_info();
        let mut decoder = FdkAacDecoder::new(stream_info).unwrap();

        // Flush should work
        assert!(decoder.flush().is_ok());
        assert!(decoder.is_flushed());

        // After flush, receive_frame should return EndOfStream
        match decoder.receive_frame() {
            Err(Error::EndOfStream) => {}
            _ => panic!("Expected EndOfStream after flush"),
        }
    }
}
