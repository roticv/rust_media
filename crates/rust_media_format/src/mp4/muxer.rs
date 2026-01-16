//! MP4 muxer implementation
//!
//! Writes video and audio data to MP4 container format (ISO Base Media File Format).
//!
//! # File Structure
//!
//! The muxer writes files in the following order:
//! 1. `ftyp` - File type box (written by `write_header()`)
//! 2. `mdat` - Media data box (packets written by `write_packet()`)
//! 3. `moov` - Movie metadata box (written by `write_trailer()`)
//!
//! This ordering is streaming-friendly and allows writing without knowing
//! the total file size upfront.
//!
//! # Supported Codecs
//!
//! - **Video**: H.264/AVC (`h264`/`avc`), VP9 (`vp9`)
//! - **Audio**: AAC (`aac`), Opus (`opus`)
//!
//! # H.264 Support
//!
//! H.264 packets from encoders like x264 are typically in Annex B format
//! (with start codes like 0x00 0x00 0x00 0x01). This muxer automatically
//! converts them to AVCC format (4-byte NAL unit lengths) as required by MP4.
//!
//! The avcC box is constructed from SPS/PPS NAL units found in:
//! 1. The stream's `extra_data` field (if already in avcC format)
//! 2. Annex B formatted SPS/PPS in `extra_data`
//! 3. The first keyframe packet (SPS/PPS are often prepended)

use crate::mp4::boxes::*;
use crate::mp4::writer::*;
use rust_media_core::error::{Error, Result};
use rust_media_core::muxer::Muxer;
use rust_media_core::packet::Packet;
use rust_media_core::stream::{StreamInfo, StreamParams};
use rust_media_core::types::MediaType;
use std::io::{Seek, SeekFrom, Write};

// ============================================================================
// H.264 NAL Unit Utilities
// ============================================================================

/// H.264 NAL unit types
const NAL_TYPE_SPS: u8 = 7;
const NAL_TYPE_PPS: u8 = 8;

/// Finds the next Annex B start code (0x000001 or 0x00000001) in the data
fn find_start_code(data: &[u8], offset: usize) -> Option<(usize, usize)> {
    let mut i = offset;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            if data[i + 2] == 1 {
                return Some((i, 3)); // 3-byte start code
            } else if i + 3 < data.len() && data[i + 2] == 0 && data[i + 3] == 1 {
                return Some((i, 4)); // 4-byte start code
            }
        }
        i += 1;
    }
    None
}

/// Parses Annex B formatted data into individual NAL units (without start codes)
fn parse_annex_b_nal_units(data: &[u8]) -> Vec<Vec<u8>> {
    let mut nal_units = Vec::new();
    let mut offset = 0;

    // Find first start code
    let Some((first_start, first_len)) = find_start_code(data, offset) else {
        // No start codes found - might be raw NAL unit
        if !data.is_empty() {
            nal_units.push(data.to_vec());
        }
        return nal_units;
    };

    offset = first_start + first_len;

    loop {
        // Find next start code (or end of data)
        let nal_end = if let Some((next_start, _)) = find_start_code(data, offset) {
            next_start
        } else {
            data.len()
        };

        // Extract NAL unit (skip trailing zeros before next start code)
        let mut nal_data = data[offset..nal_end].to_vec();
        while nal_data.last() == Some(&0) {
            nal_data.pop();
        }

        if !nal_data.is_empty() {
            nal_units.push(nal_data);
        }

        if nal_end >= data.len() {
            break;
        }

        // Move past the start code
        if let Some((_, start_len)) = find_start_code(data, nal_end) {
            offset = nal_end + start_len;
        } else {
            break;
        }
    }

    nal_units
}

/// Extracts SPS and PPS NAL units from Annex B data
fn extract_sps_pps(data: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let nal_units = parse_annex_b_nal_units(data);
    let mut sps_list = Vec::new();
    let mut pps_list = Vec::new();

    for nal in nal_units {
        if nal.is_empty() {
            continue;
        }
        let nal_type = nal[0] & 0x1F;
        match nal_type {
            NAL_TYPE_SPS => sps_list.push(nal),
            NAL_TYPE_PPS => pps_list.push(nal),
            _ => {}
        }
    }

    (sps_list, pps_list)
}

/// Converts Annex B formatted H.264 data to AVCC format (4-byte length prefixes)
fn annex_b_to_avcc(data: &[u8]) -> Vec<u8> {
    let nal_units = parse_annex_b_nal_units(data);
    let mut avcc_data = Vec::new();

    for nal in nal_units {
        if nal.is_empty() {
            continue;
        }

        // Skip SPS/PPS in frame data - they belong in avcC box
        let nal_type = nal[0] & 0x1F;
        if nal_type == NAL_TYPE_SPS || nal_type == NAL_TYPE_PPS {
            continue;
        }

        // Write 4-byte length prefix (big-endian)
        let len = nal.len() as u32;
        avcc_data.extend_from_slice(&len.to_be_bytes());
        avcc_data.extend_from_slice(&nal);
    }

    avcc_data
}

/// Checks if data is already in avcC format (starts with version 1)
fn is_avcc_format(data: &[u8]) -> bool {
    !data.is_empty() && data[0] == 1
}

/// Builds an avcC box from SPS and PPS NAL units
fn build_avcc(sps_list: &[Vec<u8>], pps_list: &[Vec<u8>]) -> Result<Vec<u8>> {
    if sps_list.is_empty() {
        return Err(Error::InvalidData(
            "H.264 stream requires at least one SPS".to_string(),
        ));
    }

    let sps = &sps_list[0];
    if sps.len() < 4 {
        return Err(Error::InvalidData("SPS too short".to_string()));
    }

    let mut avcc = Vec::new();

    // AVCDecoderConfigurationRecord
    avcc.push(1); // configurationVersion
    avcc.push(sps[1]); // AVCProfileIndication
    avcc.push(sps[2]); // profile_compatibility
    avcc.push(sps[3]); // AVCLevelIndication
    avcc.push(0xFF); // lengthSizeMinusOne = 3 (4-byte NAL lengths) | reserved 6 bits

    // SPS array
    avcc.push(0xE0 | (sps_list.len() as u8)); // numOfSequenceParameterSets | reserved 3 bits
    for sps in sps_list {
        let len = sps.len() as u16;
        avcc.extend_from_slice(&len.to_be_bytes());
        avcc.extend_from_slice(sps);
    }

    // PPS array
    avcc.push(pps_list.len() as u8); // numOfPictureParameterSets
    for pps in pps_list {
        let len = pps.len() as u16;
        avcc.extend_from_slice(&len.to_be_bytes());
        avcc.extend_from_slice(pps);
    }

    Ok(avcc)
}

// ============================================================================
// MP4 Muxer Data Structures
// ============================================================================

/// Time-to-sample entry (stts box)
#[derive(Debug, Clone)]
struct SttsEntry {
    sample_count: u32,
    sample_delta: u32,
}

/// Sample-to-chunk entry (stsc box)
#[derive(Debug, Clone)]
struct StscEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
    sample_description_index: u32,
}

/// Composition time offset entry (ctts box)
#[derive(Debug, Clone)]
struct CttsEntry {
    sample_count: u32,
    sample_offset: i32,
}

/// Per-track data accumulated during muxing
#[derive(Debug)]
struct TrackData {
    /// Sample sizes (stsz)
    sample_sizes: Vec<u32>,
    /// Chunk offsets (stco/co64)
    chunk_offsets: Vec<u64>,
    /// Time-to-sample entries (stts) - run-length encoded
    time_to_sample: Vec<SttsEntry>,
    /// Sample-to-chunk entries (stsc)
    sample_to_chunk: Vec<StscEntry>,
    /// Sync sample indices (stss) - keyframes only
    sync_samples: Vec<u32>,
    /// Composition time offsets (ctts) - when pts != dts
    composition_offsets: Vec<CttsEntry>,
    /// Total sample count
    sample_count: u32,
    /// Total duration in track timescale
    duration: u64,
    /// Last DTS for delta calculation
    last_dts: Option<i64>,
    /// Track timescale (ticks per second)
    timescale: u32,
    /// Whether we need ctts box
    needs_ctts: bool,
    /// Whether this track uses H.264 (needs Annex B to AVCC conversion)
    is_h264: bool,
    /// Extracted SPS NAL units (for H.264)
    sps_list: Vec<Vec<u8>>,
    /// Extracted PPS NAL units (for H.264)
    pps_list: Vec<Vec<u8>>,
}

