//! WAV file demuxer implementation
//!
//! WAV (Waveform Audio File Format) is a simple container for PCM audio data.
//! It uses the RIFF (Resource Interchange File Format) structure.

use rust_media_core::{
    AudioStreamParams, ContainerInfo, Demuxer, Error, MediaType, Packet, Result, SampleFormat,
    StreamInfo, StreamParams,
};
use std::io::{Read, Seek, SeekFrom};

/// WAV file demuxer
///
/// Reads PCM audio data from WAV files using the streaming Demuxer API.
pub struct WavDemuxer<R> {
    reader: R,
    stream_info: StreamInfo,
    container_info: ContainerInfo,
    data_start: u64,
    data_size: u64,
    current_position: u64,
    bytes_per_frame: usize,
}

impl<R: Read + Seek> WavDemuxer<R> {
    /// Opens a WAV file for demuxing
    pub fn open(mut reader: R) -> Result<Self> {
        // Parse RIFF header
        let mut riff_header = [0u8; 12];
        reader.read_exact(&mut riff_header)?;

        // Check "RIFF" signature
        if &riff_header[0..4] != b"RIFF" {
            return Err(Error::InvalidData("Not a RIFF file".to_string()));
        }

        // Check "WAVE" format
        if &riff_header[8..12] != b"WAVE" {
            return Err(Error::InvalidData("Not a WAVE file".to_string()));
        }

        let _file_size = u32::from_le_bytes([
            riff_header[4],
            riff_header[5],
            riff_header[6],
            riff_header[7],
        ]) as u64
            + 8; // Add 8 for "RIFF" and size fields

        // Find and parse "fmt " chunk
        let (audio_params, bytes_per_frame) = Self::parse_fmt_chunk(&mut reader)?;

        // Find "data" chunk
        let (data_start, data_size) = Self::find_data_chunk(&mut reader)?;

        // Calculate duration
        let total_frames = data_size / bytes_per_frame as u64;
        let duration_us = (total_frames * 1_000_000) / audio_params.sample_rate as u64;

        // Create stream info
        let stream_info = StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_time_base(1, 1_000_000) // Microseconds
            .with_duration(duration_us as i64)
            .with_params(StreamParams::Audio(audio_params));

        // Create container info
        let container_info = ContainerInfo {
            format_name: "wav".to_string(),
            duration: Some(duration_us as i64),
            bitrate: None,
            metadata: rust_media_core::stream::Metadata::new(),
        };

        Ok(Self {
            reader,
            stream_info,
            container_info,
            data_start,
            data_size,
            current_position: 0,
            bytes_per_frame,
        })
    }

    /// Parses the "fmt " chunk
    fn parse_fmt_chunk(reader: &mut R) -> Result<(AudioStreamParams, usize)> {
        loop {
            let mut chunk_header = [0u8; 8];
            reader.read_exact(&mut chunk_header)?;

            let chunk_id = &chunk_header[0..4];
            let chunk_size = u32::from_le_bytes([
                chunk_header[4],
                chunk_header[5],
                chunk_header[6],
                chunk_header[7],
            ]) as u64;

            if chunk_id == b"fmt " {
                let mut fmt_data = vec![0u8; chunk_size as usize];
                reader.read_exact(&mut fmt_data)?;

                // Parse fmt chunk
                let audio_format = u16::from_le_bytes([fmt_data[0], fmt_data[1]]);
                let num_channels = u16::from_le_bytes([fmt_data[2], fmt_data[3]]) as usize;
                let sample_rate = u32::from_le_bytes([fmt_data[4], fmt_data[5], fmt_data[6], fmt_data[7]]);
                let bits_per_sample = u16::from_le_bytes([fmt_data[14], fmt_data[15]]) as usize;

                // Only support PCM (format 1)
                if audio_format != 1 {
                    return Err(Error::Unsupported(format!(
                        "Only PCM (format 1) is supported, got format {}",
                        audio_format
                    )));
                }

                // Determine sample format
                let sample_format = match bits_per_sample {
                    8 => SampleFormat::U8,
                    16 => SampleFormat::S16,
                    32 => SampleFormat::S32,
                    _ => {
                        return Err(Error::Unsupported(format!(
                            "Unsupported bit depth: {}",
                            bits_per_sample
                        )))
                    }
                };

                let bytes_per_frame = (bits_per_sample / 8) * num_channels;

                let audio_params = AudioStreamParams::new(sample_rate, num_channels, sample_format);

                return Ok((audio_params, bytes_per_frame));
            } else {
                // Skip this chunk
                reader.seek(SeekFrom::Current(chunk_size as i64))?;
            }
        }
    }

    /// Finds the "data" chunk and returns its position and size
    fn find_data_chunk(reader: &mut R) -> Result<(u64, u64)> {
        loop {
            let mut chunk_header = [0u8; 8];
            match reader.read_exact(&mut chunk_header) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(Error::InvalidData("Data chunk not found".to_string()));
                }
                Err(e) => return Err(e.into()),
            }

            let chunk_id = &chunk_header[0..4];
            let chunk_size = u32::from_le_bytes([
                chunk_header[4],
                chunk_header[5],
                chunk_header[6],
                chunk_header[7],
            ]) as u64;

            if chunk_id == b"data" {
                let data_start = reader.stream_position()?;
                return Ok((data_start, chunk_size));
            } else {
                // Skip this chunk
                reader.seek(SeekFrom::Current(chunk_size as i64))?;
            }
        }
    }
}

impl<R: Read + Seek> Demuxer for WavDemuxer<R> {
    fn container_info(&self) -> Result<ContainerInfo> {
        Ok(self.container_info.clone())
    }

