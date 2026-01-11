//! PCM (Pulse Code Modulation) codec implementation
//!
//! PCM is raw, uncompressed audio. This codec is essentially a pass-through
//! that validates input/output and handles format conversions if needed.

use rust_media_core::{
    Decoder, DecoderCapabilities, Encoder, EncoderCapabilities, Error, Frame, Packet, Result,
    SampleFormat, StreamInfo,
};

/// PCM Audio Decoder
///
/// Decodes PCM audio packets into frames. Since PCM is uncompressed,
/// this is mostly a validation and pass-through operation.
pub struct PcmDecoder {
    stream_info: StreamInfo,
    flushed: bool,
}

impl PcmDecoder {
    /// Creates a new PCM decoder
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate that this is an audio stream
        if stream_info.codec != "pcm" {
            return Err(Error::Config(format!(
                "Expected PCM codec, got: {}",
                stream_info.codec
            )));
        }

        Ok(Self {
            stream_info,
            flushed: false,
        })
    }

}

impl Decoder for PcmDecoder {
    fn codec(&self) -> &str {
        "pcm"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_packet(&mut self, _packet: &Packet) -> Result<()> {
        if self.flushed {
            return Err(Error::InvalidState(
                "Cannot send packet after flush".to_string(),
            ));
        }
        // PCM is uncompressed, so we just accept the packet
        // In a real implementation, we'd queue it for processing
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if self.flushed {
            return Err(Error::EndOfStream);
        }

        // For now, return NeedMoreData since we haven't implemented packet buffering
        // A real implementation would convert the packet data to a frame
        Err(Error::NeedMoreData)
    }

    fn flush(&mut self) -> Result<()> {
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.flushed = false;
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
            output_formats: vec!["pcm".to_string()],
            requires_extra_data: false,
        }
    }
}

/// PCM Audio Encoder
///
/// Encodes audio frames into PCM packets. Since PCM is uncompressed,
/// this is mostly a validation and pass-through operation.
pub struct PcmEncoder {
    stream_info: StreamInfo,
    flushed: bool,
}

impl PcmEncoder {
    /// Creates a new PCM encoder with the given configuration
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate that this is an audio stream
        match &stream_info.params {
            rust_media_core::StreamParams::Audio(_) => {}
            _ => {
                return Err(Error::Config(
                    "PCM encoder requires audio stream".to_string(),
                ))
            }
        }

        Ok(Self {
            stream_info,
            flushed: false,
        })
    }

    /// Creates a PCM encoder from audio parameters
    pub fn from_params(
        sample_rate: u32,
        channels: usize,
        sample_format: SampleFormat,
    ) -> Result<Self> {
        let audio_params = rust_media_core::AudioStreamParams::new(
            sample_rate,
            channels,
            sample_format,
        );

        let stream_info = StreamInfo::new(0, rust_media_core::MediaType::Audio, "pcm".to_string())
            .with_params(rust_media_core::StreamParams::Audio(audio_params));

        Self::new(stream_info)
    }
}

impl Encoder for PcmEncoder {
    fn codec(&self) -> &str {
        "pcm"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_frame(&mut self, _frame: &Frame) -> Result<()> {
        if self.flushed {
            return Err(Error::InvalidState(
                "Cannot send frame after flush".to_string(),
            ));
        }
        // PCM is uncompressed, so we just accept the frame
        // In a real implementation, we'd queue it for processing
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        if self.flushed {
            return Err(Error::EndOfStream);
        }

        // For now, return NeedMoreData since we haven't implemented frame buffering
        // A real implementation would convert the frame data to a packet
        Err(Error::NeedMoreData)
    }

    fn flush(&mut self) -> Result<()> {
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.flushed = false;
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
            input_formats: vec!["pcm".to_string()],
            requires_alignment: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::{AudioStreamParams, MediaType, StreamParams};

    fn create_test_stream_info() -> StreamInfo {
        let audio_params = AudioStreamParams::new(48000, 2, SampleFormat::S16);
        StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_params(StreamParams::Audio(audio_params))
    }

    #[test]
    fn test_pcm_decoder_creation() {
        let stream_info = create_test_stream_info();
        let decoder = PcmDecoder::new(stream_info).unwrap();

        assert_eq!(decoder.codec(), "pcm");
        assert!(!decoder.is_flushed());
    }

    #[test]
    fn test_pcm_decoder_wrong_codec() {
        let mut stream_info = create_test_stream_info();
        stream_info.codec = "aac".to_string();

        let result = PcmDecoder::new(stream_info);
        assert!(result.is_err());
    }

    #[test]
    fn test_pcm_decoder_flush() {
        let stream_info = create_test_stream_info();
        let mut decoder = PcmDecoder::new(stream_info).unwrap();

        assert!(!decoder.is_flushed());
        decoder.flush().unwrap();
        assert!(decoder.is_flushed());

        // After flush, receive_frame should return EndOfStream
        match decoder.receive_frame() {
            Err(Error::EndOfStream) => {}
            _ => panic!("Expected EndOfStream after flush"),
        }
    }

    #[test]
    fn test_pcm_decoder_reset() {
        let stream_info = create_test_stream_info();
        let mut decoder = PcmDecoder::new(stream_info).unwrap();

        decoder.flush().unwrap();
        assert!(decoder.is_flushed());

        decoder.reset().unwrap();
        assert!(!decoder.is_flushed());
    }

    #[test]
    fn test_pcm_decoder_capabilities() {
        let stream_info = create_test_stream_info();
        let decoder = PcmDecoder::new(stream_info).unwrap();
        let caps = decoder.capabilities();

        assert!(!caps.hardware_acceleration);
        assert!(!caps.reordering);
        assert!(!caps.requires_extra_data);
        assert_eq!(caps.output_formats, vec!["pcm".to_string()]);
    }

    #[test]
    fn test_pcm_encoder_creation() {
        let stream_info = create_test_stream_info();
        let encoder = PcmEncoder::new(stream_info).unwrap();

        assert_eq!(encoder.codec(), "pcm");
        assert!(!encoder.is_flushed());
    }

    #[test]
    fn test_pcm_encoder_from_params() {
        let encoder = PcmEncoder::from_params(48000, 2, SampleFormat::S16).unwrap();

        assert_eq!(encoder.codec(), "pcm");
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
    fn test_pcm_encoder_flush() {
        let stream_info = create_test_stream_info();
        let mut encoder = PcmEncoder::new(stream_info).unwrap();

        assert!(!encoder.is_flushed());
        encoder.flush().unwrap();
        assert!(encoder.is_flushed());

        // After flush, receive_packet should return EndOfStream
        match encoder.receive_packet() {
            Err(Error::EndOfStream) => {}
            _ => panic!("Expected EndOfStream after flush"),
        }
    }

    #[test]
    fn test_pcm_encoder_reset() {
        let stream_info = create_test_stream_info();
        let mut encoder = PcmEncoder::new(stream_info).unwrap();

        encoder.flush().unwrap();
        assert!(encoder.is_flushed());

        encoder.reset().unwrap();
        assert!(!encoder.is_flushed());
    }

    #[test]
    fn test_pcm_encoder_capabilities() {
        let stream_info = create_test_stream_info();
        let encoder = PcmEncoder::new(stream_info).unwrap();
        let caps = encoder.capabilities();

        assert!(!caps.hardware_acceleration);
        assert!(!caps.b_frames);
        assert_eq!(caps.max_lookahead, None);
        assert!(!caps.requires_alignment);
        assert_eq!(caps.input_formats, vec!["pcm".to_string()]);
    }
}
