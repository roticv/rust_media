//! WebM file muxer implementation
//!
//! WebM muxer for writing audio (Opus, Vorbis) and video (VP8, VP9, AV1) to WebM containers using EBML/Matroska format.

use crate::webm::ebml::element_id;
use crate::webm::writer::{
    write_binary_element, write_float_element, write_master_header,
    write_master_header_unknown_size, write_string_element, write_uint_element,
};
use byteorder::WriteBytesExt;
use rust_media_core::{
    AudioStreamParams, Error, MediaType, Muxer, Packet, Result, StreamInfo, StreamParams,
};
use std::io::{Seek, SeekFrom, Write};

/// WebM file muxer
///
/// Writes audio (Opus, Vorbis) and video (VP8, VP9, AV1) data to WebM files using the streaming Muxer API.
pub struct WebmMuxer<W> {
    writer: W,
    streams: Vec<StreamInfo>,
    header_written: bool,
    trailer_written: bool,
    position: u64,
    timecode_scale: u64, // Nanoseconds per tick (default: 1_000_000 = 1ms)
    cluster_timecode: i64,
    cluster_start_position: u64,
    cluster_packet_count: usize,
    max_cluster_duration_ms: i64,
    segment_start_position: u64,
}

impl<W: Write + Seek> WebmMuxer<W> {
    /// Creates a new WebM muxer with the given writer
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            streams: Vec::new(),
            header_written: false,
            trailer_written: false,
            position: 0,
            timecode_scale: 1_000_000, // 1ms per tick (WebM standard)
            cluster_timecode: 0,
            cluster_start_position: 0,
            cluster_packet_count: 0,
            max_cluster_duration_ms: 5000, // 5 seconds max per cluster
            segment_start_position: 0,
        }
    }

    /// Writes the EBML header
    fn write_ebml_header(&mut self) -> Result<()> {
        // Calculate EBML header size
        let ebml_version_size = 2 + 1 + 1; // ID (0x4286 = 2 bytes) + size + value (1)
        let read_version_size = 2 + 1 + 1; // ID (0x42F7 = 2 bytes) + size + value (1)
        let max_id_length_size = 2 + 1 + 1; // ID (0x42F2 = 2 bytes) + size + value (4)
        let max_size_length_size = 2 + 1 + 1; // ID (0x42F3 = 2 bytes) + size + value (8)
        let doctype_size = 2 + 1 + 4; // ID (0x4282 = 2 bytes) + size + "webm"
        let doctype_version_size = 2 + 1 + 1; // ID (0x4287 = 2 bytes) + size + value (2)
        let doctype_read_version_size = 2 + 1 + 1; // ID (0x4285 = 2 bytes) + size + value (2)

        let total_size = ebml_version_size
            + read_version_size
            + max_id_length_size
            + max_size_length_size
            + doctype_size
            + doctype_version_size
            + doctype_read_version_size;

        // Write EBML master element
        write_master_header(&mut self.writer, element_id::EBML, total_size as u64)?;

        // EBML Version
        write_uint_element(&mut self.writer, element_id::EBML_VERSION, 1)?;

        // EBML Read Version
        write_uint_element(&mut self.writer, element_id::EBML_READ_VERSION, 1)?;

        // EBML Max ID Length
        write_uint_element(&mut self.writer, element_id::EBML_MAX_ID_LENGTH, 4)?;

        // EBML Max Size Length
        write_uint_element(&mut self.writer, element_id::EBML_MAX_SIZE_LENGTH, 8)?;

        // Doc Type
        write_string_element(&mut self.writer, element_id::DOC_TYPE, "webm")?;

        // Doc Type Version
        write_uint_element(&mut self.writer, element_id::DOC_TYPE_VERSION, 2)?;

        // Doc Type Read Version
        write_uint_element(&mut self.writer, element_id::DOC_TYPE_READ_VERSION, 2)?;

        // Update position to actual stream position
        self.position = self.writer.stream_position()?;

        Ok(())
    }

    /// Writes the Segment header (with unknown size for streaming)
    fn write_segment_header(&mut self) -> Result<()> {
        // Get actual position before writing segment header
        self.segment_start_position = self.writer.stream_position()?;

        // Write segment header with unknown size
        write_master_header_unknown_size(&mut self.writer, element_id::SEGMENT)?;

        // Update position to actual stream position
        self.position = self.writer.stream_position()?;

        Ok(())
    }

    /// Writes the Info section
    fn write_info(&mut self) -> Result<()> {
        let muxing_app = "rust_media WebM Muxer";
        let writing_app = "rust_media v0.1.0";

        // Write Info contents to a buffer first to measure actual size
        let mut info_buffer = std::io::Cursor::new(Vec::new());

        // Timecode Scale (1ms = 1,000,000 nanoseconds)
        write_uint_element(
            &mut info_buffer,
            element_id::TIMECODE_SCALE,
            self.timecode_scale,
        )?;

        // Muxing App
        write_string_element(&mut info_buffer, element_id::MUXING_APP, muxing_app)?;

        // Writing App
        write_string_element(&mut info_buffer, element_id::WRITING_APP, writing_app)?;

        let info_data = info_buffer.into_inner();
        let info_size = info_data.len() as u64;

        // Write Info master element with actual measured size
        write_master_header(&mut self.writer, element_id::INFO, info_size)?;

        // Write the info data
        self.writer.write_all(&info_data)?;

        // Update position to actual stream position
        self.position = self.writer.stream_position()?;

        Ok(())
    }

    /// Writes the Tracks section
    fn write_tracks(&mut self) -> Result<()> {
        if self.streams.is_empty() {
            return Err(Error::InvalidData(
                "No streams added, cannot write tracks".to_string(),
            ));
        }

        // Clone streams to avoid borrow checker issues
        let streams = self.streams.clone();

        // Write all track entries to a buffer first to measure actual size
        let mut tracks_buffer = std::io::Cursor::new(Vec::new());

        for (idx, stream) in streams.iter().enumerate() {
            self.write_track_entry_to_buffer(&mut tracks_buffer, idx, stream)?;
        }

        let tracks_data = tracks_buffer.into_inner();
        let tracks_size = tracks_data.len() as u64;

        // Write Tracks master element with actual measured size
        write_master_header(&mut self.writer, element_id::TRACKS, tracks_size)?;

        // Write the tracks data
        self.writer.write_all(&tracks_data)?;

        // Update position to actual stream position
        self.position = self.writer.stream_position()?;

        Ok(())
    }

    /// Creates OpusHead for CodecPrivate
    fn create_opus_head(&self, params: &AudioStreamParams) -> Vec<u8> {
        let mut opus_head = Vec::new();

        // OpusHead structure (Opus encapsulation in Ogg/Matroska)
        opus_head.extend_from_slice(b"OpusHead"); // Magic signature (8 bytes)
        opus_head.push(1); // Version
        opus_head.push(params.channels as u8); // Channel count
        opus_head.extend_from_slice(&3840u16.to_le_bytes()); // Pre-skip (80ms at 48kHz)
        opus_head.extend_from_slice(&(params.sample_rate).to_le_bytes()); // Input sample rate
        opus_head.extend_from_slice(&0u16.to_le_bytes()); // Output gain (0 dB)
        opus_head.push(0); // Channel mapping family (0 = mono/stereo)

        opus_head
    }

    /// Writes a track entry to a buffer and returns the data
    fn write_track_entry_to_buffer<Writer: Write>(
        &self,
        writer: &mut Writer,
        track_idx: usize,
        stream: &StreamInfo,
    ) -> Result<()> {
        // Write track entry contents to a temp buffer first to measure size
        let mut entry_buffer = std::io::Cursor::new(Vec::new());

        // Track Number (1-indexed)
        write_uint_element(
            &mut entry_buffer,
            element_id::TRACK_NUMBER,
            (track_idx + 1) as u64,
        )?;

        // Track UID (use track number as UID)
        write_uint_element(
            &mut entry_buffer,
            element_id::TRACK_UID,
            (track_idx + 1) as u64,
        )?;

        // Track Type (1 = video, 2 = audio)
        let track_type = match stream.media_type {
            MediaType::Video => 1,
            MediaType::Audio => 2,
            _ => {
                return Err(Error::Unsupported(format!(
                    "WebM muxer does not support media type: {:?}",
                    stream.media_type
                )))
            }
        };
        write_uint_element(&mut entry_buffer, element_id::TRACK_TYPE, track_type)?;

        // Codec ID
        let codec_id_str = match stream.codec.as_str() {
            // Audio codecs
            "opus" => "A_OPUS",
            "vorbis" => "A_VORBIS",
            // Video codecs
            "vp8" => "V_VP8",
            "vp9" => "V_VP9",
            "av1" => "V_AV1",
            _ => {
                return Err(Error::Unsupported(format!(
                    "WebM muxer does not support codec: {}",
                    stream.codec
                )))
            }
        };
        write_string_element(&mut entry_buffer, element_id::CODEC_ID, codec_id_str)?;

        // Stream-specific parameters
        match &stream.params {
            StreamParams::Audio(params) => {
                // Opus-specific elements
                if stream.codec == "opus" {
                    // CodecPrivate (OpusHead)
                    let opus_head = self.create_opus_head(params);
                    write_binary_element(&mut entry_buffer, element_id::CODEC_PRIVATE, &opus_head)?;

                    // CodecDelay: 6.5ms for Opus (3120000 ns at 48kHz)
                    write_uint_element(&mut entry_buffer, element_id::CODEC_DELAY, 6500000)?;

                    // SeekPreRoll: 80ms for Opus (80000000 ns)
                    write_uint_element(&mut entry_buffer, element_id::SEEK_PRE_ROLL, 80000000)?;
                }

                // Audio element - write to temp buffer first
                let mut audio_buffer = std::io::Cursor::new(Vec::new());

                // Sampling Frequency
                write_float_element(
                    &mut audio_buffer,
                    element_id::SAMPLING_FREQUENCY,
                    params.sample_rate as f64,
                )?;

                // Channels
                write_uint_element(
                    &mut audio_buffer,
                    element_id::CHANNELS,
                    params.channels as u64,
                )?;

                let audio_data = audio_buffer.into_inner();
                let audio_size = audio_data.len() as u64;

                // Write Audio master element with actual size
                write_master_header(&mut entry_buffer, element_id::AUDIO, audio_size)?;
                entry_buffer.write_all(&audio_data)?;
            }
            StreamParams::Video(params) => {
                // Video element - write to temp buffer first
                let mut video_buffer = std::io::Cursor::new(Vec::new());

                // Pixel Width
                write_uint_element(
                    &mut video_buffer,
                    element_id::PIXEL_WIDTH,
                    params.width as u64,
                )?;

                // Pixel Height
                write_uint_element(
                    &mut video_buffer,
                    element_id::PIXEL_HEIGHT,
                    params.height as u64,
                )?;

                let video_data = video_buffer.into_inner();
                let video_size = video_data.len() as u64;

                // Write Video master element with actual size
                write_master_header(&mut entry_buffer, element_id::VIDEO, video_size)?;
                entry_buffer.write_all(&video_data)?;
            }
            _ => {}
        }

        // Now write TrackEntry with actual measured size
        let entry_data = entry_buffer.into_inner();
        let entry_size = entry_data.len() as u64;

        write_master_header(writer, element_id::TRACK_ENTRY, entry_size)?;
        writer.write_all(&entry_data)?;

        Ok(())
    }

    /// Starts a new cluster
    fn start_cluster(&mut self, timecode: i64) -> Result<()> {
        // Finalize previous cluster if needed
        if self.cluster_packet_count > 0 {
            self.finalize_cluster()?;
        }

        // Get actual position before writing
        self.cluster_start_position = self.writer.stream_position()?;
        self.position = self.cluster_start_position;

        // Write Cluster header (unknown size for streaming)
        write_master_header_unknown_size(&mut self.writer, element_id::CLUSTER)?;

        // Write Cluster Timecode
        write_uint_element(&mut self.writer, element_id::TIMECODE, timecode as u64)?;

        // Update position to actual stream position
        self.position = self.writer.stream_position()?;

        self.cluster_timecode = timecode;
        self.cluster_packet_count = 0;

        Ok(())
    }

    /// Finalizes the current cluster by updating its size
    fn finalize_cluster(&mut self) -> Result<()> {
        if self.cluster_packet_count == 0 {
            return Ok(());
        }

        // Calculate cluster size (content only, excluding ID and size field)
        let cluster_size = self.position - self.cluster_start_position - 12; // Subtract ID (4) + size field (8)

        // Seek back to update cluster size (right after the 4-byte Cluster ID)
        self.writer
            .seek(SeekFrom::Start(self.cluster_start_position + 4))?;

        // Write cluster size as proper 8-byte VINT (0x01 prefix + 7 bytes of size)
        self.writer.write_u8(0x01)?; // 8-byte VINT marker
        for i in (0..7).rev() {
            self.writer.write_u8(((cluster_size >> (i * 8)) & 0xFF) as u8)?;
        }

        // Seek back to end
        self.writer.seek(SeekFrom::Start(self.position))?;

        Ok(())
    }

    /// Writes a SimpleBlock for a packet
    fn write_simple_block(&mut self, packet: &Packet) -> Result<()> {
        let stream_idx = packet.stream_index();
        if stream_idx >= self.streams.len() {
            return Err(Error::InvalidData(format!(
                "Invalid stream index: {}",
                stream_idx
            )));
        }

        // Calculate relative timecode (within cluster)
        let packet_pts_ms = packet.pts().unwrap_or(0) / 1000; // Convert µs to ms
        let relative_timecode = (packet_pts_ms - self.cluster_timecode) as i16;

        // SimpleBlock structure:
        // - Track number (VINT)
        // - Timecode (2 bytes, signed)
        // - Flags (1 byte)
        // - Frame data

        let track_number = (stream_idx + 1) as u8;
        let flags = 0x80; // Keyframe flag (all audio frames are keyframes)

        let block_data_size = 1 + 2 + 1 + packet.data().len(); // track# + timecode + flags + data

        // Write SimpleBlock element
        write_master_header(&mut self.writer, element_id::SIMPLE_BLOCK, block_data_size as u64)?;

        // Track number (1-byte VINT for track 1-127)
        self.writer.write_u8(0x80 | track_number)?;

        // Relative timecode (big-endian signed 16-bit)
        self.writer
            .write_all(&relative_timecode.to_be_bytes())?;

        // Flags
        self.writer.write_u8(flags)?;

        // Frame data
        self.writer.write_all(packet.data())?;

        // Update position to actual stream position
        self.position = self.writer.stream_position()?;

        self.cluster_packet_count += 1;

        Ok(())
    }
}