impl TrackData {
    fn new(timescale: u32, is_h264: bool) -> Self {
        Self {
            sample_sizes: Vec::new(),
            chunk_offsets: Vec::new(),
            time_to_sample: Vec::new(),
            sample_to_chunk: Vec::new(),
            sync_samples: Vec::new(),
            composition_offsets: Vec::new(),
            sample_count: 0,
            duration: 0,
            last_dts: None,
            timescale,
            needs_ctts: false,
            is_h264,
            sps_list: Vec::new(),
            pps_list: Vec::new(),
        }
    }

    /// Adds a sample and updates all tracking tables
    fn add_sample(
        &mut self,
        size: u32,
        offset: u64,
        dts: i64,
        pts: i64,
        is_keyframe: bool,
        is_video: bool,
    ) {
        self.sample_count += 1;
        self.sample_sizes.push(size);
        self.chunk_offsets.push(offset);

        // Update sync samples (keyframes) for video
        if is_video && is_keyframe {
            self.sync_samples.push(self.sample_count);
        }

        // Calculate sample delta (duration)
        let delta = if let Some(last) = self.last_dts {
            (dts - last).max(0) as u32
        } else {
            // First sample - use a default or calculate from pts
            0
        };

        // Run-length encode stts
        if let Some(last_entry) = self.time_to_sample.last_mut() {
            if last_entry.sample_delta == delta && self.sample_count > 1 {
                last_entry.sample_count += 1;
            } else if self.sample_count > 1 {
                self.time_to_sample.push(SttsEntry {
                    sample_count: 1,
                    sample_delta: delta,
                });
            }
        }
        if self.time_to_sample.is_empty() {
            self.time_to_sample.push(SttsEntry {
                sample_count: 1,
                sample_delta: delta,
            });
        }

        // Update duration
        self.duration = dts as u64 + delta as u64;
        self.last_dts = Some(dts);

        // Handle composition time offset (pts - dts)
        let cts_offset = (pts - dts) as i32;
        if cts_offset != 0 {
            self.needs_ctts = true;
        }

        // Run-length encode ctts
        if let Some(last_entry) = self.composition_offsets.last_mut() {
            if last_entry.sample_offset == cts_offset {
                last_entry.sample_count += 1;
            } else {
                self.composition_offsets.push(CttsEntry {
                    sample_count: 1,
                    sample_offset: cts_offset,
                });
            }
        } else {
            self.composition_offsets.push(CttsEntry {
                sample_count: 1,
                sample_offset: cts_offset,
            });
        }

        // Update sample-to-chunk (one sample per chunk for simplicity)
        // This could be optimized to group consecutive samples
        if self.sample_to_chunk.is_empty() {
            self.sample_to_chunk.push(StscEntry {
                first_chunk: 1,
                samples_per_chunk: 1,
                sample_description_index: 1,
            });
        }
    }

    /// Returns true if 64-bit chunk offsets are needed
    fn needs_64bit_offsets(&self) -> bool {
        self.chunk_offsets
            .last()
            .is_some_and(|&offset| offset > u32::MAX as u64)
    }
}

/// MP4 muxer
///
/// Writes video and audio streams to MP4 container format.
///
/// # Example
///
/// ```ignore
/// use rust_media_format::mp4::Mp4Muxer;
/// use std::fs::File;
///
/// let file = File::create("output.mp4")?;
/// let mut muxer = Mp4Muxer::new(file);
///
/// muxer.add_stream(video_stream)?;
/// muxer.add_stream(audio_stream)?;
/// muxer.write_header()?;
///
/// for packet in packets {
///     muxer.write_packet(&packet)?;
/// }
///
/// muxer.write_trailer()?;
/// ```
pub struct Mp4Muxer<W> {
    writer: W,
    streams: Vec<StreamInfo>,
    track_data: Vec<TrackData>,
    header_written: bool,
    trailer_written: bool,
    position: u64,
    mdat_start_position: u64,
    mdat_size: u64,
    movie_timescale: u32,
}

