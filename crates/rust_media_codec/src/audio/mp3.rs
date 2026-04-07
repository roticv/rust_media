//! MP3 audio decoder implementation
//!
//! Provides MP3 decoding via the minimp3 crate (MIT licensed).
//! Decode-only — MP3 encoding is not supported.
//!
//! # Output Format
//!
//! - Signed 16-bit PCM (S16), interleaved for stereo
//! - Sample rate and channels are determined per-frame from the MP3 data

use minimp3::{Decoder as MiniMp3Decoder, Error as Mp3Error};
use rust_media_core::{
    Decoder, DecoderCapabilities, Error, Frame, MediaType, Packet, Result, SampleFormat, StreamInfo,
};
use std::io::Cursor;

/// MP3 Audio Decoder
///
/// Decodes MP3 audio packets into PCM frames using minimp3.
/// Each packet is expected to contain one or more complete MP3 frames.
pub struct Mp3Decoder {
    stream_info: StreamInfo,
    flushed: bool,
    buffered_frames: Vec<Frame>,
}

impl Mp3Decoder {
    /// Creates a new MP3 decoder
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "mp3" {
            return Err(Error::Config(format!(
                "Expected mp3 codec, got: {}",
                stream_info.codec
            )));
        }

        Ok(Self {
            stream_info,
            flushed: false,
            buffered_frames: Vec::new(),
        })
    }
}

impl Decoder for Mp3Decoder {
    fn codec(&self) -> &str {
        "mp3"
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

        let mut decoder = MiniMp3Decoder::new(Cursor::new(data));
        let pts = packet.pts();

        loop {
            match decoder.next_frame() {
                Ok(mp3_frame) => {
                    let channels = mp3_frame.channels;
                    let sample_rate = mp3_frame.sample_rate as u32;
                    let samples_per_channel = mp3_frame.data.len() / channels;

                    let mut frame = Frame::new_audio(
                        sample_rate,
                        channels,
                        SampleFormat::S16,
                        samples_per_channel,
                    );

                    // Copy interleaved i16 samples to frame data
                    let frame_data = frame.plane_mut(0).ok_or_else(|| {
                        Error::InvalidState("Frame missing data plane".to_string())
                    })?;

                    for (i, &sample) in mp3_frame.data.iter().enumerate() {
                        let bytes = sample.to_le_bytes();
                        frame_data[i * 2] = bytes[0];
                        frame_data[i * 2 + 1] = bytes[1];
                    }

                    if let Some(p) = pts {
                        frame.set_pts(Some(p));
                    }

                    let duration =
                        (samples_per_channel as u64 * 1_000_000) / sample_rate as u64;
                    frame = frame.with_duration(duration as i64);

                    self.buffered_frames.push(frame);
                }
                Err(Mp3Error::Eof) => break,
                Err(Mp3Error::InsufficientData) => break,
                Err(Mp3Error::SkippedData) => continue,
                Err(Mp3Error::Io(e)) => {
                    return Err(Error::Decode(format!("MP3 IO error: {}", e)));
                }
            }
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if !self.buffered_frames.is_empty() {
            Ok(self.buffered_frames.remove(0))
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
        self.flushed = false;
        self.buffered_frames.clear();
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
