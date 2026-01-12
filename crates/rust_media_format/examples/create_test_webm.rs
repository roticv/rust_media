//! Helper to create a test WebM file with Opus audio
//!
//! This creates a minimal valid WebM file containing Opus audio for testing.

use byteorder::{BigEndian, WriteBytesExt};
use std::fs::File;
use std::io::{Write, Result};

fn write_vint(writer: &mut dyn Write, value: u64, length: usize) -> Result<()> {
    if length == 0 || length > 8 {
        panic!("Invalid VINT length");
    }

    // Set the length marker bit
    let marker_bit = 0x80u8 >> (length - 1);
    let mut bytes = vec![0u8; length];

    // Write the value in big-endian
    let mut remaining = value;
    for i in (0..length).rev() {
        bytes[i] = (remaining & 0xFF) as u8;
        remaining >>= 8;
    }

    // Set the marker bit in the first byte
    bytes[0] |= marker_bit;

    writer.write_all(&bytes)
}

fn write_element_header(writer: &mut dyn Write, id: u64, size: u64) -> Result<()> {
    // Determine VINT length for ID based on actual value
    let id_length = if id <= 0x7F { 1 }
        else if id <= 0x3FFF { 2 }
        else if id <= 0x1FFFFF { 3 }
        else if id <= 0x0FFFFFFF { 4 }
        else if id <= 0x07FFFFFFFF { 5 }
        else { panic!("ID too large: 0x{:X}", id) };

    write_vint(writer, id, id_length)?;

    // Size is also a VINT
    let size_length = if size < 0x7F { 1 }
        else if size < 0x3FFF { 2 }
        else if size < 0x1FFFFF { 3 }
        else if size < 0xFFFFFFF { 4 }
        else { panic!("Size too large") };

    write_vint(writer, size, size_length)
}

fn write_uint_element(writer: &mut dyn Write, id: u64, value: u64) -> Result<()> {
    let mut bytes = Vec::new();
    let mut val = value;

    // Encode as minimal bytes
    if val == 0 {
        bytes.push(0);
    } else {
        while val > 0 {
            bytes.insert(0, (val & 0xFF) as u8);
            val >>= 8;
        }
    }

    write_element_header(writer, id, bytes.len() as u64)?;
    writer.write_all(&bytes)
}

fn write_float_element(writer: &mut dyn Write, id: u64, value: f64) -> Result<()> {
    write_element_header(writer, id, 8)?;
    writer.write_f64::<BigEndian>(value)
}

fn write_string_element(writer: &mut dyn Write, id: u64, value: &str) -> Result<()> {
    write_element_header(writer, id, value.len() as u64)?;
    writer.write_all(value.as_bytes())
}

fn main() -> Result<()> {
    println!("Creating test WebM file with Opus audio...\n");

    let mut file = File::create("test_opus.webm")?;

    // EBML Header
    println!("Writing EBML header...");
    let ebml_header_content = {
        let mut buf = Vec::new();
        write_uint_element(&mut buf, 0x4286, 1)?; // EBMLVersion
        write_uint_element(&mut buf, 0x42F7, 1)?; // EBMLReadVersion
        write_uint_element(&mut buf, 0x42F2, 4)?; // EBMLMaxIDLength
        write_uint_element(&mut buf, 0x42F3, 8)?; // EBMLMaxSizeLength
        write_string_element(&mut buf, 0x4282, "webm")?; // DocType
        write_uint_element(&mut buf, 0x4287, 2)?; // DocTypeVersion
        write_uint_element(&mut buf, 0x4285, 2)?; // DocTypeReadVersion
        buf
    };

    write_element_header(&mut file, 0x1A45DFA3, ebml_header_content.len() as u64)?;
    file.write_all(&ebml_header_content)?;

    // Segment
    println!("Writing Segment...");
    // Use unknown size (all 1s in VINT)
    file.write_all(&[0x18, 0x53, 0x80, 0x67])?; // Segment ID
    file.write_all(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])?; // Unknown size

    // Segment Info
    println!("Writing Info...");
    let info_content = {
        let mut buf = Vec::new();
        write_uint_element(&mut buf, 0x2AD7B1, 1000000)?; // TimecodeScale (1ms)
        write_float_element(&mut buf, 0x4489, 1000.0)?; // Duration (1 second)
        write_string_element(&mut buf, 0x4D80, "rust_media")?; // MuxingApp
        write_string_element(&mut buf, 0x5741, "rust_media")?; // WritingApp
        buf
    };

    write_element_header(&mut file, 0x1549A966, info_content.len() as u64)?;
    file.write_all(&info_content)?;

    // Tracks
    println!("Writing Tracks...");
    let track_entry = {
        let mut buf = Vec::new();
        write_uint_element(&mut buf, 0xD7, 1)?; // TrackNumber
        write_uint_element(&mut buf, 0x73C5, 1)?; // TrackUID
        write_uint_element(&mut buf, 0x83, 2)?; // TrackType (Audio = 2)
        write_string_element(&mut buf, 0x86, "A_OPUS")?; // CodecID

        // Audio settings
        let audio_settings = {
            let mut audio_buf = Vec::new();
            write_float_element(&mut audio_buf, 0xB5, 48000.0)?; // SamplingFrequency
            write_uint_element(&mut audio_buf, 0x9F, 2)?; // Channels
            audio_buf
        };
        write_element_header(&mut buf, 0xE1, audio_settings.len() as u64)?;
        buf.write_all(&audio_settings)?;

        buf
    };

    let tracks_content = {
        let mut buf = Vec::new();
        write_element_header(&mut buf, 0xAE, track_entry.len() as u64)?;
        buf.write_all(&track_entry)?;
        buf
    };

    write_element_header(&mut file, 0x1654AE6B, tracks_content.len() as u64)?;
    file.write_all(&tracks_content)?;

    // Cluster with dummy Opus packet
    println!("Writing Cluster with sample data...");
    let cluster_content = {
        let mut buf = Vec::new();
        write_uint_element(&mut buf, 0xE7, 0)?; // Timecode

        // SimpleBlock with dummy Opus data
        let block_data = {
            let mut block_buf = Vec::new();
            // Track number (VINT)
            block_buf.write_all(&[0x81])?; // Track 1
            // Timecode (int16 BE)
            block_buf.write_all(&[0x00, 0x00])?;
            // Flags
            block_buf.write_all(&[0x80])?; // Keyframe
            // Dummy Opus packet (minimal valid packet)
            block_buf.write_all(&[0xFC, 0xFF, 0xFE])?; // TOC byte + dummy payload
            block_buf
        };

        write_element_header(&mut buf, 0xA3, block_data.len() as u64)?;
        buf.write_all(&block_data)?;
        buf
    };

    write_element_header(&mut file, 0x1F43B675, cluster_content.len() as u64)?;
    file.write_all(&cluster_content)?;

    println!("\n✓ Created test_opus.webm");
    println!("  This is a minimal WebM file with Opus audio.");
    println!("  Note: The Opus packet is dummy data for structure testing only.");
    println!("\nFor real testing, use ffmpeg to create a proper WebM file:");
    println!("  ffmpeg -f lavfi -i \"sine=frequency=440:duration=1\" -c:a libopus test.webm");

    Ok(())
}
