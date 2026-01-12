//! WAV file muxer implementation
//!
//! WAV (Waveform Audio File Format) muxer for writing PCM audio data.
//! Uses the RIFF (Resource Interchange File Format) structure.

use rust_media_core::{
    AudioStreamParams, Error, MediaType, Muxer, Packet, Result, SampleFormat, StreamInfo,
    StreamParams,
};
use std::io::{Seek, SeekFrom, Write};

/// WAV file muxer
///
/// Writes PCM audio data to WAV files using the streaming Muxer API.
pub struct WavMuxer<W> {
    writer: W,
    streams: Vec<StreamInfo>,
    header_written: bool,
    trailer_written: bool,
    data_bytes_written: u64,
    position: u64,
}

impl<W: Write + Seek> WavMuxer<W> {
    /// Creates a new WAV muxer with the given writer
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            streams: Vec::new(),
            header_written: false,
            trailer_written: false,
            data_bytes_written: 0,
            position: 0,
        }
    }

    /// Writes the RIFF header
    fn write_riff_header(&mut self) -> Result<()> {
        // Write "RIFF" signature
        self.writer.write_all(b"RIFF")?;
        self.position += 4;

        // Write placeholder file size (will update in write_trailer)
        self.writer.write_all(&0u32.to_le_bytes())?;
        self.position += 4;

        // Write "WAVE" format
        self.writer.write_all(b"WAVE")?;
        self.position += 4;

        Ok(())
    }

    /// Writes the fmt chunk
    fn write_fmt_chunk(&mut self, audio_params: &AudioStreamParams) -> Result<()> {
        // Validate we have exactly one audio stream
        if self.streams.len() != 1 {
            return Err(Error::InvalidData(
                "WAV files must have exactly one audio stream".to_string(),
            ));
        }

        // Write "fmt " chunk ID
        self.writer.write_all(b"fmt ")?;
        self.position += 4;

        // Write chunk size (16 bytes for PCM)
        self.writer.write_all(&16u32.to_le_bytes())?;
        self.position += 4;

        // Write audio format (1 = PCM)
        self.writer.write_all(&1u16.to_le_bytes())?;
        self.position += 2;

        // Write number of channels
        self.writer
            .write_all(&(audio_params.channels as u16).to_le_bytes())?;
        self.position += 2;

        // Write sample rate
        self.writer.write_all(&audio_params.sample_rate.to_le_bytes())?;
        self.position += 4;

        // Calculate and write byte rate
        let bits_per_sample = match audio_params.sample_format {
            SampleFormat::U8 => 8,
            SampleFormat::S16 => 16,
            SampleFormat::S32 => 32,
            _ => {
                return Err(Error::Unsupported(format!(
                    "Sample format {:?} not supported for WAV",
                    audio_params.sample_format
                )))
            }
        };

        let byte_rate =
            audio_params.sample_rate * audio_params.channels as u32 * bits_per_sample / 8;
        self.writer.write_all(&byte_rate.to_le_bytes())?;
        self.position += 4;

        // Write block align
        let block_align = (audio_params.channels * bits_per_sample as usize / 8) as u16;
        self.writer.write_all(&block_align.to_le_bytes())?;
        self.position += 2;

        // Write bits per sample
        self.writer.write_all(&(bits_per_sample as u16).to_le_bytes())?;
        self.position += 2;

        Ok(())
    }

    /// Writes the data chunk header
    fn write_data_chunk_header(&mut self) -> Result<()> {
        // Write "data" chunk ID
        self.writer.write_all(b"data")?;
        self.position += 4;

        // Write placeholder data size (will update in write_trailer)
        self.writer.write_all(&0u32.to_le_bytes())?;
        self.position += 4;

        Ok(())
    }
}

