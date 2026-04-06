//! MP4 box reader utilities
//!
//! Provides utilities for reading and parsing MP4 boxes (atoms) from a stream.

use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Read, Seek, SeekFrom};

use crate::mp4::boxes::*;
use rust_media_core::error::Result;

/// Information about a box header
#[derive(Debug, Clone)]
pub struct BoxHeader {
    /// Box type (fourcc)
    pub box_type: u32,
    /// Total size of the box including header
    pub size: u64,
    /// Offset where the box header starts
    pub offset: u64,
    /// Size of the header itself (8 or 16 bytes)
    pub header_size: u8,
}

impl BoxHeader {
    /// Returns the size of the box content (excluding header)
    pub fn content_size(&self) -> u64 {
        self.size.saturating_sub(self.header_size as u64)
    }

    /// Returns the offset where the content starts
    pub fn content_offset(&self) -> u64 {
        self.offset + self.header_size as u64
    }

    /// Returns the fourcc as a string for debugging
    pub fn type_str(&self) -> String {
        let bytes = self.box_type.to_be_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }
}

/// Reads a box header from the current position
pub fn read_box_header<R: Read + Seek>(reader: &mut R) -> Result<BoxHeader> {
    let offset = reader.stream_position()?;

    // Read size (4 bytes) and type (4 bytes)
    let size32 = reader.read_u32::<BigEndian>()?;
    let box_type = reader.read_u32::<BigEndian>()?;

    let (size, header_size) = if size32 == 1 {
        // Extended size (64-bit)
        let size64 = reader.read_u64::<BigEndian>()?;
        (size64, 16u8)
    } else if size32 == 0 {
        // Box extends to end of file - we need to calculate this
        let current = reader.stream_position()?;
        let end = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(current))?;
        (end - offset, 8u8)
    } else {
        (size32 as u64, 8u8)
    };

    Ok(BoxHeader {
        box_type,
        size,
        offset,
        header_size,
    })
}

/// Skips past the current box
pub fn skip_box<R: Read + Seek>(reader: &mut R, header: &BoxHeader) -> Result<()> {
    let end = header.offset + header.size;
    reader.seek(SeekFrom::Start(end))?;
    Ok(())
}

/// Reads bytes from the box content
pub fn read_box_content<R: Read>(reader: &mut R, size: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; size];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

/// Finds a child box of the given type within a container box
pub fn find_box<R: Read + Seek>(
    reader: &mut R,
    container_end: u64,
    box_type: u32,
) -> Result<Option<BoxHeader>> {
    while reader.stream_position()? < container_end {
        let header = read_box_header(reader)?;
        if header.box_type == box_type {
            // Seek back to content start
            reader.seek(SeekFrom::Start(header.content_offset()))?;
            return Ok(Some(header));
        }
        skip_box(reader, &header)?;
    }
    Ok(None)
}

/// Iterates over all child boxes in a container
pub fn iterate_boxes<R: Read + Seek, F>(
    reader: &mut R,
    container_end: u64,
    mut callback: F,
) -> Result<()>
where
    F: FnMut(&mut R, BoxHeader) -> Result<bool>,
{
    while reader.stream_position()? < container_end {
        let header = read_box_header(reader)?;
        let should_continue = callback(reader, header.clone())?;
        if !should_continue {
            return Ok(());
        }
        skip_box(reader, &header)?;
    }
    Ok(())
}

/// Parsed sample description for audio
#[derive(Debug, Clone)]
pub struct AudioSampleEntry {
    pub codec: String,
    pub channels: u16,
    pub sample_size: u16,
    pub sample_rate: u32,
    /// Codec-specific data (e.g., AudioSpecificConfig for AAC)
    pub extra_data: Vec<u8>,
}

/// Parsed sample description for video
#[derive(Debug, Clone)]
pub struct VideoSampleEntry {
    pub codec: String,
    pub width: u16,
    pub height: u16,
    /// Codec-specific data (e.g., avcC for H.264)
    pub extra_data: Vec<u8>,
}

/// Sample description (codec info)
#[derive(Debug, Clone)]
pub enum SampleEntry {
    Audio(AudioSampleEntry),
    Video(VideoSampleEntry),
    Unknown,
}