    fn streams(&self) -> Result<Vec<StreamInfo>> {
        Ok(vec![self.stream_info.clone()])
    }

    fn stream_info(&self, stream_index: usize) -> Result<StreamInfo> {
        if stream_index != 0 {
            return Err(Error::InvalidData(format!(
                "WAV files have only one stream, requested index: {}",
                stream_index
            )));
        }
        Ok(self.stream_info.clone())
    }

    fn read_packet(&mut self) -> Result<Packet> {
        // Check if we've reached the end
        if self.current_position >= self.data_size {
            return Err(Error::EndOfStream);
        }

        // Read chunk size (e.g., 4096 bytes at a time for streaming)
        let chunk_size = 4096.min((self.data_size - self.current_position) as usize);
        let mut data = vec![0u8; chunk_size];

        self.reader.read_exact(&mut data)?;

        // Calculate timestamp based on position
        let audio_params = match &self.stream_info.params {
            StreamParams::Audio(params) => params,
            _ => unreachable!(),
        };

        let frames_read = self.current_position / self.bytes_per_frame as u64;
        let pts = (frames_read * 1_000_000) / audio_params.sample_rate as u64;

        let num_frames = chunk_size / self.bytes_per_frame;
        let duration = (num_frames as u64 * 1_000_000) / audio_params.sample_rate as u64;

        self.current_position += chunk_size as u64;

        let packet = Packet::new(data, 0, MediaType::Audio)
            .with_pts(pts as i64)
            .with_duration(duration as i64);

        Ok(packet)
    }

    fn seek(&mut self, timestamp_us: i64) -> Result<()> {
        let audio_params = match &self.stream_info.params {
            StreamParams::Audio(params) => params,
            _ => unreachable!(),
        };

        // Convert timestamp to frame number
        let frame_number = (timestamp_us as u64 * audio_params.sample_rate as u64) / 1_000_000;
        let byte_offset = frame_number * self.bytes_per_frame as u64;

        // Ensure we don't seek past the end
        let byte_offset = byte_offset.min(self.data_size);

        // Seek to position
        self.reader
            .seek(SeekFrom::Start(self.data_start + byte_offset))?;
        self.current_position = byte_offset;

        Ok(())
    }

    fn seek_stream(&mut self, stream_index: usize, timestamp: i64) -> Result<()> {
        if stream_index != 0 {
            return Err(Error::InvalidData(format!(
                "WAV files have only one stream, requested index: {}",
                stream_index
            )));
        }
        self.seek(timestamp)
    }

    fn position(&self) -> u64 {
        self.data_start + self.current_position
    }

    fn size(&self) -> Option<u64> {
        Some(self.data_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn create_test_wav_file() -> Vec<u8> {
        let mut data = Vec::new();

        // RIFF header
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&36u32.to_le_bytes()); // File size - 8
        data.extend_from_slice(b"WAVE");

        // fmt chunk
        data.extend_from_slice(b"fmt ");
        data.extend_from_slice(&16u32.to_le_bytes()); // Chunk size
        data.extend_from_slice(&1u16.to_le_bytes()); // Audio format (PCM)
        data.extend_from_slice(&2u16.to_le_bytes()); // Num channels (stereo)
        data.extend_from_slice(&48000u32.to_le_bytes()); // Sample rate
        data.extend_from_slice(&192000u32.to_le_bytes()); // Byte rate
        data.extend_from_slice(&4u16.to_le_bytes()); // Block align
        data.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample

        // data chunk
        data.extend_from_slice(b"data");
        data.extend_from_slice(&8u32.to_le_bytes()); // Data size
        // 2 frames of stereo 16-bit audio
        data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);

        data
    }

    #[test]
    fn test_wav_demuxer_creation() {
        let wav_data = create_test_wav_file();
        let cursor = Cursor::new(wav_data);
        let demuxer = WavDemuxer::open(cursor).unwrap();

        let streams = demuxer.streams().unwrap();
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].codec, "pcm");
    }

    #[test]
    fn test_wav_demuxer_stream_info() {
        let wav_data = create_test_wav_file();
        let cursor = Cursor::new(wav_data);
        let demuxer = WavDemuxer::open(cursor).unwrap();

        let stream_info = demuxer.stream_info(0).unwrap();
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
    fn test_wav_demuxer_container_info() {
        let wav_data = create_test_wav_file();
        let cursor = Cursor::new(wav_data);
        let demuxer = WavDemuxer::open(cursor).unwrap();

        let container_info = demuxer.container_info().unwrap();
        assert_eq!(container_info.format_name, "wav");
        assert!(container_info.duration.is_some());
    }

    #[test]
    fn test_wav_demuxer_invalid_file() {
        let invalid_data = vec![0u8; 100];
        let cursor = Cursor::new(invalid_data);
        let result = WavDemuxer::open(cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_wav_demuxer_read_packet() {
        let wav_data = create_test_wav_file();
        let cursor = Cursor::new(wav_data);
        let mut demuxer = WavDemuxer::open(cursor).unwrap();

        let packet = demuxer.read_packet().unwrap();
        assert_eq!(packet.stream_index(), 0);
        assert!(packet.pts().is_some());
        assert_eq!(packet.data().len(), 8); // All data fits in one packet
    }

    #[test]
    fn test_wav_demuxer_end_of_stream() {
        let wav_data = create_test_wav_file();
        let cursor = Cursor::new(wav_data);
        let mut demuxer = WavDemuxer::open(cursor).unwrap();

        // Read the only packet
        let _ = demuxer.read_packet().unwrap();

        // Next read should return EndOfStream
        match demuxer.read_packet() {
            Err(Error::EndOfStream) => {}
            _ => panic!("Expected EndOfStream"),
        }
    }
}