impl<W: Write + Seek> Muxer for WavMuxer<W> {
    fn add_stream(&mut self, stream_info: StreamInfo) -> Result<usize> {
        if self.header_written {
            return Err(Error::InvalidData(
                "Cannot add streams after header is written".to_string(),
            ));
        }

        // WAV only supports one audio stream
        if !self.streams.is_empty() {
            return Err(Error::InvalidData(
                "WAV files can only have one audio stream".to_string(),
            ));
        }

        // Validate it's an audio stream
        if !matches!(stream_info.params, StreamParams::Audio(_)) {
            return Err(Error::InvalidData(
                "WAV files only support audio streams".to_string(),
            ));
        }

        // Validate codec is PCM
        if stream_info.codec != "pcm" {
            return Err(Error::Unsupported(format!(
                "WAV muxer only supports PCM codec, got: {}",
                stream_info.codec
            )));
        }

        let stream_index = self.streams.len();
        self.streams.push(stream_info);
        Ok(stream_index)
    }

    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }

    fn write_header(&mut self) -> Result<()> {
        if self.header_written {
            return Err(Error::InvalidData(
                "Header already written".to_string(),
            ));
        }

        if self.streams.is_empty() {
            return Err(Error::InvalidData(
                "No streams added, cannot write header".to_string(),
            ));
        }

        // Get audio parameters (clone to avoid borrow checker issues)
        let audio_params = match &self.streams[0].params {
            StreamParams::Audio(params) => params.clone(),
            _ => unreachable!(),
        };

        // Write RIFF header
        self.write_riff_header()?;

        // Write fmt chunk
        self.write_fmt_chunk(&audio_params)?;

        // Write data chunk header
        self.write_data_chunk_header()?;

        self.header_written = true;
        Ok(())
    }

    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        if !self.header_written {
            return Err(Error::InvalidData(
                "Header must be written before packets".to_string(),
            ));
        }

        if self.trailer_written {
            return Err(Error::InvalidData(
                "Cannot write packets after trailer".to_string(),
            ));
        }

        // Validate stream index
        if packet.stream_index() >= self.streams.len() {
            return Err(Error::InvalidData(format!(
                "Invalid stream index: {}",
                packet.stream_index()
            )));
        }

        // Validate media type
        if packet.media_type() != MediaType::Audio {
            return Err(Error::InvalidData(
                "WAV muxer only supports audio packets".to_string(),
            ));
        }

        // Write PCM data
        self.writer.write_all(packet.data())?;
        let bytes_written = packet.data().len() as u64;
        self.data_bytes_written += bytes_written;
        self.position += bytes_written;

        Ok(())
    }

    fn write_trailer(&mut self) -> Result<()> {
        if !self.header_written {
            return Err(Error::InvalidData(
                "Header must be written before trailer".to_string(),
            ));
        }

        if self.trailer_written {
            return Err(Error::InvalidData(
                "Trailer already written".to_string(),
            ));
        }

        // Update RIFF chunk size (file size - 8)
        // RIFF size = 4 (WAVE) + 8 (fmt ) + 16 (fmt data) + 8 (data) + data_size
        let riff_size = 4 + 8 + 16 + 8 + self.data_bytes_written;
        self.writer.seek(SeekFrom::Start(4))?;
        self.writer.write_all(&(riff_size as u32).to_le_bytes())?;

        // Update data chunk size
        // Position of data size field = 12 (RIFF header) + 24 (fmt chunk) + 4 (data ID)
        self.writer.seek(SeekFrom::Start(40))?;
        self.writer
            .write_all(&(self.data_bytes_written as u32).to_le_bytes())?;

        // Seek to end of file
        self.writer.seek(SeekFrom::End(0))?;

        self.trailer_written = true;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }

    fn position(&self) -> u64 {
        self.position
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::SampleFormat;
    use std::io::Cursor;

    fn create_test_audio_stream() -> StreamInfo {
        let audio_params = AudioStreamParams::new(48000, 2, SampleFormat::S16);
        StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(StreamParams::Audio(audio_params))
    }

    fn create_test_pcm_packet(stream_index: usize, data: Vec<u8>) -> Packet {
        Packet::new(data, stream_index, MediaType::Audio)
            .with_pts(0)
            .with_duration(1000)
    }

    #[test]
    fn test_wav_muxer_creation() {
        let cursor = Cursor::new(Vec::new());
        let muxer = WavMuxer::new(cursor);
        assert_eq!(muxer.streams().len(), 0);
        assert!(!muxer.header_written);
    }

    #[test]
    fn test_wav_muxer_add_stream() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(cursor);

        let stream = create_test_audio_stream();
        let index = muxer.add_stream(stream).unwrap();
        assert_eq!(index, 0);
        assert_eq!(muxer.streams().len(), 1);
    }

    #[test]
    fn test_wav_muxer_rejects_multiple_streams() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(cursor);

        let stream1 = create_test_audio_stream();
        muxer.add_stream(stream1).unwrap();

        let stream2 = create_test_audio_stream();
        let result = muxer.add_stream(stream2);
        assert!(result.is_err());
    }

    #[test]
    fn test_wav_muxer_write_header() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(cursor);

        let stream = create_test_audio_stream();
        muxer.add_stream(stream).unwrap();
        muxer.write_header().unwrap();

        assert!(muxer.header_written);
    }

    #[test]
    fn test_wav_muxer_write_packet() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(cursor);

        let stream = create_test_audio_stream();
        muxer.add_stream(stream).unwrap();
        muxer.write_header().unwrap();

        let packet = create_test_pcm_packet(0, vec![0u8; 100]);
        muxer.write_packet(&packet).unwrap();

        assert_eq!(muxer.data_bytes_written, 100);
    }

    #[test]
    fn test_wav_muxer_full_pipeline() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(cursor);

        // Add stream
        let stream = create_test_audio_stream();
        muxer.add_stream(stream).unwrap();

        // Write header
        muxer.write_header().unwrap();

        // Write some packets
        for _ in 0..5 {
            let packet = create_test_pcm_packet(0, vec![0u8; 100]);
            muxer.write_packet(&packet).unwrap();
        }

        // Write trailer
        muxer.write_trailer().unwrap();

        // Verify the output
        let output = muxer.writer.into_inner();

        // Check RIFF header
        assert_eq!(&output[0..4], b"RIFF");
        assert_eq!(&output[8..12], b"WAVE");

        // Check fmt chunk
        assert_eq!(&output[12..16], b"fmt ");

        // Check data chunk
        assert_eq!(&output[36..40], b"data");

        // Verify data size was updated (should be 500 bytes)
        let data_size = u32::from_le_bytes([output[40], output[41], output[42], output[43]]);
        assert_eq!(data_size, 500);
    }

    #[test]
    fn test_wav_muxer_requires_stream_before_header() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(cursor);

        let result = muxer.write_header();
        assert!(result.is_err());
    }

    #[test]
    fn test_wav_muxer_requires_header_before_packet() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(cursor);

        let stream = create_test_audio_stream();
        muxer.add_stream(stream).unwrap();

        let packet = create_test_pcm_packet(0, vec![0u8; 100]);
        let result = muxer.write_packet(&packet);
        assert!(result.is_err());
    }
}