impl<W: Write + Seek> Mp4Muxer<W> {
    /// Creates a new MP4 muxer with the given writer
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            streams: Vec::new(),
            track_data: Vec::new(),
            header_written: false,
            trailer_written: false,
            position: 0,
            mdat_start_position: 0,
            mdat_size: 0,
            movie_timescale: 1000, // millisecond precision
        }
    }

    /// Validates that a codec is supported
    fn validate_codec(codec: &str, media_type: MediaType) -> Result<()> {
        match media_type {
            MediaType::Video => match codec {
                "h264" | "avc" | "avc1" => Ok(()),
                "vp9" | "vp09" => Ok(()),
                _ => Err(Error::Unsupported(format!(
                    "Unsupported video codec for MP4: {}",
                    codec
                ))),
            },
            MediaType::Audio => match codec {
                "aac" | "mp4a" => Ok(()),
                "opus" => Ok(()),
                _ => Err(Error::Unsupported(format!(
                    "Unsupported audio codec for MP4: {}",
                    codec
                ))),
            },
            _ => Err(Error::Unsupported(format!(
                "Unsupported media type for MP4: {:?}",
                media_type
            ))),
        }
    }

    /// Writes the ftyp box
    fn write_ftyp(&mut self) -> Result<()> {
        // ftyp box: size(4) + type(4) + major_brand(4) + minor_version(4) + compatible_brands(n*4)
        let brands = [BRAND_ISOM, BRAND_ISO2, BRAND_MP41];
        let size = 8 + 4 + 4 + (brands.len() * 4) as u32;

        write_box_header(&mut self.writer, FTYP, size)?;
        self.writer.write_all(&BRAND_ISOM)?; // major brand
        write_u32(&mut self.writer, 0x200)?; // minor version
        for brand in &brands {
            self.writer.write_all(brand)?;
        }

        self.position += size as u64;
        Ok(())
    }

    /// Writes the mdat header with placeholder size
    fn write_mdat_header(&mut self) -> Result<()> {
        self.mdat_start_position = self.position;

        // Write mdat header with size=0 (placeholder)
        // We'll use 64-bit size format to allow files > 4GB
        write_u32(&mut self.writer, 1)?; // size=1 indicates 64-bit size follows
        write_u32(&mut self.writer, MDAT)?;
        write_u64(&mut self.writer, 0)?; // 64-bit size placeholder

        self.position += 16;
        self.mdat_size = 16; // Header size
        Ok(())
    }

    /// Updates the mdat size field
    fn update_mdat_size(&mut self) -> Result<()> {
        let current = self.writer.stream_position()?;
        self.writer
            .seek(SeekFrom::Start(self.mdat_start_position + 8))?;
        write_u64(&mut self.writer, self.mdat_size)?;
        self.writer.seek(SeekFrom::Start(current))?;
        Ok(())
    }

    /// Writes the complete moov box
    fn write_moov(&mut self) -> Result<()> {
        let moov_start = write_box_header_placeholder(&mut self.writer, MOOV)?;

        // Write mvhd (movie header)
        self.write_mvhd()?;

        // Write trak boxes for each stream
        for i in 0..self.streams.len() {
            self.write_trak(i)?;
        }

        // Update moov size
        let moov_end = self.writer.stream_position()?;
        let moov_size = (moov_end - moov_start) as u32;
        update_box_size(&mut self.writer, moov_start, moov_size)?;

        self.position = moov_end;
        Ok(())
    }

    /// Writes the mvhd (movie header) box
    fn write_mvhd(&mut self) -> Result<()> {
        // Calculate overall movie duration
        let max_duration = self
            .track_data
            .iter()
            .map(|td| {
                // Convert track duration to movie timescale
                convert_timestamp(td.duration as i64, (1, td.timescale), self.movie_timescale)
                    as u64
            })
            .max()
            .unwrap_or(0);

        let mvhd_start = write_full_box_header(&mut self.writer, MVHD, 0, 0)?;

        // Version 0 mvhd fields
        write_u32(&mut self.writer, 0)?; // creation_time
        write_u32(&mut self.writer, 0)?; // modification_time
        write_u32(&mut self.writer, self.movie_timescale)?; // timescale
        write_u32(&mut self.writer, max_duration as u32)?; // duration
        write_fixed_16_16(&mut self.writer, 1.0)?; // rate (1.0 = normal)
        write_fixed_8_8(&mut self.writer, 1.0)?; // volume (1.0 = full)
        write_u16(&mut self.writer, 0)?; // reserved
        write_u32(&mut self.writer, 0)?; // reserved
        write_u32(&mut self.writer, 0)?; // reserved
        write_identity_matrix(&mut self.writer)?; // matrix
        write_zeros(&mut self.writer, 24)?; // pre_defined
        write_u32(&mut self.writer, (self.streams.len() + 1) as u32)?; // next_track_id

        let mvhd_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, mvhd_start, (mvhd_end - mvhd_start) as u32)?;
        Ok(())
    }

    /// Writes a trak box for a stream
    fn write_trak(&mut self, track_index: usize) -> Result<()> {
        let trak_start = write_box_header_placeholder(&mut self.writer, TRAK)?;

        self.write_tkhd(track_index)?;
        self.write_mdia(track_index)?;

        let trak_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, trak_start, (trak_end - trak_start) as u32)?;
        Ok(())
    }

    /// Writes the tkhd (track header) box
    fn write_tkhd(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];
        let track_data = &self.track_data[track_index];

        // Track duration in movie timescale
        let duration = convert_timestamp(
            track_data.duration as i64,
            (1, track_data.timescale),
            self.movie_timescale,
        ) as u32;

        // Get video dimensions if applicable
        let (width, height) = match &stream.params {
            StreamParams::Video(v) => (v.width as f64, v.height as f64),
            _ => (0.0, 0.0),
        };

        // flags: 0x01 = track enabled, 0x02 = in movie, 0x04 = in preview
        let tkhd_start = write_full_box_header(&mut self.writer, TKHD, 0, 0x07)?;

        write_u32(&mut self.writer, 0)?; // creation_time
        write_u32(&mut self.writer, 0)?; // modification_time
        write_u32(&mut self.writer, (track_index + 1) as u32)?; // track_id
        write_u32(&mut self.writer, 0)?; // reserved
        write_u32(&mut self.writer, duration)?; // duration
        write_u64(&mut self.writer, 0)?; // reserved
        write_i16(&mut self.writer, 0)?; // layer
        write_i16(&mut self.writer, 0)?; // alternate_group
        write_fixed_8_8(
            &mut self.writer,
            if stream.media_type == MediaType::Audio {
                1.0
            } else {
                0.0
            },
        )?; // volume
        write_u16(&mut self.writer, 0)?; // reserved
        write_identity_matrix(&mut self.writer)?; // matrix
        write_fixed_16_16(&mut self.writer, width)?; // width
        write_fixed_16_16(&mut self.writer, height)?; // height

        let tkhd_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, tkhd_start, (tkhd_end - tkhd_start) as u32)?;
        Ok(())
    }

    /// Writes the mdia (media) box
    fn write_mdia(&mut self, track_index: usize) -> Result<()> {
        let mdia_start = write_box_header_placeholder(&mut self.writer, MDIA)?;

        self.write_mdhd(track_index)?;
        self.write_hdlr(track_index)?;
        self.write_minf(track_index)?;

        let mdia_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, mdia_start, (mdia_end - mdia_start) as u32)?;
        Ok(())
    }

    /// Writes the mdhd (media header) box
    fn write_mdhd(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        let mdhd_start = write_full_box_header(&mut self.writer, MDHD, 0, 0)?;

        write_u32(&mut self.writer, 0)?; // creation_time
        write_u32(&mut self.writer, 0)?; // modification_time
        write_u32(&mut self.writer, track_data.timescale)?; // timescale
        write_u32(&mut self.writer, track_data.duration as u32)?; // duration
        write_u16(&mut self.writer, 0x55C4)?; // language (und = undetermined)
        write_u16(&mut self.writer, 0)?; // pre_defined

        let mdhd_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, mdhd_start, (mdhd_end - mdhd_start) as u32)?;
        Ok(())
    }

    /// Writes the hdlr (handler) box
    fn write_hdlr(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];

        let (handler_type, name) = match stream.media_type {
            MediaType::Video => (HANDLER_VIDEO, "VideoHandler"),
            MediaType::Audio => (HANDLER_SOUND, "SoundHandler"),
            _ => (0, "DataHandler"),
        };

        let hdlr_start = write_full_box_header(&mut self.writer, HDLR, 0, 0)?;

        write_u32(&mut self.writer, 0)?; // pre_defined
        write_u32(&mut self.writer, handler_type)?; // handler_type
        write_zeros(&mut self.writer, 12)?; // reserved
        self.writer.write_all(name.as_bytes())?; // name (null-terminated)
        write_u8(&mut self.writer, 0)?; // null terminator

        let hdlr_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, hdlr_start, (hdlr_end - hdlr_start) as u32)?;
        Ok(())
    }

    /// Writes the minf (media information) box
    fn write_minf(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];

        let minf_start = write_box_header_placeholder(&mut self.writer, MINF)?;

        // Write vmhd or smhd based on media type
        match stream.media_type {
            MediaType::Video => self.write_vmhd()?,
            MediaType::Audio => self.write_smhd()?,
            _ => {}
        }

        self.write_dinf()?;
        self.write_stbl(track_index)?;

        let minf_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, minf_start, (minf_end - minf_start) as u32)?;
        Ok(())
    }

    /// Writes the vmhd (video media header) box
    fn write_vmhd(&mut self) -> Result<()> {
        let vmhd_start = write_full_box_header(&mut self.writer, VMHD, 0, 1)?; // flags=1

        write_u16(&mut self.writer, 0)?; // graphicsmode
        write_u16(&mut self.writer, 0)?; // opcolor[0]
        write_u16(&mut self.writer, 0)?; // opcolor[1]
        write_u16(&mut self.writer, 0)?; // opcolor[2]

        let vmhd_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, vmhd_start, (vmhd_end - vmhd_start) as u32)?;
        Ok(())
    }

    /// Writes the smhd (sound media header) box
    fn write_smhd(&mut self) -> Result<()> {
        let smhd_start = write_full_box_header(&mut self.writer, SMHD, 0, 0)?;

        write_i16(&mut self.writer, 0)?; // balance
        write_u16(&mut self.writer, 0)?; // reserved

        let smhd_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, smhd_start, (smhd_end - smhd_start) as u32)?;
        Ok(())
    }

    /// Writes the dinf (data information) box
    fn write_dinf(&mut self) -> Result<()> {
        let dinf_start = write_box_header_placeholder(&mut self.writer, DINF)?;

        // Write dref box
        let dref_start = write_full_box_header(&mut self.writer, DREF, 0, 0)?;
        write_u32(&mut self.writer, 1)?; // entry_count

        // Write url box (self-reference)
        let url_start = write_full_box_header(&mut self.writer, URL, 0, 1)?; // flags=1 means self-reference
        let url_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, url_start, (url_end - url_start) as u32)?;

        let dref_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, dref_start, (dref_end - dref_start) as u32)?;

        let dinf_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, dinf_start, (dinf_end - dinf_start) as u32)?;
        Ok(())
    }

    /// Writes the stbl (sample table) box
    fn write_stbl(&mut self, track_index: usize) -> Result<()> {
        let stbl_start = write_box_header_placeholder(&mut self.writer, STBL)?;

        self.write_stsd(track_index)?;
        self.write_stts(track_index)?;

        // Write ctts if needed
        if self.track_data[track_index].needs_ctts {
            self.write_ctts(track_index)?;
        }

        self.write_stsc(track_index)?;
        self.write_stsz(track_index)?;

        // Write stco or co64 based on offset sizes
        if self.track_data[track_index].needs_64bit_offsets() {
            self.write_co64(track_index)?;
        } else {
            self.write_stco(track_index)?;
        }

        // Write stss for video tracks (sync samples)
        if self.streams[track_index].media_type == MediaType::Video {
            self.write_stss(track_index)?;
        }

        let stbl_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, stbl_start, (stbl_end - stbl_start) as u32)?;
        Ok(())
    }

    /// Writes the stsd (sample description) box
    fn write_stsd(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];

        let stsd_start = write_full_box_header(&mut self.writer, STSD, 0, 0)?;
        write_u32(&mut self.writer, 1)?; // entry_count

        match stream.media_type {
            MediaType::Video => self.write_video_sample_entry(track_index)?,
            MediaType::Audio => self.write_audio_sample_entry(track_index)?,
            _ => {}
        }

        let stsd_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, stsd_start, (stsd_end - stsd_start) as u32)?;
        Ok(())
    }

    /// Writes a video sample entry (avc1, vp09, etc.)
    fn write_video_sample_entry(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];
        let codec = stream.codec.as_str();

        let (width, height) = match &stream.params {
            StreamParams::Video(v) => (v.width as u16, v.height as u16),
            _ => (0, 0),
        };

        let box_type = match codec {
            "h264" | "avc" | "avc1" => AVC1,
            "vp9" | "vp09" => VP09,
            _ => {
                return Err(Error::Unsupported(format!(
                    "Unsupported video codec: {}",
                    codec
                )))
            }
        };

        let entry_start = write_box_header_placeholder(&mut self.writer, box_type)?;

        // VisualSampleEntry fields
        write_zeros(&mut self.writer, 6)?; // reserved
        write_u16(&mut self.writer, 1)?; // data_reference_index
        write_u16(&mut self.writer, 0)?; // pre_defined
        write_u16(&mut self.writer, 0)?; // reserved
        write_zeros(&mut self.writer, 12)?; // pre_defined
        write_u16(&mut self.writer, width)?; // width
        write_u16(&mut self.writer, height)?; // height
        write_u32(&mut self.writer, 0x00480000)?; // horizresolution (72 dpi)
        write_u32(&mut self.writer, 0x00480000)?; // vertresolution (72 dpi)
        write_u32(&mut self.writer, 0)?; // reserved
        write_u16(&mut self.writer, 1)?; // frame_count
        write_zeros(&mut self.writer, 32)?; // compressorname
        write_u16(&mut self.writer, 0x0018)?; // depth (24-bit)
        write_i16(&mut self.writer, -1)?; // pre_defined

        // Write codec-specific configuration box
        match codec {
            "h264" | "avc" | "avc1" => self.write_avcc(track_index)?,
            "vp9" | "vp09" => self.write_vpcc(track_index)?,
            _ => {}
        }

        let entry_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, entry_start, (entry_end - entry_start) as u32)?;
        Ok(())
    }

    /// Writes the avcC (AVC decoder configuration) box
    fn write_avcc(&mut self, track_index: usize) -> Result<()> {
        let extra_data = self.streams[track_index].extra_data.clone();
        let sps_list = self.track_data[track_index].sps_list.clone();
        let pps_list = self.track_data[track_index].pps_list.clone();

        let avcc_start = write_box_header_placeholder(&mut self.writer, AVCC)?;

        // Case 1: extra_data is already in avcC format
        if is_avcc_format(&extra_data) {
            self.writer.write_all(&extra_data)?;
        }
        // Case 2: We have extracted SPS/PPS from extra_data or packets
        else if !sps_list.is_empty() {
            let avcc_data = build_avcc(&sps_list, &pps_list)?;
            self.writer.write_all(&avcc_data)?;
        }
        // Case 3: Try to parse extra_data as Annex B
        else if !extra_data.is_empty() {
            let (sps, pps) = extract_sps_pps(&extra_data);
            if !sps.is_empty() {
                let avcc_data = build_avcc(&sps, &pps)?;
                self.writer.write_all(&avcc_data)?;
            } else {
                return Err(Error::InvalidData(
                    "H.264 stream requires SPS/PPS in extra_data or first keyframe".to_string(),
                ));
            }
        }
        // Case 4: No SPS/PPS available
        else {
            return Err(Error::InvalidData(
                "H.264 stream requires SPS/PPS. Provide extra_data or ensure first packet contains headers.".to_string(),
            ));
        }

        let avcc_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, avcc_start, (avcc_end - avcc_start) as u32)?;
        Ok(())
    }

    /// Writes the vpcC (VP9 codec configuration) box
    fn write_vpcc(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];

        let (bit_depth, chroma_subsampling) = match &stream.params {
            StreamParams::Video(v) => (v.bit_depth, 1u8), // 1 = 4:2:0
            _ => (8, 1),
        };

        let vpcc_start = write_full_box_header(&mut self.writer, VPCC, 1, 0)?;

        write_u8(&mut self.writer, 0)?; // profile
        write_u8(&mut self.writer, 10)?; // level (level 1.0)
        let bit_depth_chroma = (bit_depth << 4) | (chroma_subsampling << 1);
        write_u8(&mut self.writer, bit_depth_chroma)?; // bitDepth(4) | chromaSubsampling(3) | videoFullRangeFlag(1)
        write_u8(&mut self.writer, 1)?; // colourPrimaries (BT.709)
        write_u8(&mut self.writer, 1)?; // transferCharacteristics (BT.709)
        write_u8(&mut self.writer, 1)?; // matrixCoefficients (BT.709)
        write_u16(&mut self.writer, 0)?; // codecIntializationDataSize

        let vpcc_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, vpcc_start, (vpcc_end - vpcc_start) as u32)?;
        Ok(())
    }

    /// Writes an audio sample entry (mp4a, Opus, etc.)
    fn write_audio_sample_entry(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];
        let codec = stream.codec.as_str();

        let (sample_rate, channels) = match &stream.params {
            StreamParams::Audio(a) => (a.sample_rate, a.channels as u16),
            _ => (48000, 2),
        };

        let box_type = match codec {
            "aac" | "mp4a" => MP4A,
            "opus" => OPUS,
            _ => {
                return Err(Error::Unsupported(format!(
                    "Unsupported audio codec: {}",
                    codec
                )))
            }
        };

        let entry_start = write_box_header_placeholder(&mut self.writer, box_type)?;

        // AudioSampleEntry fields
        write_zeros(&mut self.writer, 6)?; // reserved
        write_u16(&mut self.writer, 1)?; // data_reference_index
        write_u32(&mut self.writer, 0)?; // reserved
        write_u32(&mut self.writer, 0)?; // reserved
        write_u16(&mut self.writer, channels)?; // channelcount
        write_u16(&mut self.writer, 16)?; // samplesize
        write_u16(&mut self.writer, 0)?; // pre_defined
        write_u16(&mut self.writer, 0)?; // reserved

        // Sample rate as 16.16 fixed point
        // For Opus, this is always 48000 regardless of input sample rate
        let sr = if codec == "opus" { 48000 } else { sample_rate };
        write_u32(&mut self.writer, sr << 16)?; // samplerate

        // Write codec-specific configuration
        match codec {
            "aac" | "mp4a" => self.write_esds(track_index)?,
            "opus" => self.write_dops(track_index)?,
            _ => {}
        }

        let entry_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, entry_start, (entry_end - entry_start) as u32)?;
        Ok(())
    }

    /// Writes the esds (elementary stream descriptor) box for AAC
    fn write_esds(&mut self, track_index: usize) -> Result<()> {
        // Copy all needed values to avoid borrow issues
        let extra_data = self.streams[track_index].extra_data.clone();
        let bitrate = self.streams[track_index].bitrate.unwrap_or(128000) as u32;

        let (sample_rate, channels) = match &self.streams[track_index].params {
            StreamParams::Audio(a) => (a.sample_rate, a.channels as u8),
            _ => (48000, 2),
        };

        let esds_start = write_full_box_header(&mut self.writer, ESDS, 0, 0)?;

        // ES_Descriptor
        write_u8(&mut self.writer, 0x03)?; // ES_DescrTag
        let es_descr_size = 23 + extra_data.len();
        self.write_descr_length(es_descr_size)?;
        write_u16(&mut self.writer, 0)?; // ES_ID
        write_u8(&mut self.writer, 0)?; // flags

        // DecoderConfigDescriptor
        write_u8(&mut self.writer, 0x04)?; // DecoderConfigDescrTag
        let dec_config_size = 15 + extra_data.len();
        self.write_descr_length(dec_config_size)?;
        write_u8(&mut self.writer, 0x40)?; // objectTypeIndication (AAC)
        write_u8(&mut self.writer, 0x15)?; // streamType (5=audio) << 2 | upStream(0) << 1 | reserved(1)
        write_u8(&mut self.writer, 0)?; // bufferSizeDB (24-bit)
        write_u16(&mut self.writer, 0)?;
        write_u32(&mut self.writer, bitrate)?; // maxBitrate
        write_u32(&mut self.writer, bitrate)?; // avgBitrate

        // DecoderSpecificInfo
        write_u8(&mut self.writer, 0x05)?; // DecSpecificInfoTag
        if !extra_data.is_empty() {
            self.write_descr_length(extra_data.len())?;
            self.writer.write_all(&extra_data)?;
        } else {
            // Generate basic AudioSpecificConfig
            let asc = self.generate_aac_config(sample_rate, channels);
            self.write_descr_length(asc.len())?;
            self.writer.write_all(&asc)?;
        }

        // SLConfigDescriptor
        write_u8(&mut self.writer, 0x06)?; // SLConfigDescrTag
        self.write_descr_length(1)?;
        write_u8(&mut self.writer, 0x02)?; // predefined = 2

        let esds_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, esds_start, (esds_end - esds_start) as u32)?;
        Ok(())
    }

    /// Writes descriptor length in expandable format
    fn write_descr_length(&mut self, len: usize) -> Result<()> {
        // MPEG-4 expandable length encoding
        if len < 128 {
            write_u8(&mut self.writer, len as u8)?;
        } else if len < 16384 {
            write_u8(&mut self.writer, ((len >> 7) | 0x80) as u8)?;
            write_u8(&mut self.writer, (len & 0x7F) as u8)?;
        } else {
            write_u8(&mut self.writer, ((len >> 14) | 0x80) as u8)?;
            write_u8(&mut self.writer, (((len >> 7) & 0x7F) | 0x80) as u8)?;
            write_u8(&mut self.writer, (len & 0x7F) as u8)?;
        }
        Ok(())
    }

    /// Generates a basic AAC AudioSpecificConfig
    fn generate_aac_config(&self, sample_rate: u32, channels: u8) -> Vec<u8> {
        // AAC-LC AudioSpecificConfig (2 bytes minimum)
        let audio_object_type = 2u8; // AAC-LC
        let sampling_freq_index = match sample_rate {
            96000 => 0,
            88200 => 1,
            64000 => 2,
            48000 => 3,
            44100 => 4,
            32000 => 5,
            24000 => 6,
            22050 => 7,
            16000 => 8,
            12000 => 9,
            11025 => 10,
            8000 => 11,
            _ => 4, // Default to 44100
        };
        let channel_config = channels.min(7);

        // Pack into 2 bytes: AAAAA SSS SCCC C000
        let byte1 = (audio_object_type << 3) | (sampling_freq_index >> 1);
        let byte2 = ((sampling_freq_index & 1) << 7) | (channel_config << 3);

        vec![byte1, byte2]
    }

    /// Writes the dOps (Opus specific) box
    fn write_dops(&mut self, track_index: usize) -> Result<()> {
        let stream = &self.streams[track_index];

        let (sample_rate, channels) = match &stream.params {
            StreamParams::Audio(a) => (a.sample_rate, a.channels as u8),
            _ => (48000, 2),
        };

        let dops_start = write_box_header_placeholder(&mut self.writer, DOPS)?;

        write_u8(&mut self.writer, 0)?; // Version
        write_u8(&mut self.writer, channels)?; // OutputChannelCount
        write_u16(&mut self.writer, 312)?; // PreSkip (typical value)
        write_u32(&mut self.writer, sample_rate)?; // InputSampleRate
        write_i16(&mut self.writer, 0)?; // OutputGain
        write_u8(&mut self.writer, 0)?; // ChannelMappingFamily (0 = mono/stereo)

        let dops_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, dops_start, (dops_end - dops_start) as u32)?;
        Ok(())
    }

    /// Writes the stts (time-to-sample) box
    fn write_stts(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        let stts_start = write_full_box_header(&mut self.writer, STTS, 0, 0)?;

        write_u32(&mut self.writer, track_data.time_to_sample.len() as u32)?; // entry_count

        for entry in &track_data.time_to_sample {
            write_u32(&mut self.writer, entry.sample_count)?;
            write_u32(&mut self.writer, entry.sample_delta)?;
        }

        let stts_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, stts_start, (stts_end - stts_start) as u32)?;
        Ok(())
    }

    /// Writes the ctts (composition time-to-sample) box
    fn write_ctts(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        // Use version 1 to allow negative offsets
        let ctts_start = write_full_box_header(&mut self.writer, CTTS, 1, 0)?;

        write_u32(&mut self.writer, track_data.composition_offsets.len() as u32)?;

        for entry in &track_data.composition_offsets {
            write_u32(&mut self.writer, entry.sample_count)?;
            write_i32(&mut self.writer, entry.sample_offset)?;
        }

        let ctts_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, ctts_start, (ctts_end - ctts_start) as u32)?;
        Ok(())
    }

    /// Writes the stsc (sample-to-chunk) box
    fn write_stsc(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        let stsc_start = write_full_box_header(&mut self.writer, STSC, 0, 0)?;

        write_u32(&mut self.writer, track_data.sample_to_chunk.len() as u32)?;

        for entry in &track_data.sample_to_chunk {
            write_u32(&mut self.writer, entry.first_chunk)?;
            write_u32(&mut self.writer, entry.samples_per_chunk)?;
            write_u32(&mut self.writer, entry.sample_description_index)?;
        }

        let stsc_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, stsc_start, (stsc_end - stsc_start) as u32)?;
        Ok(())
    }

    /// Writes the stsz (sample size) box
    fn write_stsz(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        let stsz_start = write_full_box_header(&mut self.writer, STSZ, 0, 0)?;

        write_u32(&mut self.writer, 0)?; // sample_size (0 = variable sizes)
        write_u32(&mut self.writer, track_data.sample_count)?; // sample_count

        for &size in &track_data.sample_sizes {
            write_u32(&mut self.writer, size)?;
        }

        let stsz_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, stsz_start, (stsz_end - stsz_start) as u32)?;
        Ok(())
    }

    /// Writes the stco (chunk offset) box - 32-bit offsets
    fn write_stco(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        let stco_start = write_full_box_header(&mut self.writer, STCO, 0, 0)?;

        write_u32(&mut self.writer, track_data.chunk_offsets.len() as u32)?;

        for &offset in &track_data.chunk_offsets {
            write_u32(&mut self.writer, offset as u32)?;
        }

        let stco_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, stco_start, (stco_end - stco_start) as u32)?;
        Ok(())
    }

    /// Writes the co64 (chunk offset) box - 64-bit offsets
    fn write_co64(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        let co64_start = write_full_box_header(&mut self.writer, CO64, 0, 0)?;

        write_u32(&mut self.writer, track_data.chunk_offsets.len() as u32)?;

        for &offset in &track_data.chunk_offsets {
            write_u64(&mut self.writer, offset)?;
        }

        let co64_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, co64_start, (co64_end - co64_start) as u32)?;
        Ok(())
    }

    /// Writes the stss (sync sample) box for video tracks
    fn write_stss(&mut self, track_index: usize) -> Result<()> {
        let track_data = &self.track_data[track_index];

        // Don't write stss if all samples are sync samples (all keyframes)
        if track_data.sync_samples.len() == track_data.sample_count as usize {
            return Ok(());
        }

        let stss_start = write_full_box_header(&mut self.writer, STSS, 0, 0)?;

        write_u32(&mut self.writer, track_data.sync_samples.len() as u32)?;

        for &sample_number in &track_data.sync_samples {
            write_u32(&mut self.writer, sample_number)?;
        }

        let stss_end = self.writer.stream_position()?;
        update_box_size(&mut self.writer, stss_start, (stss_end - stss_start) as u32)?;
        Ok(())
    }
}

