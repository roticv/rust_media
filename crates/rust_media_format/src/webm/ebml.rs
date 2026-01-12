//! EBML (Extensible Binary Meta Language) parser
//!
//! EBML is the binary format used by Matroska/WebM containers.

use byteorder::{BigEndian, ReadBytesExt};
use rust_media_core::{Error, Result};
use std::io::Read;

/// EBML Element IDs (Matroska/WebM)
#[allow(dead_code)]
pub mod element_id {
    // EBML Header
    pub const EBML: u64 = 0x1A45DFA3;
    pub const EBML_VERSION: u64 = 0x4286;
    pub const EBML_READ_VERSION: u64 = 0x42F7;
    pub const EBML_MAX_ID_LENGTH: u64 = 0x42F2;
    pub const EBML_MAX_SIZE_LENGTH: u64 = 0x42F3;
    pub const DOC_TYPE: u64 = 0x4282;
    pub const DOC_TYPE_VERSION: u64 = 0x4287;
    pub const DOC_TYPE_READ_VERSION: u64 = 0x4285;

    // Segment
    pub const SEGMENT: u64 = 0x18538067;
    pub const SEEK_HEAD: u64 = 0x114D9B74;
    pub const INFO: u64 = 0x1549A966;
    pub const TRACKS: u64 = 0x1654AE6B;
    pub const CLUSTER: u64 = 0x1F43B675;
    pub const CUES: u64 = 0x1C53BB6B;

    // Segment Info
    pub const TIMECODE_SCALE: u64 = 0x2AD7B1;
    pub const DURATION: u64 = 0x4489;
    pub const MUXING_APP: u64 = 0x4D80;
    pub const WRITING_APP: u64 = 0x5741;

    // Track
    pub const TRACK_ENTRY: u64 = 0xAE;
    pub const TRACK_NUMBER: u64 = 0xD7;
    pub const TRACK_UID: u64 = 0x73C5;
    pub const TRACK_TYPE: u64 = 0x83;
    pub const CODEC_ID: u64 = 0x86;
    pub const CODEC_PRIVATE: u64 = 0x63A2;
    pub const CODEC_DELAY: u64 = 0x56AA;
    pub const SEEK_PRE_ROLL: u64 = 0x56BB;

    // Audio
    pub const AUDIO: u64 = 0xE1;
    pub const SAMPLING_FREQUENCY: u64 = 0xB5;
    pub const CHANNELS: u64 = 0x9F;
    pub const BIT_DEPTH: u64 = 0x6264;

    // Cluster
    pub const TIMECODE: u64 = 0xE7;
    pub const SIMPLE_BLOCK: u64 = 0xA3;
    pub const BLOCK_GROUP: u64 = 0xA0;
    pub const BLOCK: u64 = 0xA1;
    pub const BLOCK_DURATION: u64 = 0x9B;
}

/// Reads a variable-length integer (VINT) from the reader for Element IDs
///
/// For element IDs, the length marker bit is INCLUDED in the value
pub fn read_vint<R: Read>(reader: &mut R) -> Result<u64> {
    let first_byte = reader.read_u8()?;

    // Find the length marker (first 1 bit)
    let mut mask = 0x80u8;
    let mut length = 0usize;

    for i in 0..8 {
        if (first_byte & mask) != 0 {
            length = i + 1;
            break;
        }
        mask >>= 1;
    }

    if length == 0 {
        return Err(Error::InvalidData("Invalid VINT".to_string()));
    }

    // For element IDs, we keep the marker bit as part of the value
    let mut value = first_byte as u64;

    // Read remaining bytes
    for _ in 1..length {
        let byte = reader.read_u8()?;
        value = (value << 8) | (byte as u64);
    }

    Ok(value)
}

/// Reads a variable-length size (data size in EBML)
pub fn read_vint_size<R: Read>(reader: &mut R) -> Result<Option<u64>> {
    let first_byte = reader.read_u8()?;

    // Find the length marker
    let mut mask = 0x80u8;
    let mut length = 0usize;

    for i in 0..8 {
        if (first_byte & mask) != 0 {
            length = i + 1;
            break;
        }
        mask >>= 1;
    }

    if length == 0 {
        return Err(Error::InvalidData("Invalid VINT size".to_string()));
    }

    // Remove the length marker
    let mut value = (first_byte & (mask - 1)) as u64;

    // Read remaining bytes
    for _ in 1..length {
        value = (value << 8) | (reader.read_u8()? as u64);
    }

    // Check for unknown size (all data bits are 1)
    let unknown_marker = (1u64 << (7 * length)) - 1;
    if value == unknown_marker {
        return Ok(None); // Unknown size
    }

    Ok(Some(value))
}