/// Parses the stsd (sample description) box
pub fn parse_stsd<R: Read + Seek>(reader: &mut R, size: u64) -> Result<Vec<SampleEntry>> {
    let start = reader.stream_position()?;
    let end = start + size;

    // Version and flags
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    // Entry count
    let entry_count = reader.read_u32::<BigEndian>()?;

    let mut entries = Vec::new();

    for _ in 0..entry_count {
        if reader.stream_position()? >= end {
            break;
        }

        let entry_header = read_box_header(reader)?;
        let entry_end = entry_header.offset + entry_header.size;

        let entry = match entry_header.box_type {
            MP4A => parse_mp4a_entry(reader, &entry_header)?,
            AVC1 => parse_avc1_entry(reader, &entry_header)?,
            VP09 => parse_vp09_entry(reader, &entry_header)?,
            OPUS => parse_opus_entry(reader, &entry_header)?,
            _ => SampleEntry::Unknown,
        };

        entries.push(entry);

        // Move to end of entry
        reader.seek(SeekFrom::Start(entry_end))?;
    }

    Ok(entries)
}

/// Parses mp4a (AAC) sample entry
fn parse_mp4a_entry<R: Read + Seek>(reader: &mut R, header: &BoxHeader) -> Result<SampleEntry> {
    let entry_end = header.offset + header.size;

    // Skip reserved (6 bytes) + data reference index (2 bytes)
    reader.seek(SeekFrom::Current(8))?;

    // Audio-specific fields
    let _version = reader.read_u16::<BigEndian>()?;
    let _revision = reader.read_u16::<BigEndian>()?;
    let _vendor = reader.read_u32::<BigEndian>()?;

    let channels = reader.read_u16::<BigEndian>()?;
    let sample_size = reader.read_u16::<BigEndian>()?;
    let _compression_id = reader.read_u16::<BigEndian>()?;
    let _packet_size = reader.read_u16::<BigEndian>()?;

    // Sample rate is 16.16 fixed point
    let sample_rate_fixed = reader.read_u32::<BigEndian>()?;
    let sample_rate = sample_rate_fixed >> 16;

    // Look for esds box with AudioSpecificConfig
    let mut extra_data = Vec::new();

    while reader.stream_position()? < entry_end {
        let child = read_box_header(reader)?;
        if child.box_type == ESDS {
            extra_data = parse_esds(reader, child.content_size() as usize)?;
            break;
        }
        skip_box(reader, &child)?;
    }

    Ok(SampleEntry::Audio(AudioSampleEntry {
        codec: "aac".to_string(),
        channels,
        sample_size,
        sample_rate,
        extra_data,
    }))
}

/// Parses esds box to extract AudioSpecificConfig
fn parse_esds<R: Read>(reader: &mut R, size: usize) -> Result<Vec<u8>> {
    let mut data = vec![0u8; size];
    reader.read_exact(&mut data)?;

    // ESDS structure:
    // - version (1) + flags (3)
    // - ES_Descriptor tag (1) + size (variable)
    // - ES_ID (2) + flags (1)
    // - DecoderConfigDescriptor tag (1) + size (variable)
    // - objectTypeId (1) + streamType (1) + bufferSize (3) + maxBitrate (4) + avgBitrate (4)
    // - DecoderSpecificInfo tag (1) + size (variable)
    // - AudioSpecificConfig data

    // Skip version and flags
    let mut pos = 4;

    // Find DecoderSpecificInfo (tag 0x05)
    while pos < data.len() {
        let tag = data[pos];
        pos += 1;

        // Read variable length size
        let mut desc_size = 0usize;
        for _ in 0..4 {
            if pos >= data.len() {
                break;
            }
            let b = data[pos];
            pos += 1;
            desc_size = (desc_size << 7) | ((b & 0x7F) as usize);
            if b & 0x80 == 0 {
                break;
            }
        }

        if tag == 0x05 {
            // DecoderSpecificInfo - this is the AudioSpecificConfig
            if pos + desc_size <= data.len() {
                return Ok(data[pos..pos + desc_size].to_vec());
            }
        }

        // Skip this descriptor
        if tag == 0x03 {
            // ES_Descriptor - skip ES_ID and flags
            pos += 3;
        } else if tag == 0x04 {
            // DecoderConfigDescriptor - skip to nested descriptors
            pos += 13;
        } else {
            pos += desc_size;
        }
    }

    Ok(Vec::new())
}

