//! WebM demuxer implementation
//!
//! WebM is a subset of Matroska designed for web use, typically containing VP8/VP9/AV1 video
//! and Vorbis/Opus audio.

use crate::webm::ebml::{element_id, Element};
use rust_media_core::{
    AudioStreamParams, ContainerInfo, Demuxer, Error, MediaType, Packet, PixelFormat, Result,
    SampleFormat, StreamInfo, StreamParams, VideoStreamParams,
};
use std::io::{Read, Seek, SeekFrom};

/// WebM file demuxer
///
/// Reads audio/video data from WebM files using the streaming Demuxer API.
pub struct WebmDemuxer<R> {
    reader: R,
    streams: Vec<StreamInfo>,
    container_info: ContainerInfo,
    timecode_scale: u64, // nanoseconds per tick
    cluster_timecode: u64,
    current_position: u64,
    #[allow(dead_code)]
    segment_start: u64,
    tracks_parsed: bool,
}

#[derive(Debug)]
#[allow(dead_code)]
struct TrackInfo {
    track_number: u64,
    track_type: u8,
    codec_id: String,
    codec_private: Option<Vec<u8>>,
    audio_params: Option<AudioParams>,
    video_params: Option<VideoParams>,
}

#[derive(Debug)]
struct AudioParams {
    sampling_frequency: f64,
    channels: u64,
    bit_depth: Option<u64>,
}

#[derive(Debug)]
#[allow(dead_code)]
struct VideoParams {
    pixel_width: u64,
    pixel_height: u64,
    display_width: Option<u64>,
    display_height: Option<u64>,
    frame_rate: Option<f64>,
}

impl<R: Read + Seek> WebmDemuxer<R> {
    /// Opens a WebM/MKV file for demuxing
    pub fn open(mut reader: R) -> Result<Self> {
        // Parse EBML header
        let ebml_elem = Element::read(&mut reader)?;
        if ebml_elem.id != element_id::EBML {
            return Err(Error::InvalidData("Not a valid EBML file".to_string()));
        }

        // Parse EBML header to find DocType (distinguishes WebM from MKV)
        let header_data_start = reader.stream_position()?;
        let header_size = ebml_elem.size.unwrap_or(0);
        let header_end = header_data_start + header_size;
        let mut doc_type = String::from("matroska"); // default
        while reader.stream_position()? < header_end {
            let child = Element::read(&mut reader)?;
            if child.id == element_id::DOC_TYPE {
                doc_type = child.read_string(&mut reader)?;
            } else {
                child.skip(&mut reader)?;
            }
        }

        let format_name = if doc_type == "webm" {
            "webm".to_string()
        } else {
            "matroska".to_string()
        };

        // Find Segment element
        let segment_elem = Element::read(&mut reader)?;
        if segment_elem.id != element_id::SEGMENT {
            return Err(Error::InvalidData("Segment not found".to_string()));
        }

        let segment_start = reader.stream_position()?;

        let mut demuxer = Self {
            reader,
            streams: Vec::new(),
            container_info: ContainerInfo {
                format_name,
                duration: None,
                bitrate: None,
                metadata: rust_media_core::stream::Metadata::new(),
            },
            timecode_scale: 1_000_000, // Default: 1ms
            cluster_timecode: 0,
            current_position: 0,
            segment_start,
            tracks_parsed: false,
        };

        // Parse segment info and tracks
        demuxer.parse_segment_header()?;

        Ok(demuxer)
    }

    fn parse_segment_header(&mut self) -> Result<()> {
        loop {
            let elem = Element::read(&mut self.reader)?;

            match elem.id {
                element_id::INFO => {
                    self.parse_info(&elem)?;
                }
                element_id::TRACKS => {
                    self.parse_tracks(&elem)?;
                    self.tracks_parsed = true;
                }
                element_id::CLUSTER => {
                    // Found first cluster, seek back to it
                    self.reader
                        .seek(SeekFrom::Start(elem.position))?;
                    break;
                }
                element_id::SEEK_HEAD | element_id::CUES => {
                    // Skip these for now
                    elem.skip(&mut self.reader)?;
                }
                _ => {
                    // Skip unknown elements
                    elem.skip(&mut self.reader)?;
                }
            }

            if self.tracks_parsed {
                break;
            }
        }

        Ok(())
    }

