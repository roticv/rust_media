//! FLAC audio decoder implementation
//!
//! Provides FLAC decoding via the claxon crate (Apache-2.0, pure Rust).
//! Decode-only — FLAC encoding is not supported.
//!
//! # Initialization
//!
//! The decoder requires the FLAC STREAMINFO metadata block, which arrives via
//! `StreamInfo.extra_data` in the Matroska CodecPrivate format:
//! `fLaC` (4 bytes) + metadata block header (4 bytes) + STREAMINFO (34 bytes).
//!
//! # Output Format
//!
//! - Signed 16-bit PCM (S16), interleaved for multi-channel
//! - FLAC samples are i32 at the codec's native bit depth (typically 16 or 24);
//!   we shift/truncate to 16-bit for consistent output with other decoders.

use claxon::frame::FrameReader;
use rust_media_core::{
    Decoder, DecoderCapabilities, Error, Frame, MediaType, Packet, Result, SampleFormat, StreamInfo,
};
use std::collections::VecDeque;
use std::io::Cursor;

/// FLAC stream metadata parsed from CodecPrivate.
#[allow(dead_code)]
struct FlacStreamInfo {
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
}

/// FLAC audio decoder using claxon.
///
/// Decodes FLAC audio packets into interleaved S16 PCM frames.
///
/// # Container Support
///
/// FLAC in MKV: the demuxer passes the FLAC header (fLaC + STREAMINFO) as
/// `StreamInfo.extra_data`. Audio packets contain raw FLAC frames.
pub struct FlacDecoder {
    stream_info: StreamInfo,
    flac_info: FlacStreamInfo,
    buffered_frames: VecDeque<Frame>,
    flushed: bool,
}

impl FlacDecoder {
    /// Creates a new FLAC decoder from stream information.
    ///
    /// `stream_info.extra_data` must contain the FLAC header with STREAMINFO.
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "flac" {
            return Err(Error::Config(format!(
                "Expected flac codec, got: {}",
                stream_info.codec
            )));
        }

        let flac_info = parse_flac_streaminfo(&stream_info.extra_data)?;

        Ok(Self {
            stream_info,
            flac_info,
            buffered_frames: VecDeque::new(),
            flushed: false,
        })
    }
}

impl Decoder for FlacDecoder {
    fn codec(&self) -> &str {
        "flac"
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

        if packet.media_type() != MediaType::Audio {
            return Err(Error::InvalidData(format!(
                "Expected audio packet, got {:?}",
                packet.media_type()
            )));
        }

        let data = packet.data();
        if data.is_empty() {
            return Ok(());
        }

        // Decode raw FLAC frame using claxon's FrameReader.
        // Each MKV audio packet is a complete FLAC frame.
        let cursor = Cursor::new(data);
        let mut reader = FrameReader::new(cursor);
        let block = reader
            .read_next_or_eof(Vec::new())
            .map_err(|e| Error::Decode(format!("FLAC decode: {}", e)))?;

        let block = match block {
            Some(b) => b,
            None => return Ok(()),
        };

        let channels = block.channels() as usize;
        let samples_per_channel = block.duration() as usize;
        let sample_rate = self.flac_info.sample_rate;
        let bps = self.flac_info.bits_per_sample;

        let mut frame = Frame::new_audio(
            sample_rate,
            channels,
            SampleFormat::S16,
            samples_per_channel,
        );

        // Convert i32 samples to interleaved S16.
        // FLAC stores samples as i32 at the native bit depth.
        // Shift to 16-bit: if bps > 16, right-shift; if bps < 16, left-shift.
        if let Some(frame_data) = frame.plane_mut(0) {
            for s in 0..samples_per_channel {
                for ch in 0..channels {
                    let sample_i32 = block.sample(ch as u32, s as u32);
                    let sample_i16 = if bps > 16 {
                        (sample_i32 >> (bps - 16)) as i16
                    } else if bps < 16 {
                        (sample_i32 << (16 - bps)) as i16
                    } else {
                        sample_i32 as i16
                    };
                    let idx = (s * channels + ch) * 2;
                    let bytes = sample_i16.to_le_bytes();
                    frame_data[idx] = bytes[0];
                    frame_data[idx + 1] = bytes[1];
                }
            }
        }

        if let Some(pts) = packet.pts() {
            frame.set_pts(Some(pts));
        }

        let duration = (samples_per_channel as u64 * 1_000_000) / sample_rate as u64;
        frame = frame.with_duration(duration as i64);

        self.buffered_frames.push_back(frame);
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if let Some(frame) = self.buffered_frames.pop_front() {
            Ok(frame)
        } else if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.buffered_frames.clear();
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
            output_formats: vec!["s16".to_string()],
            requires_extra_data: true,
        }
    }
}