/// Represents an EBML element
#[derive(Debug, Clone)]
pub struct Element {
    pub id: u64,
    pub size: Option<u64>,
    pub position: u64,
}

impl Element {
    /// Reads an EBML element header
    pub fn read<R: Read + std::io::Seek>(reader: &mut R) -> Result<Self> {
        let position = reader.stream_position()?;
        let id = read_vint(reader)?;
        let size = read_vint_size(reader)?;

        Ok(Element {
            id,
            size,
            position,
        })
    }

    /// Returns the position where the element's data starts
    pub fn data_position<R: Read + std::io::Seek>(&self, reader: &mut R) -> Result<u64> {
        Ok(reader.stream_position()?)
    }

    /// Skips this element's data
    pub fn skip<R: Read + std::io::Seek>(&self, reader: &mut R) -> Result<()> {
        if let Some(size) = self.size {
            reader.seek(std::io::SeekFrom::Current(size as i64))?;
        }
        Ok(())
    }

    /// Reads the element data as bytes
    pub fn read_data<R: Read>(&self, reader: &mut R) -> Result<Vec<u8>> {
        match self.size {
            Some(size) => {
                let mut data = vec![0u8; size as usize];
                reader.read_exact(&mut data)?;
                Ok(data)
            }
            None => Err(Error::InvalidData(
                "Cannot read element with unknown size".to_string(),
            )),
        }
    }

    /// Reads the element data as a UTF-8 string
    pub fn read_string<R: Read>(&self, reader: &mut R) -> Result<String> {
        let data = self.read_data(reader)?;
        String::from_utf8(data)
            .map_err(|e| Error::InvalidData(format!("Invalid UTF-8: {}", e)))
    }

    /// Reads the element data as an unsigned integer
    pub fn read_uint<R: Read>(&self, reader: &mut R) -> Result<u64> {
        match self.size {
            Some(0) => Ok(0),
            Some(size) if size <= 8 => {
                let mut value = 0u64;
                for _ in 0..size {
                    value = (value << 8) | (reader.read_u8()? as u64);
                }
                Ok(value)
            }
            Some(size) => Err(Error::InvalidData(format!(
                "Uint too large: {} bytes",
                size
            ))),
            None => Err(Error::InvalidData("Unknown size for uint".to_string())),
        }
    }

    /// Reads the element data as a float
    pub fn read_float<R: Read>(&self, reader: &mut R) -> Result<f64> {
        match self.size {
            Some(4) => Ok(reader.read_f32::<BigEndian>()? as f64),
            Some(8) => Ok(reader.read_f64::<BigEndian>()?),
            _ => Err(Error::InvalidData("Invalid float size".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_vint() {
        // Test single byte VINT (0x81 = 10000001)
        // With marker bit kept: 0x81
        let data = vec![0x81];
        let mut cursor = Cursor::new(data);
        assert_eq!(read_vint(&mut cursor).unwrap(), 0x81);

        // Test two byte VINT (0x4001 = 01000000 00000001)
        // With marker bit kept: 0x4001
        let data = vec![0x40, 0x01];
        let mut cursor = Cursor::new(data);
        assert_eq!(read_vint(&mut cursor).unwrap(), 0x4001);

        // Test EBML element ID: 0x1A45DFA3
        let data = vec![0x1A, 0x45, 0xDF, 0xA3];
        let mut cursor = Cursor::new(data);
        assert_eq!(read_vint(&mut cursor).unwrap(), 0x1A45DFA3);
    }

    #[test]
    fn test_read_element() {
        // Create a simple element: ID=0x81, size=1 byte (0x81)
        let data = vec![0x81, 0x81, 0x42]; // ID, size, data
        let mut cursor = Cursor::new(data);

        let element = Element::read(&mut cursor).unwrap();
        assert_eq!(element.id, 0x81); // ID keeps marker bit
        assert_eq!(element.size, Some(1)); // Size has marker bit removed

        let byte = cursor.read_u8().unwrap();
        assert_eq!(byte, 0x42);
    }

    #[test]
    fn test_read_vint_size() {
        // Test size VINT (marker bit removed)
        let data = vec![0x81]; // Size of 1
        let mut cursor = Cursor::new(data);
        assert_eq!(read_vint_size(&mut cursor).unwrap(), Some(1));

        // Test unknown size (all data bits = 1)
        let data = vec![0xFF]; // Unknown size
        let mut cursor = Cursor::new(data);
        assert_eq!(read_vint_size(&mut cursor).unwrap(), None);
    }
}