impl<W: Write + Seek> Muxer for WebmMuxer<W> {
    fn add_stream(&mut self, stream_info: StreamInfo) -> Result<usize> {
        if self.header_written {
            return Err(Error::InvalidData(
                "Cannot add streams after header is written".to_string(),
            ));
        }

        // Validate stream has parameters
        if !matches!(
            stream_info.params,
            StreamParams::Audio(_) | StreamParams::Video(_)
        ) {
            return Err(Error::InvalidData(
                "WebM muxer requires audio or video stream parameters".to_string(),
            ));
        }

        // Validate codec is supported
        let supported_codecs = ["opus", "vorbis", "vp8", "vp9", "av1"];
        if !supported_codecs.contains(&stream_info.codec.as_str()) {
            return Err(Error::Unsupported(format!(
                "WebM muxer does not support codec: {}. Supported: {}",
                stream_info.codec,
                supported_codecs.join(", ")
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

        // Write EBML header
        self.write_ebml_header()?;

        // Write Segment header (unknown size for streaming)
        self.write_segment_header()?;

        // Write Info section
        self.write_info()?;

        // Write Tracks section
        self.write_tracks()?;

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

        // Validate media type
        if packet.media_type() != MediaType::Audio && packet.media_type() != MediaType::Video {
            return Err(Error::InvalidData(format!(
                "WebM muxer only supports audio and video packets, got: {:?}",
                packet.media_type()
            )));
        }

        // Get packet timestamp in milliseconds
        let packet_pts_ms = packet.pts().unwrap_or(0) / 1000;

        // Start a new cluster if:
        // 1. No cluster started yet, or
        // 2. Packet timestamp is too far from cluster start
        if self.cluster_packet_count == 0
            || (packet_pts_ms - self.cluster_timecode) > self.max_cluster_duration_ms
        {
            self.start_cluster(packet_pts_ms)?;
        }

        // Write the packet as a SimpleBlock
        self.write_simple_block(packet)?;

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

        // Finalize last cluster
        if self.cluster_packet_count > 0 {
            self.finalize_cluster()?;
        }

        // Update Segment size (for non-streaming use cases like file output)
        // Get current actual position
        let current_pos = self.writer.stream_position()?;

        // Calculate segment size (current position - segment start - header size)
        let segment_size = current_pos - self.segment_start_position - 12; // Subtract Segment header (4 + 8 bytes)

        // Seek back to update segment size
        self.writer
            .seek(SeekFrom::Start(self.segment_start_position + 4))?;

        // Write actual size as 8-byte VINT (0x01 prefix for 8-byte VINT)
        self.writer.write_u8(0x01)?;
        for i in (0..7).rev() {
            self.writer.write_u8(((segment_size >> (i * 8)) & 0xFF) as u8)?;
        }

        // Seek back to end
        self.writer.seek(SeekFrom::Start(current_pos))?;

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

    fn create_test_opus_stream() -> StreamInfo {
        let audio_params = AudioStreamParams::new(48000, 2, SampleFormat::S16);
        StreamInfo::new(0, MediaType::Audio, "opus".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(StreamParams::Audio(audio_params))
    }

    fn create_test_packet(stream_index: usize, pts: i64, data: Vec<u8>) -> Packet {
        Packet::new(data, stream_index, MediaType::Audio)
            .with_pts(pts)
            .with_duration(20000) // 20ms
    }

    #[test]
    fn test_webm_muxer_creation() {
        let cursor = Cursor::new(Vec::new());
        let muxer = WebmMuxer::new(cursor);
        assert_eq!(muxer.streams().len(), 0);
        assert!(!muxer.header_written);
    }

    #[test]
    fn test_webm_muxer_add_stream() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WebmMuxer::new(cursor);

        let stream = create_test_opus_stream();
        let index = muxer.add_stream(stream).unwrap();
        assert_eq!(index, 0);
        assert_eq!(muxer.streams().len(), 1);
    }

    #[test]
    fn test_webm_muxer_write_header() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WebmMuxer::new(cursor);

        let stream = create_test_opus_stream();
        muxer.add_stream(stream).unwrap();
        muxer.write_header().unwrap();

        assert!(muxer.header_written);
    }

    #[test]
    fn test_webm_muxer_full_pipeline() {
        let cursor = Cursor::new(Vec::new());
        let mut muxer = WebmMuxer::new(cursor);

        // Add stream
        let stream = create_test_opus_stream();
        muxer.add_stream(stream).unwrap();

        // Write header
        muxer.write_header().unwrap();

        // Write some packets
        for i in 0..10 {
            let pts = i * 20000; // 20ms intervals
            let packet = create_test_packet(0, pts, vec![0u8; 100]);
            muxer.write_packet(&packet).unwrap();
        }

        // Write trailer
        muxer.write_trailer().unwrap();

        // Verify the output
        let output = muxer.writer.into_inner();

        // Check EBML header
        assert_eq!(&output[0..4], &[0x1A, 0x45, 0xDF, 0xA3]); // EBML ID

        // Should contain "webm" doctype
        let output_str = String::from_utf8_lossy(&output);
        assert!(output_str.contains("webm"));
        assert!(output_str.contains("OpusHead"));
    }

    #[test]
    fn test_webm_muxer_opus_head() {
        let cursor = Cursor::new(Vec::new());
        let muxer = WebmMuxer::new(cursor);

        let params = AudioStreamParams::new(48000, 2, SampleFormat::S16);
        let opus_head = muxer.create_opus_head(&params);

        // Verify OpusHead structure
        assert_eq!(&opus_head[0..8], b"OpusHead");
        assert_eq!(opus_head[8], 1); // Version
        assert_eq!(opus_head[9], 2); // Channels
    }
}