    fn parse_info(&mut self, parent: &Element) -> Result<()> {
        let end_pos = self.reader.stream_position()? + parent.size.unwrap_or(0);

        while self.reader.stream_position()? < end_pos {
            let elem = Element::read(&mut self.reader)?;

            match elem.id {
                element_id::TIMECODE_SCALE => {
                    self.timecode_scale = elem.read_uint(&mut self.reader)?;
                }
                element_id::DURATION => {
                    let duration_float = elem.read_float(&mut self.reader)?;
                    // Convert from timecode scale to microseconds
                    let duration_us = (duration_float * self.timecode_scale as f64 / 1000.0) as i64;
                    self.container_info.duration = Some(duration_us);
                }
                _ => {
                    elem.skip(&mut self.reader)?;
                }
            }
        }

        Ok(())
    }

    fn parse_tracks(&mut self, parent: &Element) -> Result<()> {
        let end_pos = self.reader.stream_position()? + parent.size.unwrap_or(0);
        let mut track_index = 0;

        while self.reader.stream_position()? < end_pos {
            let elem = Element::read(&mut self.reader)?;

            if elem.id == element_id::TRACK_ENTRY {
                if let Some(track_info) = self.parse_track_entry(&elem)? {
                    let stream_info = self.create_stream_info(track_index, &track_info)?;
                    self.streams.push(stream_info);
                    track_index += 1;
                }
            } else {
                elem.skip(&mut self.reader)?;
            }
        }

        Ok(())
    }

    fn parse_track_entry(&mut self, parent: &Element) -> Result<Option<TrackInfo>> {
        let end_pos = self.reader.stream_position()? + parent.size.unwrap_or(0);

        let mut track_number = None;
        let mut track_type = None;
        let mut codec_id = None;
        let mut codec_private = None;
        let mut audio_params = None;
        let mut video_params = None;

        while self.reader.stream_position()? < end_pos {
            let elem = Element::read(&mut self.reader)?;

            match elem.id {
                element_id::TRACK_NUMBER => {
                    track_number = Some(elem.read_uint(&mut self.reader)?);
                }
                element_id::TRACK_TYPE => {
                    track_type = Some(elem.read_uint(&mut self.reader)? as u8);
                }
                element_id::CODEC_ID => {
                    codec_id = Some(elem.read_string(&mut self.reader)?);
                }
                element_id::CODEC_PRIVATE => {
                    codec_private = Some(elem.read_data(&mut self.reader)?);
                }
                element_id::AUDIO => {
                    audio_params = Some(self.parse_audio(&elem)?);
                }
                element_id::VIDEO => {
                    video_params = Some(self.parse_video(&elem)?);
                }
                _ => {
                    elem.skip(&mut self.reader)?;
                }
            }
        }

        if let (Some(track_number), Some(track_type), Some(codec_id)) =
            (track_number, track_type, codec_id)
        {
            Ok(Some(TrackInfo {
                track_number,
                track_type,
                codec_id,
                codec_private,
                audio_params,
                video_params,
            }))
        } else {
            Ok(None)
        }
    }

    fn parse_audio(&mut self, parent: &Element) -> Result<AudioParams> {
        let end_pos = self.reader.stream_position()? + parent.size.unwrap_or(0);

        let mut sampling_frequency = 8000.0;
        let mut channels = 1;
        let mut bit_depth = None;

        while self.reader.stream_position()? < end_pos {
            let elem = Element::read(&mut self.reader)?;

            match elem.id {
                element_id::SAMPLING_FREQUENCY => {
                    sampling_frequency = elem.read_float(&mut self.reader)?;
                }
                element_id::CHANNELS => {
                    channels = elem.read_uint(&mut self.reader)?;
                }
                element_id::BIT_DEPTH => {
                    bit_depth = Some(elem.read_uint(&mut self.reader)?);
                }
                _ => {
                    elem.skip(&mut self.reader)?;
                }
            }
        }

        Ok(AudioParams {
            sampling_frequency,
            channels,
            bit_depth,
        })
    }

