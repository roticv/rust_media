//! Opus audio codec implementation
//!
//! Opus is a lossy audio codec developed by the IETF, optimized for interactive speech
//! and music transmission over the Internet. This implementation uses libopus via the
//! audiopus crate.

use audiopus::{
    coder::Decoder as OpusDecoderImpl, coder::Encoder as OpusEncoderImpl, packet::Packet as OpusPacket,
    Channels, MutSignals, SampleRate,
};
use rust_media_core::{
    AudioStreamParams, Decoder, DecoderCapabilities, Encoder, EncoderCapabilities, Error, Frame,
    MediaType, Packet, Result, SampleFormat, StreamInfo, StreamParams,
};

/// Opus Audio Decoder
///
/// Decodes Opus audio packets into PCM frames using libopus.
pub struct OpusDecoder {
    stream_info: StreamInfo,
    decoder: OpusDecoderImpl,
    flushed: bool,
    buffer: Option<Packet>,
}

impl OpusDecoder {
    /// Creates a new Opus decoder
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate that this is an Opus audio stream
        if stream_info.codec != "opus" {
            return Err(Error::Config(format!(
                "Expected Opus codec, got: {}",
                stream_info.codec
            )));
        }

        // Extract audio parameters
        let audio_params = match &stream_info.params {
            StreamParams::Audio(params) => params,
            _ => return Err(Error::Config("Opus decoder requires audio stream".to_string())),
        };

        // Opus natively supports 48kHz sample rate
        // The decoder will handle resampling if needed
        let sample_rate = match audio_params.sample_rate {
            8000 => SampleRate::Hz8000,
            12000 => SampleRate::Hz12000,
            16000 => SampleRate::Hz16000,
            24000 => SampleRate::Hz24000,
            48000 => SampleRate::Hz48000,
            _ => {
                return Err(Error::Unsupported(format!(
                    "Opus only supports 8, 12, 16, 24, or 48 kHz sample rates, got {}",
                    audio_params.sample_rate
                )))
            }
        };

        // Determine channel configuration
        let channels = match audio_params.channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => {
                return Err(Error::Unsupported(format!(
                    "Opus decoder only supports mono or stereo, got {} channels",
                    audio_params.channels
                )))
            }
        };

        // Create libopus decoder
        let decoder = OpusDecoderImpl::new(sample_rate, channels)
            .map_err(|e| Error::Config(format!("Failed to create Opus decoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            decoder,
            flushed: false,
            buffer: None,
        })
    }
}

impl Decoder for OpusDecoder {
    fn codec(&self) -> &str {
        "opus"
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

        // Store packet for decoding
        self.buffer = Some(packet.clone());
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if self.flushed {
            return Err(Error::EndOfStream);
        }

        // Check if we have a packet to decode
        let packet = match self.buffer.take() {
            Some(p) => p,
            None => return Err(Error::NeedMoreData),
        };

        // Get audio parameters
        let audio_params = match &self.stream_info.params {
            StreamParams::Audio(params) => params,
            _ => unreachable!(),
        };

        // Maximum frame size for Opus at 48kHz is 120ms = 5760 samples per channel
        let max_frame_size = 5760;
        let mut output = vec![0i16; max_frame_size * audio_params.channels];

        // Create OpusPacket from the packet data
        let opus_packet = OpusPacket::try_from(packet.data())
            .map_err(|e| Error::InvalidData(format!("Invalid Opus packet: {:?}", e)))?;

        // Decode the packet using MutSignals
        let mut_signals: MutSignals<i16> = (&mut output[..]).try_into()
            .map_err(|e| Error::InvalidData(format!("Failed to create MutSignals: {:?}", e)))?;
        let decoded_samples = self
            .decoder
            .decode(Some(opus_packet), mut_signals, false)
            .map_err(|e| Error::Decode(format!("Opus decode error: {:?}", e)))?;

        // Calculate the actual duration based on decoded samples
        let duration = (decoded_samples as u64 * 1_000_000) / audio_params.sample_rate as u64;

        // Create frame with appropriate PTS from packet
        let pts = packet.pts().unwrap_or(0);

        // Create audio frame
        let mut frame = Frame::new_audio(
            audio_params.sample_rate,
            audio_params.channels,
            audio_params.sample_format,
            decoded_samples,
        );

        // Copy decoded samples to frame data (first plane for interleaved audio)
        let frame_data = frame.plane_mut(0).ok_or_else(|| {
            Error::InvalidState("Frame missing data plane".to_string())
        })?;

        for (i, &sample) in output[..decoded_samples * audio_params.channels].iter().enumerate() {
            let bytes = sample.to_le_bytes();
            frame_data[i * 2] = bytes[0];
            frame_data[i * 2 + 1] = bytes[1];
        }

        Ok(frame.with_pts(pts).with_duration(duration as i64))
    }