/// Parses avc1 (H.264) sample entry
fn parse_avc1_entry<R: Read + Seek>(reader: &mut R, header: &BoxHeader) -> Result<SampleEntry> {
    let entry_end = header.offset + header.size;

    // Skip reserved (6 bytes) + data reference index (2 bytes)
    reader.seek(SeekFrom::Current(8))?;

    // Video-specific fields
    let _version = reader.read_u16::<BigEndian>()?;
    let _revision = reader.read_u16::<BigEndian>()?;
    let _vendor = reader.read_u32::<BigEndian>()?;
    let _temporal_quality = reader.read_u32::<BigEndian>()?;
    let _spatial_quality = reader.read_u32::<BigEndian>()?;

    let width = reader.read_u16::<BigEndian>()?;
    let height = reader.read_u16::<BigEndian>()?;

    let _h_resolution = reader.read_u32::<BigEndian>()?;
    let _v_resolution = reader.read_u32::<BigEndian>()?;
    let _data_size = reader.read_u32::<BigEndian>()?;
    let _frame_count = reader.read_u16::<BigEndian>()?;

    // Compressor name (32 bytes)
    reader.seek(SeekFrom::Current(32))?;

    let _depth = reader.read_u16::<BigEndian>()?;
    let _color_table = reader.read_i16::<BigEndian>()?;

    // Look for avcC box
    let mut extra_data = Vec::new();

    while reader.stream_position()? < entry_end {
        let child = read_box_header(reader)?;
        if child.box_type == AVCC {
            extra_data = read_box_content(reader, child.content_size() as usize)?;
            break;
        }
        skip_box(reader, &child)?;
    }

    Ok(SampleEntry::Video(VideoSampleEntry {
        codec: "h264".to_string(),
        width,
        height,
        extra_data,
    }))
}

/// Parses vp09 (VP9) sample entry
fn parse_vp09_entry<R: Read + Seek>(reader: &mut R, header: &BoxHeader) -> Result<SampleEntry> {
    let entry_end = header.offset + header.size;

    // Skip reserved (6 bytes) + data reference index (2 bytes)
    reader.seek(SeekFrom::Current(8))?;

    // Video-specific fields (same as avc1)
    reader.seek(SeekFrom::Current(16))?; // version, revision, vendor, temporal/spatial quality

    let width = reader.read_u16::<BigEndian>()?;
    let height = reader.read_u16::<BigEndian>()?;

    // Skip rest of video sample entry
    reader.seek(SeekFrom::Current(50))?;

    // Look for vpcC box
    let mut extra_data = Vec::new();

    while reader.stream_position()? < entry_end {
        let child = read_box_header(reader)?;
        if child.box_type == VPCC {
            extra_data = read_box_content(reader, child.content_size() as usize)?;
            break;
        }
        skip_box(reader, &child)?;
    }

    Ok(SampleEntry::Video(VideoSampleEntry {
        codec: "vp9".to_string(),
        width,
        height,
        extra_data,
    }))
}

/// Parses Opus sample entry
fn parse_opus_entry<R: Read + Seek>(reader: &mut R, header: &BoxHeader) -> Result<SampleEntry> {
    let entry_end = header.offset + header.size;

    // Skip reserved (6 bytes) + data reference index (2 bytes)
    reader.seek(SeekFrom::Current(8))?;

    // Audio-specific fields
    reader.seek(SeekFrom::Current(8))?; // version, revision, vendor

    let channels = reader.read_u16::<BigEndian>()?;
    let sample_size = reader.read_u16::<BigEndian>()?;

    reader.seek(SeekFrom::Current(4))?; // compression_id, packet_size

    // Sample rate is 16.16 fixed point
    let sample_rate_fixed = reader.read_u32::<BigEndian>()?;
    let sample_rate = sample_rate_fixed >> 16;

    // Look for dOps box
    let mut extra_data = Vec::new();

    while reader.stream_position()? < entry_end {
        let child = read_box_header(reader)?;
        if child.box_type == DOPS {
            extra_data = read_box_content(reader, child.content_size() as usize)?;
            break;
        }
        skip_box(reader, &child)?;
    }

    Ok(SampleEntry::Audio(AudioSampleEntry {
        codec: "opus".to_string(),
        channels,
        sample_size,
        sample_rate,
        extra_data,
    }))
}

/// Time-to-sample entry
#[derive(Debug, Clone, Copy)]
pub struct SttsEntry {
    pub sample_count: u32,
    pub sample_delta: u32,
}

/// Parses stts (time-to-sample) box
pub fn parse_stts<R: Read>(reader: &mut R) -> Result<Vec<SttsEntry>> {
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut entries = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        let sample_count = reader.read_u32::<BigEndian>()?;
        let sample_delta = reader.read_u32::<BigEndian>()?;
        entries.push(SttsEntry {
            sample_count,
            sample_delta,
        });
    }

    Ok(entries)
}