    fn parse_video(&mut self, parent: &Element) -> Result<VideoParams> {
        let end_pos = self.reader.stream_position()? + parent.size.unwrap_or(0);

        let mut pixel_width = 0;
        let mut pixel_height = 0;
        let mut display_width = None;
        let mut display_height = None;
        let mut frame_rate = None;

        while self.reader.stream_position()? < end_pos {
            let elem = Element::read(&mut self.reader)?;

            match elem.id {
                element_id::PIXEL_WIDTH => {
                    pixel_width = elem.read_uint(&mut self.reader)?;
                }
                element_id::PIXEL_HEIGHT => {
                    pixel_height = elem.read_uint(&mut self.reader)?;
                }
                element_id::DISPLAY_WIDTH => {
                    display_width = Some(elem.read_uint(&mut self.reader)?);
                }
                element_id::DISPLAY_HEIGHT => {
                    display_height = Some(elem.read_uint(&mut self.reader)?);
                }
                element_id::FRAME_RATE => {
                    frame_rate = Some(elem.read_float(&mut self.reader)?);
                }
                _ => {
                    elem.skip(&mut self.reader)?;
                }
            }
        }

        if pixel_width == 0 || pixel_height == 0 {
            return Err(Error::InvalidData(
                "Video track missing width or height".to_string(),
            ));
        }

        Ok(VideoParams {
            pixel_width,
            pixel_height,
            display_width,
            display_height,
            frame_rate,
        })
    }

    fn create_stream_info(&self, index: usize, track: &TrackInfo) -> Result<StreamInfo> {
        let media_type = match track.track_type {
            1 => MediaType::Video,
            2 => MediaType::Audio,
            _ => MediaType::Unknown,
        };

        // Map codec ID to our format
        // WebM codec IDs (always supported)
        // MKV/Matroska codec IDs (additional codecs supported by MKV)
        let codec = match track.codec_id.as_str() {
            // Audio
            "A_OPUS" => "opus",
            "A_VORBIS" => "vorbis",
            "A_AAC" => "aac",
            "A_MPEG/L3" => "mp3",
            "A_FLAC" => "flac",
            "A_AC3" => "ac3",
            "A_PCM/INT/LIT" | "A_PCM/INT/BIG" => "pcm",
            // Video
            "V_VP8" => "vp8",
            "V_VP9" => "vp9",
            "V_AV1" => "av1",
            "V_MPEG4/ISO/AVC" => "h264",
            "V_MPEGH/ISO/HEVC" => "hevc",
            _ => "unknown",
        };

        let mut stream_info = StreamInfo::new(index, media_type, codec.to_string())
            .with_time_base(1, 1_000_000); // Microseconds

        if let Some(duration) = self.container_info.duration {
            stream_info = stream_info.with_duration(duration);
        }

        // CodecPrivate from MKV is the codec-specific extra data
        // For H.264 this is the AVCDecoderConfigurationRecord (avcC)
        // For H.265 this is the HEVCDecoderConfigurationRecord (hvcC)
        // For AAC this is the AudioSpecificConfig
        // For Opus this is the OpusHead (already handled separately by Opus decoder)
        if let Some(ref cp) = track.codec_private {
            stream_info.extra_data = cp.clone();
        }

        // Add audio parameters
        if let Some(ref audio) = track.audio_params {
            let sample_format = match audio.bit_depth {
                Some(8) => SampleFormat::U8,
                Some(16) => SampleFormat::S16,
                Some(32) => SampleFormat::S32,
                _ => SampleFormat::S16, // Default for Opus
            };

            let audio_params = AudioStreamParams::new(
                audio.sampling_frequency as u32,
                audio.channels as usize,
                sample_format,
            );

            stream_info = stream_info.with_params(StreamParams::Audio(audio_params));
        }

        // Add video parameters
        if let Some(ref video) = track.video_params {
            // VP8/VP9 use YUV420P pixel format
            let pixel_format = match codec {
                "vp8" | "vp9" | "av1" => PixelFormat::YUV420P,
                _ => PixelFormat::YUV420P, // Default
            };

            let video_params = VideoStreamParams::new(
                video.pixel_width as usize,
                video.pixel_height as usize,
                pixel_format,
            );

            stream_info = stream_info.with_params(StreamParams::Video(video_params));
        }

        Ok(stream_info)
    }
}

