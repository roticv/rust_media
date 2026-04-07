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
///
/// Uses a streaming approach: packet data is accumulated in a buffer, and
/// minimp3 decodes from the concatenated stream. This is necessary because
/// minimp3 needs to see ahead to the next frame's sync word to validate
/// the current frame.
pub struct Mp3Decoder {
    stream_info: StreamInfo,
    flushed: bool,
    buffered_frames: Vec<Frame>,
    /// Accumulated MP3 data from packets
    data_buf: Vec<u8>,
    /// PTS values corresponding to each packet appended
    pts_queue: Vec<Option<i64>>,
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
            data_buf: Vec::new(),
            pts_queue: Vec::new(),
        })
    }

    /// Decode as many complete frames as possible from the accumulated buffer.
    fn decode_available(&mut self) {
        if self.data_buf.is_empty() {
            return;
        }

        let mut decoder = MiniMp3Decoder::new(Cursor::new(&self.data_buf));
        let mut consumed = 0usize;

        loop {
            match decoder.next_frame() {
                Ok(mp3_frame) => {
                    consumed = decoder.reader().position() as usize;

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
                    if let Some(frame_data) = frame.plane_mut(0) {
                        for (i, &sample) in mp3_frame.data.iter().enumerate() {
                            let bytes = sample.to_le_bytes();
                            frame_data[i * 2] = bytes[0];
                            frame_data[i * 2 + 1] = bytes[1];
                        }
                    }

                    // Assign PTS from queue
                    if let Some(pts) = self.pts_queue.first().copied() {
                        frame.set_pts(pts);
                        self.pts_queue.remove(0);
                    }

                    let duration =
                        (samples_per_channel as u64 * 1_000_000) / sample_rate as u64;
                    frame = frame.with_duration(duration as i64);

                    self.buffered_frames.push(frame);
                }
                Err(Mp3Error::SkippedData) => {
                    consumed = decoder.reader().position() as usize;
                    continue;
                }
                Err(Mp3Error::Eof) | Err(Mp3Error::InsufficientData) => {
                    // Not enough data for another frame — keep remainder for next packet
                    break;
                }
                Err(Mp3Error::Io(_)) => break,
            }
        }

        // Remove consumed data from the buffer
        if consumed > 0 {
            self.data_buf.drain(..consumed);
        }
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

        self.data_buf.extend_from_slice(data);
        self.pts_queue.push(packet.pts());
        self.decode_available();

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
        // Pad buffer to let minimp3 decode the last frame
        if !self.data_buf.is_empty() {
            self.data_buf.extend_from_slice(&[0u8; 1536]);
            self.decode_available();
        }
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.flushed = false;
        self.buffered_frames.clear();
        self.data_buf.clear();
        self.pts_queue.clear();
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