/// Parse FLAC STREAMINFO from Matroska CodecPrivate data.
///
/// Expected format:
/// - `fLaC` (4 bytes, optional — some muxers omit it)
/// - Metadata block header (4 bytes): type(7 bits) + last-flag(1 bit) + length(24 bits)
/// - STREAMINFO data (34 bytes)
fn parse_flac_streaminfo(data: &[u8]) -> Result<FlacStreamInfo> {
    if data.is_empty() {
        return Err(Error::InvalidData(
            "Empty FLAC CodecPrivate".to_string(),
        ));
    }

    // Find where the metadata block header starts.
    let meta_start = if data.len() >= 4 && &data[0..4] == b"fLaC" {
        4 // Skip the fLaC marker
    } else {
        0 // No marker, assume data starts at metadata block header
    };

    if data.len() < meta_start + 4 + 34 {
        return Err(Error::InvalidData(
            "FLAC CodecPrivate too short for STREAMINFO".to_string(),
        ));
    }

    // Metadata block header: 1 byte (last-flag | type) + 3 bytes (length)
    let block_type = data[meta_start] & 0x7F;
    if block_type != 0 {
        return Err(Error::InvalidData(format!(
            "Expected STREAMINFO block (type 0), got type {}",
            block_type
        )));
    }

    let si = &data[meta_start + 4..]; // Skip 4-byte block header

    // STREAMINFO layout (34 bytes):
    // bytes 0-1:   min block size (u16 BE)
    // bytes 2-3:   max block size (u16 BE)
    // bytes 4-6:   min frame size (u24 BE)
    // bytes 7-9:   max frame size (u24 BE)
    // bytes 10-13: sample_rate(20 bits) | channels-1(3 bits) | bps-1(5 bits) | total_samples_hi(4 bits)
    // bytes 14-17: total_samples_lo (32 bits)
    // bytes 18-33: MD5 (16 bytes)

    if si.len() < 18 {
        return Err(Error::InvalidData(
            "STREAMINFO data too short".to_string(),
        ));
    }

    let sample_rate = ((si[10] as u32) << 12) | ((si[11] as u32) << 4) | ((si[12] as u32) >> 4);
    let channels = (((si[12] >> 1) & 0x07) as u32) + 1;
    let bits_per_sample = ((((si[12] & 0x01) as u32) << 4) | ((si[13] >> 4) as u32)) + 1;

    if sample_rate == 0 {
        return Err(Error::InvalidData(
            "FLAC STREAMINFO: sample rate is 0".to_string(),
        ));
    }

    Ok(FlacStreamInfo {
        sample_rate,
        channels,
        bits_per_sample,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::{AudioStreamParams, StreamParams};

    /// Build a minimal FLAC CodecPrivate with fLaC marker + STREAMINFO.
    fn build_flac_codec_private(sample_rate: u32, channels: u32, bps: u32) -> Vec<u8> {
        let mut buf = Vec::new();

        // fLaC marker
        buf.extend_from_slice(b"fLaC");

        // Metadata block header: type 0 (STREAMINFO), last-metadata-block flag, length 34
        buf.push(0x80); // last-metadata-block = 1, type = 0
        buf.push(0x00);
        buf.push(0x00);
        buf.push(0x22); // length = 34

        // STREAMINFO (34 bytes)
        // min/max block size
        buf.extend_from_slice(&[0x00, 0x10]); // min = 16
        buf.extend_from_slice(&[0x10, 0x00]); // max = 4096
        // min/max frame size (24-bit each)
        buf.extend_from_slice(&[0x00, 0x00, 0x00]); // min = 0
        buf.extend_from_slice(&[0x00, 0x00, 0x00]); // max = 0

        // sample_rate (20 bits) | channels-1 (3 bits) | bps-1 (5 bits) | total_samples_hi (4 bits)
        let sr = sample_rate;
        let ch = channels - 1;
        let bp = bps - 1;
        buf.push((sr >> 12) as u8);
        buf.push((sr >> 4) as u8);
        buf.push(((sr << 4) as u8 & 0xF0) | ((ch << 1) as u8 & 0x0E) | ((bp >> 4) as u8 & 0x01));
        buf.push((bp << 4) as u8 & 0xF0); // total_samples_hi = 0

        // total_samples_lo (32 bits)
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

        // MD5 checksum (16 bytes)
        buf.extend_from_slice(&[0u8; 16]);

        buf
    }

    #[test]
    fn test_flac_decoder_wrong_codec() {
        let audio_params = AudioStreamParams::new(44100, 2, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "opus".to_string())
            .with_params(StreamParams::Audio(audio_params));
        assert!(FlacDecoder::new(stream_info).is_err());
    }

    #[test]
    fn test_flac_decoder_missing_extra_data() {
        let audio_params = AudioStreamParams::new(44100, 2, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "flac".to_string())
            .with_params(StreamParams::Audio(audio_params));
        assert!(FlacDecoder::new(stream_info).is_err());
    }

    #[test]
    fn test_parse_streaminfo_44100_stereo_16bit() {
        let codec_private = build_flac_codec_private(44100, 2, 16);
        let info = parse_flac_streaminfo(&codec_private).unwrap();
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bits_per_sample, 16);
    }

    #[test]
    fn test_parse_streaminfo_48000_mono_24bit() {
        let codec_private = build_flac_codec_private(48000, 1, 24);
        let info = parse_flac_streaminfo(&codec_private).unwrap();
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 24);
    }

    #[test]
    fn test_parse_streaminfo_96000_6ch_24bit() {
        let codec_private = build_flac_codec_private(96000, 6, 24);
        let info = parse_flac_streaminfo(&codec_private).unwrap();
        assert_eq!(info.sample_rate, 96000);
        assert_eq!(info.channels, 6);
        assert_eq!(info.bits_per_sample, 24);
    }

    #[test]
    fn test_parse_streaminfo_no_flac_marker() {
        // Some muxers omit the fLaC marker
        let full = build_flac_codec_private(44100, 2, 16);
        let without_marker = &full[4..]; // Skip fLaC
        let info = parse_flac_streaminfo(without_marker).unwrap();
        assert_eq!(info.sample_rate, 44100);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bits_per_sample, 16);
    }

    #[test]
    fn test_flac_decoder_creation() {
        let codec_private = build_flac_codec_private(44100, 2, 16);
        let audio_params = AudioStreamParams::new(44100, 2, SampleFormat::S16);
        let mut stream_info = StreamInfo::new(0, MediaType::Audio, "flac".to_string())
            .with_params(StreamParams::Audio(audio_params));
        stream_info.extra_data = codec_private;
        assert!(FlacDecoder::new(stream_info).is_ok());
    }
}
