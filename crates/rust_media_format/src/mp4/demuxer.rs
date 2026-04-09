//! MP4 demuxer implementation
//!
//! Reads video and audio data from MP4 container format (ISO Base Media File Format).
//!
//! # Supported Codecs
//!
//! - **Video**: H.264/AVC (`avc1`), VP9 (`vp09`)
//! - **Audio**: AAC (`mp4a`), Opus (`Opus`)
//!
//! # Example
//!
//! ```ignore
//! use rust_media_format::mp4::Mp4Demuxer;
//! use rust_media_core::Demuxer;
//! use std::fs::File;
//!
//! let file = File::open("video.mp4")?;
//! let mut demuxer = Mp4Demuxer::new(file)?;
//!
//! // Get stream information
//! let streams = demuxer.streams()?;
//!
//! // Read packets
//! while let Ok(packet) = demuxer.read_packet() {
//!     println!("Stream {}: {} bytes", packet.stream_index(), packet.data().len());
//! }
//! ```

use crate::mp4::boxes::*;
use crate::mp4::reader::*;
use rust_media_core::demuxer::Demuxer;
use rust_media_core::error::{Error, Result};
use rust_media_core::packet::Packet;
use rust_media_core::stream::{
    AudioStreamParams, ContainerInfo, StreamInfo, StreamParams, VideoStreamParams,
};
use rust_media_core::types::{MediaType, PixelFormat, SampleFormat};
use std::io::{Read, Seek, SeekFrom};

/// Track information parsed from the MP4 file
#[derive(Debug, Clone)]
struct TrackInfo {
    /// Track ID
    #[allow(dead_code)]
    track_id: u32,
    /// Stream index (0-based)
    stream_index: usize,
    /// Media type
    media_type: MediaType,
    /// Timescale (ticks per second)
    timescale: u32,
    /// Duration in timescale units
    duration: u64,
    /// Sample description
    sample_entry: SampleEntry,
    /// Time-to-sample entries
    stts: Vec<SttsEntry>,
    /// Sample-to-chunk entries
    stsc: Vec<StscEntry>,
    /// Sample sizes
    sample_sizes: Vec<u32>,
    /// Chunk offsets
    chunk_offsets: Vec<u64>,
    /// Sync samples (keyframes) - 1-based indices
    sync_samples: Option<Vec<u32>>,
    /// Composition time offsets
    ctts: Option<Vec<CttsEntry>>,
}

/// Sample information for reading
#[derive(Debug, Clone)]
struct SampleInfo {
    /// Stream index
    stream_index: usize,
    /// Sample index within the track (0-based)
    #[allow(dead_code)]
    sample_index: u32,
    /// File offset
    offset: u64,
    /// Sample size in bytes
    size: u32,
    /// Decode timestamp
    dts: i64,
    /// Presentation timestamp
    pts: i64,
    /// Duration
    duration: u32,
    /// Is keyframe
    is_keyframe: bool,
}

/// MP4 Demuxer
///
/// Parses MP4 files and extracts media packets.
pub struct Mp4Demuxer<R> {
    reader: R,
    /// File size
    file_size: u64,
    /// Parsed track information
    tracks: Vec<TrackInfo>,
    /// Stream info for each track
    stream_infos: Vec<StreamInfo>,
    /// All samples sorted by file offset for sequential reading
    samples: Vec<SampleInfo>,
    /// Current sample index
    current_sample: usize,
    /// mdat box offset and size
    mdat_offset: u64,
    mdat_size: u64,
}

impl<R: Read + Seek> Mp4Demuxer<R> {
    /// Creates a new MP4 demuxer from a reader
    pub fn new(mut reader: R) -> Result<Self> {
        // Get file size
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let mut demuxer = Self {
            reader,
            file_size,
            tracks: Vec::new(),
            stream_infos: Vec::new(),
            samples: Vec::new(),
            current_sample: 0,
            mdat_offset: 0,
            mdat_size: 0,
        };

        demuxer.parse_file()?;
        demuxer.build_sample_table()?;

        Ok(demuxer)
    }