/// Sample-to-chunk entry
#[derive(Debug, Clone, Copy)]
pub struct StscEntry {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

/// Parses stsc (sample-to-chunk) box
pub fn parse_stsc<R: Read>(reader: &mut R) -> Result<Vec<StscEntry>> {
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut entries = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        let first_chunk = reader.read_u32::<BigEndian>()?;
        let samples_per_chunk = reader.read_u32::<BigEndian>()?;
        let sample_description_index = reader.read_u32::<BigEndian>()?;
        entries.push(StscEntry {
            first_chunk,
            samples_per_chunk,
            sample_description_index,
        });
    }

    Ok(entries)
}

/// Parses stsz (sample size) box
pub fn parse_stsz<R: Read>(reader: &mut R) -> Result<(u32, Vec<u32>)> {
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let sample_size = reader.read_u32::<BigEndian>()?;
    let sample_count = reader.read_u32::<BigEndian>()?;

    let sizes = if sample_size == 0 {
        // Variable size - read individual sizes
        let mut sizes = Vec::with_capacity(sample_count as usize);
        for _ in 0..sample_count {
            sizes.push(reader.read_u32::<BigEndian>()?);
        }
        sizes
    } else {
        // Constant size - create vector with same value
        vec![sample_size; sample_count as usize]
    };

    Ok((sample_size, sizes))
}

/// Parses stco (chunk offset) box - 32-bit offsets
pub fn parse_stco<R: Read>(reader: &mut R) -> Result<Vec<u64>> {
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut offsets = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        offsets.push(reader.read_u32::<BigEndian>()? as u64);
    }

    Ok(offsets)
}

/// Parses co64 (chunk offset) box - 64-bit offsets
pub fn parse_co64<R: Read>(reader: &mut R) -> Result<Vec<u64>> {
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut offsets = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        offsets.push(reader.read_u64::<BigEndian>()?);
    }

    Ok(offsets)
}

/// Parses stss (sync sample / keyframe) box
pub fn parse_stss<R: Read>(reader: &mut R) -> Result<Vec<u32>> {
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut samples = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        samples.push(reader.read_u32::<BigEndian>()?);
    }

    Ok(samples)
}

/// Composition time offset entry
#[derive(Debug, Clone, Copy)]
pub struct CttsEntry {
    pub sample_count: u32,
    pub sample_offset: i32,
}

/// Parses ctts (composition time to sample) box
pub fn parse_ctts<R: Read>(reader: &mut R) -> Result<Vec<CttsEntry>> {
    let version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let entry_count = reader.read_u32::<BigEndian>()?;
    let mut entries = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        let sample_count = reader.read_u32::<BigEndian>()?;
        let sample_offset = if version == 0 {
            reader.read_u32::<BigEndian>()? as i32
        } else {
            reader.read_i32::<BigEndian>()?
        };
        entries.push(CttsEntry {
            sample_count,
            sample_offset,
        });
    }

    Ok(entries)
}

/// Parses mdhd (media header) box
pub fn parse_mdhd<R: Read>(reader: &mut R) -> Result<(u32, u64)> {
    let version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let (timescale, duration) = if version == 1 {
        let _creation_time = reader.read_u64::<BigEndian>()?;
        let _modification_time = reader.read_u64::<BigEndian>()?;
        let timescale = reader.read_u32::<BigEndian>()?;
        let duration = reader.read_u64::<BigEndian>()?;
        (timescale, duration)
    } else {
        let _creation_time = reader.read_u32::<BigEndian>()?;
        let _modification_time = reader.read_u32::<BigEndian>()?;
        let timescale = reader.read_u32::<BigEndian>()?;
        let duration = reader.read_u32::<BigEndian>()? as u64;
        (timescale, duration)
    };

    Ok((timescale, duration))
}

/// Parses hdlr (handler) box to get handler type
pub fn parse_hdlr<R: Read>(reader: &mut R, size: u64) -> Result<u32> {
    let _version = reader.read_u8()?;
    let mut flags = [0u8; 3];
    reader.read_exact(&mut flags)?;

    let _pre_defined = reader.read_u32::<BigEndian>()?;
    let handler_type = reader.read_u32::<BigEndian>()?;

    // Skip rest of the box
    let remaining = size.saturating_sub(12);
    if remaining > 0 {
        let mut skip = vec![0u8; remaining as usize];
        reader.read_exact(&mut skip)?;
    }

    Ok(handler_type)
}