    fn flush(&mut self) -> Result<()> {
        self.flushed = true;
        self.buffer = None;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.flushed = false;
        self.buffer = None;
        // Note: libopus doesn't provide a reset method, so we keep the decoder state
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
            requires_extra_data: false,
        }
    }
}

/// Opus Audio Encoder
///
/// Encodes PCM audio frames into Opus packets using libopus.
pub struct OpusEncoder {
    stream_info: StreamInfo,
    encoder: OpusEncoderImpl,
    flushed: bool,
    buffer: Option<Frame>,
}

impl OpusEncoder {
    /// Creates a new Opus encoder with the given configuration
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Extract audio parameters
        let audio_params = match &stream_info.params {
            StreamParams::Audio(params) => params,
            _ => return Err(Error::Config("Opus encoder requires audio stream".to_string())),
        };

        // Opus natively supports 48kHz, but can encode other rates
        let sample_rate = match audio_params.sample_rate {
            8000 => SampleRate::Hz8000,
            12000 => SampleRate::Hz12000,
            16000 => SampleRate::Hz16000,
            24000 => SampleRate::Hz24000,
            48000 => SampleRate::Hz48000,
            _ => {
                return Err(Error::Unsupported(format!(
                    "Opus only supports 8, 12, 16, 24, or 48 kHz sample rates, got {}",
                    audio_params.sample_rate
                )))
            }
        };

        // Determine channel configuration
        let channels = match audio_params.channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => {
                return Err(Error::Unsupported(format!(
                    "Opus encoder only supports mono or stereo, got {} channels",
                    audio_params.channels
                )))
            }
        };

        // Create libopus encoder with VoIP application mode
        let encoder = OpusEncoderImpl::new(sample_rate, channels, audiopus::Application::Voip)
            .map_err(|e| Error::Config(format!("Failed to create Opus encoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            encoder,
            flushed: false,
            buffer: None,
        })
    }

    /// Creates an Opus encoder from audio parameters
    pub fn from_params(
        sample_rate: u32,
        channels: usize,
        sample_format: SampleFormat,
    ) -> Result<Self> {
        if sample_format != SampleFormat::S16 {
            return Err(Error::Unsupported(
                "Opus encoder only supports S16 sample format".to_string(),
            ));
        }

        let audio_params = AudioStreamParams::new(sample_rate, channels, sample_format);

        let stream_info = StreamInfo::new(0, MediaType::Audio, "opus".to_string())
            .with_params(StreamParams::Audio(audio_params));

        Self::new(stream_info)
    }
}

impl Encoder for OpusEncoder {
    fn codec(&self) -> &str {
        "opus"
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

        // Store frame for encoding
        self.buffer = Some(frame.clone());
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        if self.flushed {
            return Err(Error::EndOfStream);
        }

        // Check if we have a frame to encode
        let frame = match self.buffer.take() {
            Some(f) => f,
            None => return Err(Error::NeedMoreData),
        };

        // Get audio parameters
        let audio_params = match &self.stream_info.params {
            StreamParams::Audio(params) => params,
            _ => unreachable!(),
        };

        // Get frame data from first plane (interleaved audio)
        let frame_data = frame.plane(0).ok_or_else(|| {
            Error::InvalidState("Frame missing data plane".to_string())
        })?;

        if frame_data.len() % 2 != 0 {
            return Err(Error::InvalidData(
                "Frame data must be aligned to 16-bit samples".to_string(),
            ));
        }

        // Convert frame data (bytes) to i16 samples
        let mut input = Vec::with_capacity(frame_data.len() / 2);
        for i in (0..frame_data.len()).step_by(2) {
            let sample = i16::from_le_bytes([frame_data[i], frame_data[i + 1]]);
            input.push(sample);
        }

        // Calculate number of samples per channel
        let samples_per_channel = input.len() / audio_params.channels;

        // Maximum output size for Opus
        let max_packet_size = 4000;
        let mut output = vec![0u8; max_packet_size];

        // Encode the frame - encode handles interleaved audio
        let encoded_size = self
            .encoder
            .encode(&input, &mut output)
            .map_err(|e| Error::Encode(format!("Opus encode error: {:?}", e)))?;

        // Truncate output to actual encoded size
        output.truncate(encoded_size);

        // Calculate duration in microseconds
        let duration = (samples_per_channel as u64 * 1_000_000) / audio_params.sample_rate as u64;

        // Create packet with appropriate PTS from frame
        let pts = frame.pts().unwrap_or(0);
        let packet = Packet::new(output, 0, MediaType::Audio)
            .with_pts(pts)
            .with_duration(duration as i64);

        Ok(packet)
    }

