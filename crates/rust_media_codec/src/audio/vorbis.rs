//! Vorbis audio decoder implementation
//!
//! Provides Vorbis decoding via the lewton crate (BSD-3-Clause, pure Rust).
//! Decode-only — Vorbis encoding is not supported.
//!
//! # Initialization
//!
//! The decoder requires Vorbis identification, comment, and setup headers.
//! These arrive via `StreamInfo.extra_data` in the Matroska/WebM CodecPrivate
//! format (Xiph-laced three-packet header block). The decoder parses them at
//! construction time so it can decode audio packets immediately.
//!
//! # Output Format
//!
//! - Signed 16-bit PCM (S16), interleaved for stereo
//! - Sample rate and channels come from the identification header

use lewton::audio::{read_audio_packet, PreviousWindowRight};
use lewton::header::{read_header_comment, read_header_ident, read_header_setup};
use lewton::header::{IdentHeader, SetupHeader};
use rust_media_core::{
    Decoder, DecoderCapabilities, Error, Frame, MediaType, Packet, Result, SampleFormat, StreamInfo,
};
use std::collections::VecDeque;

/// Vorbis audio decoder using lewton.
///
/// Decodes Vorbis audio packets into interleaved S16 PCM frames.
///
/// # Container Support
///
/// Vorbis in WebM/MKV: the demuxer passes the Xiph-laced header block as
/// `StreamInfo.extra_data`. The decoder parses identification, comment, and
/// setup headers from this block at construction.
///
/// # PTS Handling
///
/// Each input packet's PTS is forwarded to the corresponding output frame.
pub struct VorbisDecoder {
    stream_info: StreamInfo,
    ident: IdentHeader,
    setup: SetupHeader,
    pwr: PreviousWindowRight,
    buffered_frames: VecDeque<Frame>,
    flushed: bool,
}

impl VorbisDecoder {
    /// Creates a new Vorbis decoder from stream information.
    ///
    /// `stream_info.extra_data` must contain the Matroska CodecPrivate
    /// (Xiph-laced identification + comment + setup headers). If empty, the
    /// decoder cannot be initialized.
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "vorbis" {
            return Err(Error::Config(format!(
                "Expected vorbis codec, got: {}",
                stream_info.codec
            )));
        }

        if stream_info.extra_data.is_empty() {
            return Err(Error::Config(
                "Vorbis decoder requires CodecPrivate headers in extra_data".to_string(),
            ));
        }

        let (ident_pkt, comment_pkt, setup_pkt) =
            parse_xiph_laced_headers(&stream_info.extra_data)?;

        let ident = read_header_ident(ident_pkt).map_err(|e| {
            Error::InvalidData(format!("Vorbis identification header: {}", e))
        })?;
        let _comment = read_header_comment(comment_pkt).map_err(|e| {
            Error::InvalidData(format!("Vorbis comment header: {}", e))
        })?;
        let setup = read_header_setup(setup_pkt, ident.audio_channels, (ident.blocksize_0, ident.blocksize_1))
            .map_err(|e| {
                Error::InvalidData(format!("Vorbis setup header: {}", e))
            })?;

        let pwr = PreviousWindowRight::new();

        Ok(Self {
            stream_info,
            ident,
            setup,
            pwr,
            buffered_frames: VecDeque::new(),
            flushed: false,
        })
    }
}