impl<R: Read + Seek> Demuxer for WebmDemuxer<R> {
    fn container_info(&self) -> Result<ContainerInfo> {
        Ok(self.container_info.clone())
    }

    fn streams(&self) -> Result<Vec<StreamInfo>> {
        Ok(self.streams.clone())
    }

    fn stream_info(&self, stream_index: usize) -> Result<StreamInfo> {
        self.streams
            .get(stream_index)
            .cloned()
            .ok_or_else(|| Error::InvalidData(format!("Invalid stream index: {}", stream_index)))
    }

    fn read_packet(&mut self) -> Result<Packet> {
        loop {
            let elem = match Element::read(&mut self.reader) {
                Ok(elem) => elem,
                Err(Error::Io(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Err(Error::EndOfStream);
                }
                Err(e) => return Err(e),
            };

            match elem.id {
                element_id::CLUSTER => {
                    // Parse cluster header for timecode
                    self.parse_cluster_header(&elem)?;
                }
                element_id::SIMPLE_BLOCK => {
                    return self.parse_simple_block(&elem);
                }
                element_id::BLOCK_GROUP => {
                    return self.parse_block_group(&elem);
                }
                element_id::TIMECODE => {
                    self.cluster_timecode = elem.read_uint(&mut self.reader)?;
                }
                _ => {
                    elem.skip(&mut self.reader)?;
                }
            }
        }
    }

    fn seek(&mut self, _timestamp_us: i64) -> Result<()> {
        Err(Error::NotImplemented("WebM seeking not yet implemented".to_string()))
    }

    fn seek_stream(&mut self, stream_index: usize, timestamp: i64) -> Result<()> {
        if stream_index >= self.streams.len() {
            return Err(Error::InvalidData(format!(
                "Invalid stream index: {}",
                stream_index
            )));
        }
        self.seek(timestamp)
    }

    fn position(&self) -> u64 {
        self.current_position
    }

    fn size(&self) -> Option<u64> {
        None // Unknown for streaming
    }
}

impl<R: Read + Seek> WebmDemuxer<R> {
    fn parse_cluster_header(&mut self, parent: &Element) -> Result<()> {
        let start_pos = self.reader.stream_position()?;
        let end_pos = start_pos + parent.size.unwrap_or(0);

        // Look for timecode element (first element in cluster)
        if self.reader.stream_position()? < end_pos {
            let elem = Element::read(&mut self.reader)?;

            if elem.id == element_id::TIMECODE {
                self.cluster_timecode = elem.read_uint(&mut self.reader)?;
            }
            // Seek back to start of cluster data regardless
            self.reader.seek(SeekFrom::Start(start_pos))?;
            return Ok(());
        }

        self.reader.seek(SeekFrom::Start(start_pos))?;
        Ok(())
    }

    fn parse_simple_block(&mut self, elem: &Element) -> Result<Packet> {
        let data = elem.read_data(&mut self.reader)?;

        if data.is_empty() {
            return Err(Error::InvalidData("Empty simple block".to_string()));
        }

        // Parse block header
        let (track_number, header_size) = self.parse_track_number(&data)?;

        if header_size + 3 > data.len() {
            return Err(Error::InvalidData("Invalid block header".to_string()));
        }

        // Read timecode (2 bytes, signed big-endian)
        let timecode = i16::from_be_bytes([data[header_size], data[header_size + 1]]);
        let _flags = data[header_size + 2];

        // Calculate absolute timestamp in microseconds
        let absolute_timecode = self.cluster_timecode as i64 + timecode as i64;
        let pts = (absolute_timecode * self.timecode_scale as i64) / 1000;

        // Block data starts after header
        let block_data = data[header_size + 3..].to_vec();

        // Find which stream this belongs to
        let stream_index = self.find_stream_index(track_number)?;

        // Get the correct media type from the stream
        let media_type = self.streams.get(stream_index)
            .map(|s| s.media_type)
            .unwrap_or(MediaType::Unknown);

        let packet = Packet::new(block_data, stream_index, media_type)
            .with_pts(pts);

        Ok(packet)
    }

