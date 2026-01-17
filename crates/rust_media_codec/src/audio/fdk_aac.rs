//! FDK-AAC audio encoder implementation
//!
//! AAC (Advanced Audio Coding) encoder using the Fraunhofer FDK AAC library.
//! This implementation uses the fdk-aac crate for Rust bindings.
//!
//! # Supported Profiles
//!
//! - **AAC-LC** (Low Complexity) - Most widely compatible
//! - **HE-AAC** (High Efficiency with SBR) - Better compression at low bitrates
//! - **HE-AACv2** (with SBR + Parametric Stereo) - Best for stereo at very low bitrates
//!
//! # Example
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
//! # Licensing
//!
//! This encoder uses libfdk-aac which is licensed under the Fraunhofer FDK AAC
//! Codec License. This is not GPL, but has some restrictions on use.

use fdk_aac::enc::{
    AudioObjectType, BitRate, ChannelMode, Encoder as FdkEncoder, EncoderParams, Transport,
};
use rust_media_core::{
    AudioStreamParams, Encoder, EncoderCapabilities, Error, Frame, MediaType, Packet, Result,
    SampleFormat, StreamInfo, StreamParams,
};

/// AAC frame size in samples per channel
const AAC_FRAME_SIZE: usize = 1024;

/// Maximum AAC output packet size
const MAX_AAC_PACKET_SIZE: usize = 8192;

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
    sample_rate: u32,
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
            transport: Transport::Raw, // Raw AAC for MP4 muxing (no ADTS headers)
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
            sample_rate,
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
}