    /// Parses the MP4 file structure
    fn parse_file(&mut self) -> Result<()> {
        let mut found_moov = false;

        while self.reader.stream_position()? < self.file_size {
            let header = read_box_header(&mut self.reader)?;

            match header.box_type {
                FTYP => {
                    // Skip ftyp - we just verify it exists
                    skip_box(&mut self.reader, &header)?;
                }
                MOOV => {
                    self.parse_moov(&header)?;
                    found_moov = true;
                }
                MDAT => {
                    self.mdat_offset = header.content_offset();
                    self.mdat_size = header.content_size();
                    skip_box(&mut self.reader, &header)?;
                }
                _ => {
                    // Skip unknown boxes
                    skip_box(&mut self.reader, &header)?;
                }
            }
        }

        if !found_moov {
            return Err(Error::InvalidData("No moov box found in MP4 file".to_string()));
        }

        Ok(())
    }

    /// Parses the moov (movie) box
    fn parse_moov(&mut self, moov_header: &BoxHeader) -> Result<()> {
        let moov_end = moov_header.offset + moov_header.size;
        let mut track_index = 0;

        while self.reader.stream_position()? < moov_end {
            let header = read_box_header(&mut self.reader)?;

            match header.box_type {
                TRAK => {
                    if let Some(track) = self.parse_trak(&header, track_index)? {
                        self.tracks.push(track);
                        track_index += 1;
                    }
                }
                _ => {
                    skip_box(&mut self.reader, &header)?;
                }
            }
        }

        // Build stream infos from tracks
        self.build_stream_infos()?;

        Ok(())
    }