    fn flush(&mut self) -> Result<()> {
        self.flushed = true;
        self.buffer = None;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.flushed = false;
        self.buffer = None;
        // Reset encoder state if needed
        // Note: libopus doesn't provide explicit reset, consider recreating encoder
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
        StreamInfo::new(0, MediaType::Audio, "opus".to_string())
            .with_params(StreamParams::Audio(audio_params))
    }

    #[test]
    fn test_opus_decoder_creation() {
        let stream_info = create_test_stream_info();
        let decoder = OpusDecoder::new(stream_info).unwrap();

        assert_eq!(decoder.codec(), "opus");
        assert!(!decoder.is_flushed());
    }

    #[test]
    fn test_opus_decoder_wrong_codec() {
        let mut stream_info = create_test_stream_info();
        stream_info.codec = "aac".to_string();

        let result = OpusDecoder::new(stream_info);
        assert!(result.is_err());
    }

    #[test]
    fn test_opus_decoder_flush() {
        let stream_info = create_test_stream_info();
        let mut decoder = OpusDecoder::new(stream_info).unwrap();

        assert!(!decoder.is_flushed());
        decoder.flush().unwrap();
        assert!(decoder.is_flushed());

        match decoder.receive_frame() {
            Err(Error::EndOfStream) => {}
            _ => panic!("Expected EndOfStream after flush"),
        }
    }

    #[test]
    fn test_opus_encoder_creation() {
        let stream_info = create_test_stream_info();
        let encoder = OpusEncoder::new(stream_info).unwrap();

        assert_eq!(encoder.codec(), "opus");
        assert!(!encoder.is_flushed());
    }

    #[test]
    fn test_opus_encoder_from_params() {
        let encoder = OpusEncoder::from_params(48000, 2, SampleFormat::S16).unwrap();

        assert_eq!(encoder.codec(), "opus");
        let stream_info = encoder.stream_info();
        match &stream_info.params {
            StreamParams::Audio(params) => {
                assert_eq!(params.sample_rate, 48000);
                assert_eq!(params.channels, 2);
                assert_eq!(params.sample_format, SampleFormat::S16);
            }
            _ => panic!("Expected audio parameters"),
        }
    }

    #[test]
    fn test_opus_encode_decode_roundtrip() {
        // Create encoder
        let mut encoder = OpusEncoder::from_params(48000, 2, SampleFormat::S16).unwrap();

        // Create a simple audio frame (20ms at 48kHz = 960 samples per channel, stereo = 1920 samples)
        let samples_per_channel = 960;

        // Create audio frame
        let mut frame = Frame::new_audio(48000, 2, SampleFormat::S16, samples_per_channel)
            .with_pts(0)
            .with_duration(20000); // 20ms in microseconds

        // Generate a simple sine wave and populate frame data
        let frame_data = frame.plane_mut(0).unwrap();
        for i in 0..samples_per_channel * 2 {
            let sample = ((i as f32 * 0.1).sin() * 16000.0) as i16;
            let bytes = sample.to_le_bytes();
            frame_data[i * 2] = bytes[0];
            frame_data[i * 2 + 1] = bytes[1];
        }

        // Encode
        encoder.send_frame(&frame).unwrap();
        let packet = encoder.receive_packet().unwrap();

        assert!(packet.size() > 0);
        assert!(packet.size() < 4000); // Opus packets are typically small

        // Create decoder
        let stream_info = create_test_stream_info();
        let mut decoder = OpusDecoder::new(stream_info).unwrap();

        // Decode
        decoder.send_packet(&packet).unwrap();
        let decoded_frame = decoder.receive_frame().unwrap();

        // Verify frame properties
        assert_eq!(decoded_frame.pts(), Some(0));

        // Decoded frame should have similar size (may differ slightly due to codec)
        // 960 samples per channel * 2 channels * 2 bytes per sample = 3840 bytes
        let decoded_data = decoded_frame.plane(0).unwrap();
        assert!(decoded_data.len() >= 3800);
        assert!(decoded_data.len() <= 3900);
    }

    #[test]
    fn test_opus_encoder_capabilities() {
        let stream_info = create_test_stream_info();
        let encoder = OpusEncoder::new(stream_info).unwrap();
        let caps = encoder.capabilities();

        assert!(!caps.hardware_acceleration);
        assert!(!caps.b_frames);
        assert!(caps.requires_alignment);
        assert_eq!(caps.input_formats, vec!["s16".to_string()]);
    }

    #[test]
    fn test_opus_decoder_capabilities() {
        let stream_info = create_test_stream_info();
        let decoder = OpusDecoder::new(stream_info).unwrap();
        let caps = decoder.capabilities();

        assert!(!caps.hardware_acceleration);
        assert!(!caps.reordering);
        assert!(!caps.requires_extra_data);
        assert_eq!(caps.output_formats, vec!["s16".to_string()]);
    }
}