/// Converts a timestamp from one timebase to another
fn convert_timestamp(ts: i64, from_timebase: (u32, u32), to_timescale: u32) -> i64 {
    // ts * to_timescale * from_timebase.0 / from_timebase.1
    let result = (ts as i128) * (to_timescale as i128) * (from_timebase.0 as i128)
        / (from_timebase.1 as i128);
    result as i64
}

impl<W: Write + Seek> Muxer for Mp4Muxer<W> {
    fn add_stream(&mut self, stream_info: StreamInfo) -> Result<usize> {
        if self.header_written {
            return Err(Error::InvalidState(
                "Cannot add stream after header is written".to_string(),
            ));
        }

        Self::validate_codec(&stream_info.codec, stream_info.media_type)?;

        // Calculate timescale from time_base
        let timescale = stream_info.time_base.1; // denominator is samples per second

        // Check if this is an H.264 stream
        let is_h264 = matches!(stream_info.codec.as_str(), "h264" | "avc" | "avc1");

        let index = self.streams.len();
        let mut stream = stream_info;
        stream.index = index;

        // Create track data
        let mut track_data = TrackData::new(timescale, is_h264);

        // Extract SPS/PPS from extra_data if available and it's H.264
        if is_h264 && !stream.extra_data.is_empty() {
            if is_avcc_format(&stream.extra_data) {
                // Already in avcC format - we'll use it directly in write_avcc
            } else {
                // Annex B format - extract SPS/PPS
                let (sps, pps) = extract_sps_pps(&stream.extra_data);
                track_data.sps_list = sps;
                track_data.pps_list = pps;
            }
        }

        self.streams.push(stream);
        self.track_data.push(track_data);

        Ok(index)
    }

    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }

    fn write_header(&mut self) -> Result<()> {
        if self.header_written {
            return Err(Error::InvalidState("Header already written".to_string()));
        }

        if self.streams.is_empty() {
            return Err(Error::InvalidState(
                "No streams added before writing header".to_string(),
            ));
        }

        self.write_ftyp()?;
        self.write_mdat_header()?;

        self.header_written = true;
        Ok(())
    }

    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        if !self.header_written {
            return Err(Error::InvalidState(
                "Must write header before packets".to_string(),
            ));
        }

        if self.trailer_written {
            return Err(Error::InvalidState(
                "Cannot write packets after trailer".to_string(),
            ));
        }

        let stream_index = packet.stream_index();
        if stream_index >= self.streams.len() {
            return Err(Error::InvalidData(format!(
                "Invalid stream index: {}",
                stream_index
            )));
        }

        let is_video = self.streams[stream_index].media_type == MediaType::Video;
        let is_keyframe = packet.is_keyframe();
        let is_h264 = self.track_data[stream_index].is_h264;

        // Get timestamps, using 0 as default
        let pts = packet.pts().unwrap_or(0);
        let dts = packet.dts().unwrap_or(pts);

        // Convert timestamps to track timescale
        let time_base = self.streams[stream_index].time_base;
        let timescale = self.track_data[stream_index].timescale;
        let scaled_pts = convert_timestamp(pts, time_base, timescale);
        let scaled_dts = convert_timestamp(dts, time_base, timescale);

        // Record the chunk offset (current position in mdat)
        let offset = self.position;

        // Handle H.264 data conversion
        let data = packet.data();
        let write_data: std::borrow::Cow<[u8]> = if is_h264 {
            // For H.264, check if data is in Annex B format and convert to AVCC
            if find_start_code(data, 0).is_some() {
                // Annex B format detected - extract SPS/PPS from keyframes
                if is_keyframe && self.track_data[stream_index].sps_list.is_empty() {
                    let (sps, pps) = extract_sps_pps(data);
                    if !sps.is_empty() {
                        self.track_data[stream_index].sps_list = sps;
                        self.track_data[stream_index].pps_list = pps;
                    }
                }

                // Convert to AVCC format
                let avcc_data = annex_b_to_avcc(data);
                if avcc_data.is_empty() {
                    // No video NAL units (only SPS/PPS), skip this packet
                    return Ok(());
                }
                std::borrow::Cow::Owned(avcc_data)
            } else {
                // Already in AVCC format or raw NAL units
                std::borrow::Cow::Borrowed(data)
            }
        } else {
            std::borrow::Cow::Borrowed(data)
        };

        // Write packet data
        self.writer.write_all(&write_data)?;
        self.position += write_data.len() as u64;
        self.mdat_size += write_data.len() as u64;

        // Update track data
        self.track_data[stream_index].add_sample(
            write_data.len() as u32,
            offset,
            scaled_dts,
            scaled_pts,
            is_keyframe,
            is_video,
        );

        Ok(())
    }

    fn write_trailer(&mut self) -> Result<()> {
        if !self.header_written {
            return Err(Error::InvalidState(
                "Must write header before trailer".to_string(),
            ));
        }

        if self.trailer_written {
            return Err(Error::InvalidState("Trailer already written".to_string()));
        }

        // Update mdat size
        self.update_mdat_size()?;

        // Write moov box
        self.write_moov()?;

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
    use rust_media_core::stream::{AudioStreamParams, VideoStreamParams};
    use rust_media_core::types::{ColorRange, ColorSpace, PixelFormat, SampleFormat};
    use std::io::Cursor;

    fn create_test_video_stream() -> StreamInfo {
        StreamInfo {
            index: 0,
            media_type: MediaType::Video,
            codec: "h264".to_string(),
            time_base: (1, 30), // 30 fps
            duration: None,
            bitrate: Some(1_000_000),
            params: StreamParams::Video(VideoStreamParams {
                width: 1920,
                height: 1080,
                pixel_format: PixelFormat::YUV420P,
                frame_rate: (30, 1),
                color_space: ColorSpace::BT709,
                color_range: ColorRange::Limited,
                sample_aspect_ratio: (1, 1),
                bit_depth: 8,
            }),
            // Basic avcC format data
            extra_data: vec![
                0x01, 0x64, 0x00, 0x1F, // version, profile, compatibility, level
                0xFF, // lengthSizeMinusOne
                0xE1, 0x00, 0x10, // numSPS, spsLength
                0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x50, // SPS data (simplified)
                0x05, 0xBB, 0x01, 0x6A, 0x02, 0x02, 0x02, 0x80, 0x01, 0x00, 0x04, // PPS
                0x68, 0xEE, 0x3C, 0x80, // PPS data
            ],
        }
    }

    fn create_test_audio_stream() -> StreamInfo {
        StreamInfo {
            index: 1,
            media_type: MediaType::Audio,
            codec: "aac".to_string(),
            time_base: (1, 48000),
            duration: None,
            bitrate: Some(128_000),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 48000,
                channels: 2,
                sample_format: SampleFormat::F32,
                channel_layout: "stereo".to_string(),
                bits_per_sample: 32,
                frame_size: Some(1024),
            }),
            extra_data: vec![0x11, 0x90], // AAC-LC, 48kHz, stereo
        }
    }

    #[test]
    fn test_create_muxer() {
        let buf = Cursor::new(Vec::new());
        let muxer = Mp4Muxer::new(buf);
        assert!(!muxer.header_written);
        assert!(!muxer.trailer_written);
    }

    #[test]
    fn test_add_stream() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        let video = create_test_video_stream();
        let index = muxer.add_stream(video).unwrap();
        assert_eq!(index, 0);
        assert_eq!(muxer.streams().len(), 1);
    }

    #[test]
    fn test_add_stream_after_header_fails() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        muxer.add_stream(create_test_video_stream()).unwrap();
        muxer.write_header().unwrap();

        let result = muxer.add_stream(create_test_audio_stream());
        assert!(result.is_err());
    }

    #[test]
    fn test_unsupported_codec() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        let mut stream = create_test_video_stream();
        stream.codec = "unsupported".to_string();

        let result = muxer.add_stream(stream);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_header() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        muxer.add_stream(create_test_video_stream()).unwrap();
        muxer.write_header().unwrap();

        assert!(muxer.header_written);
        assert!(muxer.position > 0);
    }

    #[test]
    fn test_write_header_without_streams_fails() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        let result = muxer.write_header();
        assert!(result.is_err());
    }

    #[test]
    fn test_write_packet_before_header_fails() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        muxer.add_stream(create_test_video_stream()).unwrap();

        let packet = Packet::new(vec![0u8; 100], 0, MediaType::Video);
        let result = muxer.write_packet(&packet);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_muxing_pipeline() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        // Add streams
        muxer.add_stream(create_test_video_stream()).unwrap();

        // Write header
        muxer.write_header().unwrap();

        // Write some packets
        for i in 0..10 {
            let mut packet = Packet::new(vec![0u8; 1000], 0, MediaType::Video);
            packet = packet.with_pts(i * 3000); // 30fps = 3000 ticks per frame at 90kHz
            packet = packet.with_dts(i * 3000);
            if i == 0 {
                packet = packet.with_keyframe();
            }
            muxer.write_packet(&packet).unwrap();
        }

        // Write trailer
        muxer.write_trailer().unwrap();

        // Verify output
        let output = muxer.writer.into_inner();
        assert!(!output.is_empty());

        // Check ftyp box signature
        assert_eq!(&output[4..8], b"ftyp");

        // Check mdat box exists
        let mdat_pos = output
            .windows(4)
            .position(|w| w == b"mdat")
            .expect("mdat not found");
        assert!(mdat_pos > 0);

        // Check moov box exists
        let moov_pos = output
            .windows(4)
            .position(|w| w == b"moov")
            .expect("moov not found");
        assert!(moov_pos > mdat_pos); // moov should come after mdat
    }

    #[test]
    fn test_video_and_audio_streams() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        muxer.add_stream(create_test_video_stream()).unwrap();
        muxer.add_stream(create_test_audio_stream()).unwrap();
        muxer.write_header().unwrap();

        // Write video packets
        for i in 0..5 {
            let mut packet = Packet::new(vec![0u8; 1000], 0, MediaType::Video);
            packet = packet.with_pts(i * 1).with_dts(i * 1);
            if i == 0 {
                packet = packet.with_keyframe();
            }
            muxer.write_packet(&packet).unwrap();
        }

        // Write audio packets
        for i in 0..10 {
            let packet = Packet::new(vec![0u8; 500], 1, MediaType::Audio)
                .with_pts(i * 1024)
                .with_dts(i * 1024);
            muxer.write_packet(&packet).unwrap();
        }

        muxer.write_trailer().unwrap();

        let output = muxer.writer.into_inner();

        // Verify trak boxes exist (should be 2)
        let trak_count = output.windows(4).filter(|w| *w == b"trak").count();
        assert_eq!(trak_count, 2);
    }

    #[test]
    fn test_opus_stream() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        let opus_stream = StreamInfo {
            index: 0,
            media_type: MediaType::Audio,
            codec: "opus".to_string(),
            time_base: (1, 48000),
            duration: None,
            bitrate: Some(64_000),
            params: StreamParams::Audio(AudioStreamParams {
                sample_rate: 48000,
                channels: 2,
                sample_format: SampleFormat::F32,
                channel_layout: "stereo".to_string(),
                bits_per_sample: 32,
                frame_size: Some(960),
            }),
            extra_data: vec![],
        };

        muxer.add_stream(opus_stream).unwrap();
        muxer.write_header().unwrap();

        // Write a packet
        let packet = Packet::new(vec![0u8; 100], 0, MediaType::Audio)
            .with_pts(0)
            .with_dts(0);
        muxer.write_packet(&packet).unwrap();

        muxer.write_trailer().unwrap();

        let output = muxer.writer.into_inner();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_vp9_stream() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        let vp9_stream = StreamInfo {
            index: 0,
            media_type: MediaType::Video,
            codec: "vp9".to_string(),
            time_base: (1, 30),
            duration: None,
            bitrate: Some(2_000_000),
            params: StreamParams::Video(VideoStreamParams {
                width: 1280,
                height: 720,
                pixel_format: PixelFormat::YUV420P,
                frame_rate: (30, 1),
                color_space: ColorSpace::BT709,
                color_range: ColorRange::Limited,
                sample_aspect_ratio: (1, 1),
                bit_depth: 8,
            }),
            extra_data: vec![],
        };

        muxer.add_stream(vp9_stream).unwrap();
        muxer.write_header().unwrap();

        let packet = Packet::new(vec![0u8; 5000], 0, MediaType::Video)
            .with_pts(0)
            .with_dts(0)
            .with_keyframe();
        muxer.write_packet(&packet).unwrap();

        muxer.write_trailer().unwrap();

        let output = muxer.writer.into_inner();
        assert!(!output.is_empty());

        // Check vp09 box exists
        assert!(output.windows(4).any(|w| w == b"vp09"));
    }

    #[test]
    fn test_timestamp_conversion() {
        // 90000 Hz timebase to 1000 Hz (milliseconds)
        let ts = convert_timestamp(90000, (1, 90000), 1000);
        assert_eq!(ts, 1000); // 1 second

        // 30 fps timebase (1/30) to milliseconds
        let ts = convert_timestamp(30, (1, 30), 1000);
        assert_eq!(ts, 1000); // 30 frames at 30fps = 1 second
    }

    // ========================================================================
    // H.264 NAL Unit Parsing Tests
    // ========================================================================

    #[test]
    fn test_find_start_code_3_byte() {
        let data = [0x00, 0x00, 0x01, 0x65, 0x88]; // 3-byte start code + IDR frame
        let result = find_start_code(&data, 0);
        assert_eq!(result, Some((0, 3)));
    }

    #[test]
    fn test_find_start_code_4_byte() {
        let data = [0x00, 0x00, 0x00, 0x01, 0x67, 0x64]; // 4-byte start code + SPS
        let result = find_start_code(&data, 0);
        assert_eq!(result, Some((0, 4)));
    }

    #[test]
    fn test_find_start_code_with_offset() {
        let data = [0xFF, 0xFF, 0x00, 0x00, 0x01, 0x65];
        let result = find_start_code(&data, 0);
        assert_eq!(result, Some((2, 3)));
    }

    #[test]
    fn test_find_start_code_not_found() {
        let data = [0x00, 0x00, 0x02, 0x65];
        let result = find_start_code(&data, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_start_code_empty() {
        let data: [u8; 0] = [];
        let result = find_start_code(&data, 0);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_annex_b_single_nal() {
        // 4-byte start code + SPS NAL
        let data = [0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F];
        let nal_units = parse_annex_b_nal_units(&data);
        assert_eq!(nal_units.len(), 1);
        assert_eq!(nal_units[0], vec![0x67, 0x64, 0x00, 0x1F]);
    }

    #[test]
    fn test_parse_annex_b_multiple_nals() {
        // SPS + PPS
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F, // SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xEE, 0x3C, 0x80, // PPS
        ];
        let nal_units = parse_annex_b_nal_units(&data);
        assert_eq!(nal_units.len(), 2);
        assert_eq!(nal_units[0], vec![0x67, 0x64, 0x00, 0x1F]);
        assert_eq!(nal_units[1], vec![0x68, 0xEE, 0x3C, 0x80]);
    }

    #[test]
    fn test_parse_annex_b_mixed_start_codes() {
        // 3-byte start code + 4-byte start code
        let data = [
            0x00, 0x00, 0x01, 0x67, 0x64, // 3-byte + SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xEE, // 4-byte + PPS
        ];
        let nal_units = parse_annex_b_nal_units(&data);
        assert_eq!(nal_units.len(), 2);
        assert_eq!(nal_units[0][0] & 0x1F, NAL_TYPE_SPS);
        assert_eq!(nal_units[1][0] & 0x1F, NAL_TYPE_PPS);
    }

    #[test]
    fn test_parse_annex_b_no_start_code() {
        // Raw NAL unit without start code
        let data = [0x67, 0x64, 0x00, 0x1F];
        let nal_units = parse_annex_b_nal_units(&data);
        assert_eq!(nal_units.len(), 1);
        assert_eq!(nal_units[0], data.to_vec());
    }

    #[test]
    fn test_extract_sps_pps() {
        // SPS + PPS + IDR frame
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F, // SPS (type 7)
            0x00, 0x00, 0x00, 0x01, 0x68, 0xEE, 0x3C, 0x80, // PPS (type 8)
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, // IDR (type 5)
        ];
        let (sps_list, pps_list) = extract_sps_pps(&data);
        assert_eq!(sps_list.len(), 1);
        assert_eq!(pps_list.len(), 1);
        assert_eq!(sps_list[0][0] & 0x1F, NAL_TYPE_SPS);
        assert_eq!(pps_list[0][0] & 0x1F, NAL_TYPE_PPS);
    }

    #[test]
    fn test_extract_sps_pps_multiple() {
        // Multiple SPS/PPS (can happen with adaptive streaming)
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F, // SPS 1
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x28, // SPS 2
            0x00, 0x00, 0x00, 0x01, 0x68, 0xEE, 0x3C, 0x80, // PPS 1
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80, // PPS 2
        ];
        let (sps_list, pps_list) = extract_sps_pps(&data);
        assert_eq!(sps_list.len(), 2);
        assert_eq!(pps_list.len(), 2);
    }

    #[test]
    fn test_extract_sps_pps_none() {
        // Data without SPS/PPS
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, // IDR frame only
        ];
        let (sps_list, pps_list) = extract_sps_pps(&data);
        assert!(sps_list.is_empty());
        assert!(pps_list.is_empty());
    }

    #[test]
    fn test_annex_b_to_avcc() {
        // SPS + PPS + IDR (only IDR should be in output)
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F, // SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xEE, 0x3C, 0x80, // PPS
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21, // IDR (no trailing zeros)
        ];
        let avcc = annex_b_to_avcc(&data);

        // Should only contain IDR NAL with 4-byte length prefix
        assert!(!avcc.is_empty());

        // Check length prefix (big-endian)
        let len = u32::from_be_bytes([avcc[0], avcc[1], avcc[2], avcc[3]]);
        assert_eq!(len, 4); // IDR NAL length (0x65, 0x88, 0x84, 0x21)

        // Check NAL type (IDR = 5)
        assert_eq!(avcc[4] & 0x1F, 5);
    }

    #[test]
    fn test_annex_b_to_avcc_multiple_frames() {
        // Two non-SPS/PPS NAL units
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, // IDR
            0x00, 0x00, 0x01, 0x41, 0x9A, 0x24, // Non-IDR P-frame (3-byte start code)
        ];
        let avcc = annex_b_to_avcc(&data);

        // Should have two NAL units with length prefixes
        assert!(!avcc.is_empty());

        // First NAL
        let len1 = u32::from_be_bytes([avcc[0], avcc[1], avcc[2], avcc[3]]);
        assert_eq!(len1, 3); // IDR NAL length

        // Second NAL starts after first
        let offset = 4 + len1 as usize;
        let len2 = u32::from_be_bytes([avcc[offset], avcc[offset + 1], avcc[offset + 2], avcc[offset + 3]]);
        assert_eq!(len2, 3); // P-frame NAL length
    }

    #[test]
    fn test_annex_b_to_avcc_only_sps_pps() {
        // Only SPS/PPS, no frame data
        let data = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x1F, // SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xEE, 0x3C, 0x80, // PPS
        ];
        let avcc = annex_b_to_avcc(&data);

        // Should be empty (SPS/PPS go in avcC box, not frame data)
        assert!(avcc.is_empty());
    }

    #[test]
    fn test_is_avcc_format() {
        // Valid avcC starts with version 1
        let avcc = [0x01, 0x64, 0x00, 0x1F, 0xFF];
        assert!(is_avcc_format(&avcc));

        // Annex B starts with start code
        let annex_b = [0x00, 0x00, 0x00, 0x01, 0x67];
        assert!(!is_avcc_format(&annex_b));

        // Empty data
        let empty: [u8; 0] = [];
        assert!(!is_avcc_format(&empty));
    }

    #[test]
    fn test_build_avcc() {
        let sps = vec![vec![0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9]];
        let pps = vec![vec![0x68, 0xEE, 0x3C, 0x80]];

        let avcc = build_avcc(&sps, &pps).unwrap();

        // Check version
        assert_eq!(avcc[0], 1);

        // Check profile/level from SPS
        assert_eq!(avcc[1], 0x64); // profile
        assert_eq!(avcc[2], 0x00); // compatibility
        assert_eq!(avcc[3], 0x1F); // level

        // Check length size minus one (should be 3 for 4-byte NAL lengths)
        assert_eq!(avcc[4] & 0x03, 3);

        // Check SPS count
        assert_eq!(avcc[5] & 0x1F, 1);
    }

    #[test]
    fn test_build_avcc_no_sps() {
        let sps: Vec<Vec<u8>> = vec![];
        let pps = vec![vec![0x68, 0xEE, 0x3C, 0x80]];

        let result = build_avcc(&sps, &pps);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_avcc_short_sps() {
        let sps = vec![vec![0x67, 0x64]]; // Too short (need at least 4 bytes)
        let pps = vec![vec![0x68, 0xEE, 0x3C, 0x80]];

        let result = build_avcc(&sps, &pps);
        assert!(result.is_err());
    }

    // ========================================================================
    // H.264 Muxing Integration Tests
    // ========================================================================

    fn create_h264_annex_b_stream() -> StreamInfo {
        // Create SPS/PPS in Annex B format for extra_data
        let extra_data = vec![
            // SPS (4-byte start code + NAL)
            0x00, 0x00, 0x00, 0x01,
            0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x50,
            0x05, 0xBB, 0x01, 0x10,
            // PPS (4-byte start code + NAL)
            0x00, 0x00, 0x00, 0x01,
            0x68, 0xEE, 0x3C, 0x80,
        ];

        StreamInfo {
            index: 0,
            media_type: MediaType::Video,
            codec: "h264".to_string(),
            time_base: (1, 30),
            duration: None,
            bitrate: Some(1_000_000),
            params: StreamParams::Video(VideoStreamParams {
                width: 1920,
                height: 1080,
                pixel_format: PixelFormat::YUV420P,
                frame_rate: (30, 1),
                color_space: ColorSpace::BT709,
                color_range: ColorRange::Limited,
                sample_aspect_ratio: (1, 1),
                bit_depth: 8,
            }),
            extra_data,
        }
    }

    #[test]
    fn test_h264_muxing_with_annex_b_extra_data() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        // Add H.264 stream with Annex B SPS/PPS in extra_data
        muxer.add_stream(create_h264_annex_b_stream()).unwrap();
        muxer.write_header().unwrap();

        // Write keyframe with Annex B format
        let keyframe_data = vec![
            // SPS (duplicated in frame - common with x264)
            0x00, 0x00, 0x00, 0x01,
            0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x50,
            0x05, 0xBB, 0x01, 0x10,
            // PPS
            0x00, 0x00, 0x00, 0x01,
            0x68, 0xEE, 0x3C, 0x80,
            // IDR frame
            0x00, 0x00, 0x00, 0x01,
            0x65, 0x88, 0x84, 0x00, 0x33, 0xFF,
        ];
        let packet = Packet::new(keyframe_data, 0, MediaType::Video)
            .with_pts(0)
            .with_dts(0)
            .with_keyframe();
        muxer.write_packet(&packet).unwrap();

        // Write P-frame with Annex B format
        let p_frame_data = vec![
            0x00, 0x00, 0x00, 0x01,
            0x41, 0x9A, 0x24, 0x6C, 0x41,
        ];
        let packet = Packet::new(p_frame_data, 0, MediaType::Video)
            .with_pts(1)
            .with_dts(1);
        muxer.write_packet(&packet).unwrap();

        muxer.write_trailer().unwrap();

        let output = muxer.writer.into_inner();

        // Verify avcC box exists
        assert!(output.windows(4).any(|w| w == b"avcC"));

        // Verify avc1 box exists
        assert!(output.windows(4).any(|w| w == b"avc1"));

        // Verify no Annex B start codes in mdat data area
        // Find mdat position and check data after it
        let mdat_pos = output.windows(4).position(|w| w == b"mdat").unwrap();
        let moov_pos = output.windows(4).position(|w| w == b"moov").unwrap();
        let mdat_data = &output[mdat_pos + 8..moov_pos]; // Skip mdat header

        // There should be no 4-byte start codes in the converted data
        let has_4byte_start_code = mdat_data.windows(4).any(|w| w == [0x00, 0x00, 0x00, 0x01]);
        assert!(!has_4byte_start_code, "Found Annex B start code in mdat - conversion failed");
    }

    #[test]
    fn test_h264_muxing_extracts_sps_pps_from_first_keyframe() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        // Create stream with empty extra_data (SPS/PPS will come from first keyframe)
        let stream = StreamInfo {
            index: 0,
            media_type: MediaType::Video,
            codec: "h264".to_string(),
            time_base: (1, 30),
            duration: None,
            bitrate: Some(1_000_000),
            params: StreamParams::Video(VideoStreamParams {
                width: 1920,
                height: 1080,
                pixel_format: PixelFormat::YUV420P,
                frame_rate: (30, 1),
                color_space: ColorSpace::BT709,
                color_range: ColorRange::Limited,
                sample_aspect_ratio: (1, 1),
                bit_depth: 8,
            }),
            extra_data: vec![],
        };

        muxer.add_stream(stream).unwrap();
        muxer.write_header().unwrap();

        // First keyframe contains SPS/PPS
        let keyframe_data = vec![
            // SPS
            0x00, 0x00, 0x00, 0x01,
            0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9, 0x40, 0x50,
            0x05, 0xBB, 0x01, 0x10,
            // PPS
            0x00, 0x00, 0x00, 0x01,
            0x68, 0xEE, 0x3C, 0x80,
            // IDR frame
            0x00, 0x00, 0x00, 0x01,
            0x65, 0x88, 0x84, 0x00, 0x33, 0xFF,
        ];
        let packet = Packet::new(keyframe_data, 0, MediaType::Video)
            .with_pts(0)
            .with_dts(0)
            .with_keyframe();
        muxer.write_packet(&packet).unwrap();

        muxer.write_trailer().unwrap();

        let output = muxer.writer.into_inner();

        // Verify avcC box was created from keyframe SPS/PPS
        assert!(output.windows(4).any(|w| w == b"avcC"));
    }

    #[test]
    fn test_h264_muxing_skips_sps_pps_only_packet() {
        let buf = Cursor::new(Vec::new());
        let mut muxer = Mp4Muxer::new(buf);

        muxer.add_stream(create_h264_annex_b_stream()).unwrap();
        muxer.write_header().unwrap();

        // Packet with only SPS/PPS (no frame data)
        let sps_pps_only = vec![
            0x00, 0x00, 0x00, 0x01,
            0x67, 0x64, 0x00, 0x1F, 0xAC, 0xD9,
            0x00, 0x00, 0x00, 0x01,
            0x68, 0xEE, 0x3C, 0x80,
        ];
        let packet = Packet::new(sps_pps_only, 0, MediaType::Video)
            .with_pts(0)
            .with_dts(0)
            .with_keyframe();

        // Should succeed but not write any data to mdat
        muxer.write_packet(&packet).unwrap();

        // Now write actual frame
        let frame_data = vec![
            0x00, 0x00, 0x00, 0x01,
            0x65, 0x88, 0x84, 0x00, 0x33, 0xFF,
        ];
        let packet = Packet::new(frame_data, 0, MediaType::Video)
            .with_pts(1)
            .with_dts(1)
            .with_keyframe();
        muxer.write_packet(&packet).unwrap();

        muxer.write_trailer().unwrap();

        // Should complete successfully
        let output = muxer.writer.into_inner();
        assert!(!output.is_empty());
    }
}