impl Decoder for VorbisDecoder {
    fn codec(&self) -> &str {
        "vorbis"
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

        // Decode the Vorbis audio packet using lewton's low-level API.
        // Returns per-channel i16 sample vectors.
        let decoded: Vec<Vec<i16>> =
            read_audio_packet(&self.ident, &self.setup, data, &mut self.pwr)
                .map_err(|e| Error::Decode(format!("Vorbis decode: {}", e)))?;

        if decoded.is_empty() || decoded[0].is_empty() {
            return Ok(());
        }

        let channels = decoded.len();
        let samples_per_channel = decoded[0].len();
        let sample_rate = self.ident.audio_sample_rate;

        let mut frame = Frame::new_audio(
            sample_rate,
            channels,
            SampleFormat::S16,
            samples_per_channel,
        );

        // Interleave channel data into the frame buffer.
        if let Some(frame_data) = frame.plane_mut(0) {
            for s in 0..samples_per_channel {
                for (ch, channel_data) in decoded.iter().enumerate() {
                    let sample = channel_data[s];
                    let idx = (s * channels + ch) * 2;
                    let bytes = sample.to_le_bytes();
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
        self.pwr = PreviousWindowRight::new();
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

/// Parse the Matroska/WebM Vorbis CodecPrivate into three header packets.
///
/// The format uses Xiph-style lacing:
/// ```text
/// [num_packets_minus_1] [xiph_size_1] [xiph_size_2] [pkt1..] [pkt2..] [pkt3..]
/// ```
fn parse_xiph_laced_headers(data: &[u8]) -> Result<(&[u8], &[u8], &[u8])> {
    if data.is_empty() {
        return Err(Error::InvalidData(
            "Empty Vorbis CodecPrivate".to_string(),
        ));
    }

    let num_packets = data[0] as usize + 1;
    if num_packets != 3 {
        return Err(Error::InvalidData(format!(
            "Vorbis CodecPrivate must contain 3 packets, got {}",
            num_packets
        )));
    }

    // Read Xiph-laced sizes for the first two packets.
    // The third packet's size is the remainder.
    let mut offset = 1usize;
    let mut sizes = [0usize; 2];

    for size in &mut sizes {
        loop {
            if offset >= data.len() {
                return Err(Error::InvalidData(
                    "Vorbis CodecPrivate truncated in size field".to_string(),
                ));
            }
            let byte = data[offset] as usize;
            offset += 1;
            *size += byte;
            if byte < 255 {
                break;
            }
        }
    }

    let pkt1_start = offset;
    let pkt1_end = pkt1_start + sizes[0];
    let pkt2_end = pkt1_end + sizes[1];

    if pkt2_end > data.len() {
        return Err(Error::InvalidData(
            "Vorbis CodecPrivate truncated in packet data".to_string(),
        ));
    }

    let pkt1 = &data[pkt1_start..pkt1_end];
    let pkt2 = &data[pkt1_end..pkt2_end];
    let pkt3 = &data[pkt2_end..];

    Ok((pkt1, pkt2, pkt3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::{AudioStreamParams, StreamParams};

    /// Build a minimal Xiph-laced CodecPrivate from raw header packets.
    fn build_codec_private(ident: &[u8], comment: &[u8], setup: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(2); // num_packets - 1

        // Xiph-lace the first two sizes
        let mut remaining = ident.len();
        while remaining >= 255 {
            buf.push(255);
            remaining -= 255;
        }
        buf.push(remaining as u8);

        remaining = comment.len();
        while remaining >= 255 {
            buf.push(255);
            remaining -= 255;
        }
        buf.push(remaining as u8);

        buf.extend_from_slice(ident);
        buf.extend_from_slice(comment);
        buf.extend_from_slice(setup);
        buf
    }

    #[test]
    fn test_vorbis_decoder_wrong_codec() {
        let audio_params = AudioStreamParams::new(44100, 2, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "opus".to_string())
            .with_params(StreamParams::Audio(audio_params));
        assert!(VorbisDecoder::new(stream_info).is_err());
    }

    #[test]
    fn test_vorbis_decoder_missing_extra_data() {
        let audio_params = AudioStreamParams::new(44100, 2, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "vorbis".to_string())
            .with_params(StreamParams::Audio(audio_params));
        assert!(VorbisDecoder::new(stream_info).is_err());
    }

    #[test]
    fn test_xiph_lacing_parse() {
        let ident = vec![0x01; 30]; // identification header
        let comment = vec![0x03; 50]; // comment header
        let setup = vec![0x05; 200]; // setup header
        let codec_private = build_codec_private(&ident, &comment, &setup);

        let (p1, p2, p3) = parse_xiph_laced_headers(&codec_private).unwrap();
        assert_eq!(p1, &ident[..]);
        assert_eq!(p2, &comment[..]);
        assert_eq!(p3, &setup[..]);
    }

    #[test]
    fn test_xiph_lacing_large_packet() {
        // Test Xiph lacing with a packet > 255 bytes
        let ident = vec![0x01; 300]; // > 255, requires multi-byte lacing
        let comment = vec![0x03; 10];
        let setup = vec![0x05; 500];
        let codec_private = build_codec_private(&ident, &comment, &setup);

        let (p1, p2, p3) = parse_xiph_laced_headers(&codec_private).unwrap();
        assert_eq!(p1.len(), 300);
        assert_eq!(p2.len(), 10);
        assert_eq!(p3.len(), 500);
    }
}