    /// Parses a trak (track) box
    fn parse_trak(&mut self, trak_header: &BoxHeader, stream_index: usize) -> Result<Option<TrackInfo>> {
        let trak_end = trak_header.offset + trak_header.size;

        let mut track_id = 0u32;
        let mut media_type = MediaType::Unknown;
        let mut timescale = 0u32;
        let mut duration = 0u64;
        let mut sample_entry = SampleEntry::Unknown;
        let mut stts = Vec::new();
        let mut stsc = Vec::new();
        let mut sample_sizes = Vec::new();
        let mut chunk_offsets = Vec::new();
        let mut sync_samples = None;
        let mut ctts = None;

        while self.reader.stream_position()? < trak_end {
            let header = read_box_header(&mut self.reader)?;

            match header.box_type {
                TKHD => {
                    // Parse track header for track ID
                    let version = self.reader.read_u8()?;
                    use byteorder::{BigEndian, ReadBytesExt};
                    let mut flags = [0u8; 3];
                    self.reader.read_exact(&mut flags)?;

                    if version == 1 {
                        self.reader.seek(SeekFrom::Current(16))?; // creation/modification time
                        track_id = self.reader.read_u32::<BigEndian>()?;
                    } else {
                        self.reader.seek(SeekFrom::Current(8))?;
                        track_id = self.reader.read_u32::<BigEndian>()?;
                    }

                    skip_box(&mut self.reader, &header)?;
                }
                MDIA => {
                    // Parse media box
                    let mdia_end = header.offset + header.size;

                    while self.reader.stream_position()? < mdia_end {
                        let mdia_child = read_box_header(&mut self.reader)?;

                        match mdia_child.box_type {
                            MDHD => {
                                let (ts, dur) = parse_mdhd(&mut self.reader)?;
                                timescale = ts;
                                duration = dur;
                                skip_box(&mut self.reader, &mdia_child)?;
                            }
                            HDLR => {
                                let handler = parse_hdlr(&mut self.reader, mdia_child.content_size())?;
                                media_type = match handler {
                                    HANDLER_VIDEO => MediaType::Video,
                                    HANDLER_SOUND => MediaType::Audio,
                                    _ => MediaType::Unknown,
                                };
                            }
                            MINF => {
                                // Parse media info
                                let minf_end = mdia_child.offset + mdia_child.size;

                                while self.reader.stream_position()? < minf_end {
                                    let minf_child = read_box_header(&mut self.reader)?;

                                    if minf_child.box_type == STBL {
                                        // Parse sample table
                                        let stbl_end = minf_child.offset + minf_child.size;

                                        while self.reader.stream_position()? < stbl_end {
                                            let stbl_child = read_box_header(&mut self.reader)?;

                                            match stbl_child.box_type {
                                                STSD => {
                                                    let entries = parse_stsd(&mut self.reader, stbl_child.content_size())?;
                                                    if !entries.is_empty() {
                                                        sample_entry = entries[0].clone();
                                                    }
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                STTS => {
                                                    stts = parse_stts(&mut self.reader)?;
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                STSC => {
                                                    stsc = parse_stsc(&mut self.reader)?;
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                STSZ => {
                                                    let (_, sizes) = parse_stsz(&mut self.reader)?;
                                                    sample_sizes = sizes;
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                STCO => {
                                                    chunk_offsets = parse_stco(&mut self.reader)?;
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                CO64 => {
                                                    chunk_offsets = parse_co64(&mut self.reader)?;
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                STSS => {
                                                    sync_samples = Some(parse_stss(&mut self.reader)?);
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                CTTS => {
                                                    ctts = Some(parse_ctts(&mut self.reader)?);
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                                _ => {
                                                    skip_box(&mut self.reader, &stbl_child)?;
                                                }
                                            }
                                        }
                                    } else {
                                        skip_box(&mut self.reader, &minf_child)?;
                                    }
                                }
                            }
                            _ => {
                                skip_box(&mut self.reader, &mdia_child)?;
                            }
                        }
                    }
                }
                _ => {
                    skip_box(&mut self.reader, &header)?;
                }
            }
        }

        // Skip tracks we can't handle
        if matches!(sample_entry, SampleEntry::Unknown) || sample_sizes.is_empty() {
            return Ok(None);
        }

        Ok(Some(TrackInfo {
            track_id,
            stream_index,
            media_type,
            timescale,
            duration,
            sample_entry,
            stts,
            stsc,
            sample_sizes,
            chunk_offsets,
            sync_samples,
            ctts,
        }))
    }

    /// Builds StreamInfo for each track
    fn build_stream_infos(&mut self) -> Result<()> {
        for track in &self.tracks {
            let (codec, params, extra_data) = match &track.sample_entry {
                SampleEntry::Audio(audio) => {
                    let params = AudioStreamParams::new(
                        audio.sample_rate,
                        audio.channels as usize,
                        SampleFormat::S16, // Decoded format
                    );
                    (
                        audio.codec.clone(),
                        StreamParams::Audio(params),
                        audio.extra_data.clone(),
                    )
                }
                SampleEntry::Video(video) => {
                    // Try to parse the bit depth from the codec configuration
                    // record (av1C / hvcC). Defaults to 8 if unknown or absent.
                    let bit_depth = match video.codec.as_str() {
                        "av1" => crate::mp4::reader::parse_av1c_bit_depth(&video.extra_data)
                            .unwrap_or(8),
                        "hevc" => crate::mp4::reader::parse_hvcc_bit_depth(&video.extra_data)
                            .unwrap_or(8),
                        _ => 8,
                    };
                    let pixel_format = match bit_depth {
                        8 => PixelFormat::YUV420P,
                        10 => PixelFormat::YUV420P10LE,
                        12 => {
                            return Err(rust_media_core::Error::Unsupported(format!(
                                "12-bit video is not yet supported (codec: {}). \
                                The codec layer and pixel format support are 8-bit/10-bit only.",
                                video.codec
                            )));
                        }
                        other => {
                            return Err(rust_media_core::Error::Unsupported(format!(
                                "unsupported bit depth: {} (codec: {})",
                                other, video.codec
                            )));
                        }
                    };
                    let params = VideoStreamParams {
                        width: video.width as usize,
                        height: video.height as usize,
                        pixel_format,
                        frame_rate: (30, 1), // Default, could be calculated from stts
                        color_space: rust_media_core::types::ColorSpace::BT709,
                        color_range: rust_media_core::types::ColorRange::Limited,
                        sample_aspect_ratio: (1, 1),
                        bit_depth,
                    };
                    (
                        video.codec.clone(),
                        StreamParams::Video(params),
                        video.extra_data.clone(),
                    )
                }
                SampleEntry::Unknown => continue,
            };

            let duration_us = if track.timescale > 0 {
                Some((track.duration as i64 * 1_000_000) / track.timescale as i64)
            } else {
                None
            };

            let stream_info = StreamInfo {
                index: track.stream_index,
                media_type: track.media_type,
                codec,
                time_base: (1, track.timescale),
                duration: duration_us,
                bitrate: None,
                params,
                extra_data,
            };

            self.stream_infos.push(stream_info);
        }

        Ok(())
    }

    /// Builds the sample table for sequential reading
    fn build_sample_table(&mut self) -> Result<()> {
        for track in &self.tracks {
            let samples = self.build_track_samples(track)?;
            self.samples.extend(samples);
        }

        // Sort samples by file offset for efficient sequential reading
        self.samples.sort_by_key(|s| s.offset);

        Ok(())
    }

    /// Builds sample information for a single track
    fn build_track_samples(&self, track: &TrackInfo) -> Result<Vec<SampleInfo>> {
        let mut samples = Vec::with_capacity(track.sample_sizes.len());

        // Build DTS from stts
        let mut dts_table = Vec::with_capacity(track.sample_sizes.len());
        let mut dts = 0i64;
        for entry in &track.stts {
            for _ in 0..entry.sample_count {
                dts_table.push(dts);
                dts += entry.sample_delta as i64;
            }
        }

        // Build CTS offsets from ctts (if present)
        let mut cts_offsets = Vec::with_capacity(track.sample_sizes.len());
        if let Some(ctts) = &track.ctts {
            for entry in ctts {
                for _ in 0..entry.sample_count {
                    cts_offsets.push(entry.sample_offset as i64);
                }
            }
        }

        // Build sample-to-chunk mapping
        let mut chunk_sample_counts = Vec::new();
        for (i, entry) in track.stsc.iter().enumerate() {
            let next_first_chunk = if i + 1 < track.stsc.len() {
                track.stsc[i + 1].first_chunk
            } else {
                track.chunk_offsets.len() as u32 + 1
            };

            let chunk_count = next_first_chunk - entry.first_chunk;
            for _ in 0..chunk_count {
                chunk_sample_counts.push(entry.samples_per_chunk);
            }
        }

        // Build sample offsets from chunk offsets and sample sizes
        let mut sample_offsets = Vec::with_capacity(track.sample_sizes.len());
        let mut sample_idx = 0usize;

        for (chunk_idx, &chunk_offset) in track.chunk_offsets.iter().enumerate() {
            let samples_in_chunk = if chunk_idx < chunk_sample_counts.len() {
                chunk_sample_counts[chunk_idx] as usize
            } else {
                1
            };

            let mut offset = chunk_offset;
            for _ in 0..samples_in_chunk {
                if sample_idx >= track.sample_sizes.len() {
                    break;
                }
                sample_offsets.push(offset);
                offset += track.sample_sizes[sample_idx] as u64;
                sample_idx += 1;
            }
        }

        // Build keyframe set
        let keyframes: std::collections::HashSet<u32> = track
            .sync_samples
            .as_ref()
            .map(|ss| ss.iter().copied().collect())
            .unwrap_or_default();

        // Create sample info entries
        for (i, &size) in track.sample_sizes.iter().enumerate() {
            if i >= sample_offsets.len() || i >= dts_table.len() {
                break;
            }

            let sample_dts = dts_table[i];
            let cts_offset = if i < cts_offsets.len() {
                cts_offsets[i]
            } else {
                0
            };
            let sample_pts = sample_dts + cts_offset;

            let duration = if i + 1 < dts_table.len() {
                (dts_table[i + 1] - dts_table[i]) as u32
            } else if !track.stts.is_empty() {
                track.stts.last().unwrap().sample_delta
            } else {
                1
            };

            // Check if keyframe (sample indices in stss are 1-based)
            let is_keyframe = if track.sync_samples.is_some() {
                keyframes.contains(&((i + 1) as u32))
            } else {
                // If no sync sample table, assume all samples are keyframes (audio)
                track.media_type == MediaType::Audio
            };

            samples.push(SampleInfo {
                stream_index: track.stream_index,
                sample_index: i as u32,
                offset: sample_offsets[i],
                size,
                dts: sample_dts,
                pts: sample_pts,
                duration,
                is_keyframe,
            });
        }

        Ok(samples)
    }

    /// Reads a sample from the file
    fn read_sample(&mut self, sample: &SampleInfo) -> Result<Packet> {
        self.reader.seek(SeekFrom::Start(sample.offset))?;

        let mut data = vec![0u8; sample.size as usize];
        self.reader.read_exact(&mut data)?;

        let stream_info = &self.stream_infos[sample.stream_index];
        let media_type = stream_info.media_type;

        let mut packet = Packet::new(data, sample.stream_index, media_type)
            .with_pts(sample.pts)
            .with_dts(sample.dts)
            .with_duration(sample.duration as i64);

        if sample.is_keyframe {
            packet.set_keyframe(true);
        }

        Ok(packet)
    }
}

impl<R: Read + Seek> Demuxer for Mp4Demuxer<R> {
    fn container_info(&self) -> Result<ContainerInfo> {
        // Calculate total duration from tracks
        let duration_us = self
            .tracks
            .iter()
            .filter_map(|t| {
                if t.timescale > 0 {
                    Some((t.duration as i64 * 1_000_000) / t.timescale as i64)
                } else {
                    None
                }
            })
            .max();

        Ok(ContainerInfo {
            format_name: "mp4".to_string(),
            duration: duration_us,
            bitrate: None,
            metadata: rust_media_core::stream::Metadata::default(),
        })
    }

    fn streams(&self) -> Result<Vec<StreamInfo>> {
        Ok(self.stream_infos.clone())
    }

    fn stream_info(&self, stream_index: usize) -> Result<StreamInfo> {
        self.stream_infos
            .get(stream_index)
            .cloned()
            .ok_or_else(|| Error::InvalidData(format!("Stream {} not found", stream_index)))
    }

    fn read_packet(&mut self) -> Result<Packet> {
        if self.current_sample >= self.samples.len() {
            return Err(Error::EndOfStream);
        }

        let sample = self.samples[self.current_sample].clone();
        self.current_sample += 1;

        self.read_sample(&sample)
    }

    fn seek(&mut self, timestamp_us: i64) -> Result<()> {
        // Find the sample closest to the timestamp
        // For simplicity, we seek to the first keyframe at or before the timestamp
        let mut best_sample = 0;

        for (i, sample) in self.samples.iter().enumerate() {
            let stream_info = &self.stream_infos[sample.stream_index];
            let sample_us = if stream_info.time_base.1 > 0 {
                sample.pts * 1_000_000 / stream_info.time_base.1 as i64
            } else {
                sample.pts
            };

            if sample_us <= timestamp_us && sample.is_keyframe {
                best_sample = i;
            } else if sample_us > timestamp_us {
                break;
            }
        }

        self.current_sample = best_sample;
        Ok(())
    }

    fn seek_stream(&mut self, stream_index: usize, timestamp: i64) -> Result<()> {
        // Find the sample in the specific stream
        let mut best_sample = 0;

        for (i, sample) in self.samples.iter().enumerate() {
            if sample.stream_index != stream_index {
                continue;
            }

            if sample.pts <= timestamp && sample.is_keyframe {
                best_sample = i;
            } else if sample.pts > timestamp {
                break;
            }
        }

        self.current_sample = best_sample;
        Ok(())
    }

    fn position(&self) -> u64 {
        if self.current_sample < self.samples.len() {
            self.samples[self.current_sample].offset
        } else {
            self.file_size
        }
    }

    fn size(&self) -> Option<u64> {
        Some(self.file_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_demuxer_requires_moov() {
        // Empty file should fail
        let data = vec![0u8; 100];
        let cursor = Cursor::new(data);
        let result = Mp4Demuxer::new(cursor);
        assert!(result.is_err());
    }
}