    fn parse_block_group(&mut self, parent: &Element) -> Result<Packet> {
        let end_pos = self.reader.stream_position()? + parent.size.unwrap_or(0);

        let mut block_data = None;
        let mut duration = None;

        while self.reader.stream_position()? < end_pos {
            let elem = Element::read(&mut self.reader)?;

            match elem.id {
                element_id::BLOCK => {
                    block_data = Some(elem.read_data(&mut self.reader)?);
                }
                element_id::BLOCK_DURATION => {
                    duration = Some(elem.read_uint(&mut self.reader)?);
                }
                _ => {
                    elem.skip(&mut self.reader)?;
                }
            }
        }

        let data = block_data.ok_or_else(|| Error::InvalidData("No block in block group".to_string()))?;

        if data.is_empty() {
            return Err(Error::InvalidData("Empty block".to_string()));
        }

        // Parse block header
        let (track_number, header_size) = self.parse_track_number(&data)?;

        if header_size + 3 > data.len() {
            return Err(Error::InvalidData("Invalid block header".to_string()));
        }

        // Read timecode (2 bytes, signed big-endian)
        let timecode = i16::from_be_bytes([data[header_size], data[header_size + 1]]);

        // Calculate absolute timestamp
        let absolute_timecode = self.cluster_timecode as i64 + timecode as i64;
        let pts = (absolute_timecode * self.timecode_scale as i64) / 1000;

        // Block data starts after header
        let block_data = data[header_size + 3..].to_vec();

        // Find stream index
        let stream_index = self.find_stream_index(track_number)?;

        // Get the correct media type from the stream
        let media_type = self.streams.get(stream_index)
            .map(|s| s.media_type)
            .unwrap_or(MediaType::Unknown);

        let mut packet = Packet::new(block_data, stream_index, media_type)
            .with_pts(pts);

        if let Some(dur) = duration {
            let duration_us = (dur * self.timecode_scale) / 1000;
            packet = packet.with_duration(duration_us as i64);
        }

        Ok(packet)
    }

    fn parse_track_number(&self, data: &[u8]) -> Result<(u64, usize)> {
        if data.is_empty() {
            return Err(Error::InvalidData("Empty block data".to_string()));
        }

        // Track number is a VINT
        let first_byte = data[0];
        let mut mask = 0x80u8;
        let mut length = 0usize;

        for i in 0..8 {
            if (first_byte & mask) != 0 {
                length = i + 1;
                break;
            }
            mask >>= 1;
        }

        if length == 0 || length > data.len() {
            return Err(Error::InvalidData("Invalid track number".to_string()));
        }

        let mut value = (first_byte & (mask - 1)) as u64;
        for &byte in &data[1..length] {
            value = (value << 8) | (byte as u64);
        }

        Ok((value, length))
    }

    fn find_stream_index(&self, track_number: u64) -> Result<usize> {
        // For now, assume track numbers match stream indices
        // In a more complete implementation, we'd maintain a mapping
        for (idx, _stream) in self.streams.iter().enumerate() {
            // Match based on track number (simplified)
            if idx as u64 + 1 == track_number {
                return Ok(idx);
            }
        }
        Ok(0) // Default to first stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webm_demuxer_needs_valid_file() {
        // Test that invalid data is rejected
        let invalid_data = vec![0u8; 100];
        let cursor = std::io::Cursor::new(invalid_data);
        let result = WebmDemuxer::open(cursor);
        assert!(result.is_err());
    }
}
