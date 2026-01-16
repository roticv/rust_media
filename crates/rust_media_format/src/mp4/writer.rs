//! MP4 box writing utilities
//!
//! Provides functions for writing MP4 box structures including:
//! - Box headers (size + type)
//! - Full box headers (version + flags)
//! - Fixed-point numbers
//! - Big-endian integers

use rust_media_core::Result;
use std::io::{Seek, SeekFrom, Write};

/// Writes a standard box header (8 bytes: size + type)
///
/// Returns the position where the size field was written, allowing
/// later updates via `update_box_size()`.
pub fn write_box_header<W: Write>(writer: &mut W, box_type: u32, size: u32) -> Result<()> {
    write_u32(writer, size)?;
    write_u32(writer, box_type)?;
    Ok(())
}

/// Writes a box header with placeholder size (0)
///
/// Returns the position where the size field was written.
/// Use `update_box_size()` to fill in the actual size later.
pub fn write_box_header_placeholder<W: Write + Seek>(
    writer: &mut W,
    box_type: u32,
) -> Result<u64> {
    let pos = writer.stream_position()?;
    write_u32(writer, 0)?; // Placeholder size
    write_u32(writer, box_type)?;
    Ok(pos)
}

/// Writes an extended box header for boxes larger than 4GB
///
/// Uses size=1 to indicate 64-bit extended size follows the type.
pub fn write_box_header_64<W: Write>(writer: &mut W, box_type: u32, size: u64) -> Result<()> {
    write_u32(writer, 1)?; // size=1 indicates 64-bit size follows
    write_u32(writer, box_type)?;
    write_u64(writer, size)?;
    Ok(())
}

/// Updates a previously written box size at the given position
pub fn update_box_size<W: Write + Seek>(writer: &mut W, pos: u64, size: u32) -> Result<()> {
    let current = writer.stream_position()?;
    writer.seek(SeekFrom::Start(pos))?;
    write_u32(writer, size)?;
    writer.seek(SeekFrom::Start(current))?;
    Ok(())
}

/// Updates a previously written 64-bit box size at the given position
pub fn update_box_size_64<W: Write + Seek>(writer: &mut W, pos: u64, size: u64) -> Result<()> {
    let current = writer.stream_position()?;
    writer.seek(SeekFrom::Start(pos + 8))?; // Skip past size=1 and type
    write_u64(writer, size)?;
    writer.seek(SeekFrom::Start(current))?;
    Ok(())
}

/// Writes a full box header (box header + version/flags)
///
/// Full boxes have a 1-byte version and 3-byte flags after the header.
pub fn write_full_box_header<W: Write + Seek>(
    writer: &mut W,
    box_type: u32,
    version: u8,
    flags: u32,
) -> Result<u64> {
    let pos = write_box_header_placeholder(writer, box_type)?;
    writer.write_all(&[version])?;
    // Write 3-byte flags (big-endian, masking off high byte)
    writer.write_all(&[(flags >> 16) as u8, (flags >> 8) as u8, flags as u8])?;
    Ok(pos)
}

/// Writes a big-endian u8
pub fn write_u8<W: Write>(writer: &mut W, value: u8) -> Result<()> {
    writer.write_all(&[value])?;
    Ok(())
}

/// Writes a big-endian i16
pub fn write_i16<W: Write>(writer: &mut W, value: i16) -> Result<()> {
    writer.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Writes a big-endian u16
pub fn write_u16<W: Write>(writer: &mut W, value: u16) -> Result<()> {
    writer.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Writes a big-endian i32
pub fn write_i32<W: Write>(writer: &mut W, value: i32) -> Result<()> {
    writer.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Writes a big-endian u32
pub fn write_u32<W: Write>(writer: &mut W, value: u32) -> Result<()> {
    writer.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Writes a big-endian i64
pub fn write_i64<W: Write>(writer: &mut W, value: i64) -> Result<()> {
    writer.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Writes a big-endian u64
pub fn write_u64<W: Write>(writer: &mut W, value: u64) -> Result<()> {
    writer.write_all(&value.to_be_bytes())?;
    Ok(())
}

/// Writes a fixed-point 16.16 number (32-bit total)
///
/// Used for values like sample rate (stored as integer.fraction).
pub fn write_fixed_16_16<W: Write>(writer: &mut W, value: f64) -> Result<()> {
    let fixed = (value * 65536.0) as u32;
    write_u32(writer, fixed)
}

/// Writes a fixed-point 8.8 number (16-bit total)
pub fn write_fixed_8_8<W: Write>(writer: &mut W, value: f64) -> Result<()> {
    let fixed = (value * 256.0) as u16;
    write_u16(writer, fixed)
}

/// Writes a fixed-point 2.30 number (32-bit total)
///
/// Used for matrix values in tkhd/mvhd.
pub fn write_fixed_2_30<W: Write>(writer: &mut W, value: f64) -> Result<()> {
    let fixed = (value * (1u64 << 30) as f64) as u32;
    write_u32(writer, fixed)
}

/// Writes zeros as padding
pub fn write_zeros<W: Write>(writer: &mut W, count: usize) -> Result<()> {
    for _ in 0..count {
        writer.write_all(&[0])?;
    }
    Ok(())
}

/// Writes a null-terminated string with padding to fixed length
pub fn write_string_fixed<W: Write>(writer: &mut W, s: &str, len: usize) -> Result<()> {
    let bytes = s.as_bytes();
    let write_len = bytes.len().min(len - 1); // Leave room for null terminator
    writer.write_all(&bytes[..write_len])?;
    // Pad with zeros
    write_zeros(writer, len - write_len)?;
    Ok(())
}

/// Writes a Pascal-style string (length byte + string)
pub fn write_pascal_string<W: Write>(writer: &mut W, s: &str) -> Result<()> {
    let bytes = s.as_bytes();
    let len = bytes.len().min(255) as u8;
    write_u8(writer, len)?;
    writer.write_all(&bytes[..len as usize])?;
    Ok(())
}

/// Writes a 3x3 transformation matrix (used in tkhd and mvhd)
///
/// Matrix values are stored as:
/// - a, b, u (row 1)
/// - c, d, v (row 2)
/// - x, y, w (row 3)
///
/// a, b, c, d, x, y are 16.16 fixed-point
/// u, v, w are 2.30 fixed-point
pub fn write_identity_matrix<W: Write>(writer: &mut W) -> Result<()> {
    // Identity matrix:
    // | 1.0  0.0  0.0 |
    // | 0.0  1.0  0.0 |
    // | 0.0  0.0  1.0 |
    write_fixed_16_16(writer, 1.0)?; // a
    write_fixed_16_16(writer, 0.0)?; // b
    write_fixed_2_30(writer, 0.0)?; // u
    write_fixed_16_16(writer, 0.0)?; // c
    write_fixed_16_16(writer, 1.0)?; // d
    write_fixed_2_30(writer, 0.0)?; // v
    write_fixed_16_16(writer, 0.0)?; // x
    write_fixed_16_16(writer, 0.0)?; // y
    write_fixed_2_30(writer, 1.0)?; // w
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_write_u32() {
        let mut buf = Vec::new();
        write_u32(&mut buf, 0x12345678).unwrap();
        assert_eq!(buf, vec![0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_write_u64() {
        let mut buf = Vec::new();
        write_u64(&mut buf, 0x123456789ABCDEF0).unwrap();
        assert_eq!(
            buf,
            vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]
        );
    }

    #[test]
    fn test_write_fixed_16_16() {
        let mut buf = Vec::new();
        write_fixed_16_16(&mut buf, 1.0).unwrap();
        assert_eq!(buf, vec![0x00, 0x01, 0x00, 0x00]); // 1.0 = 0x00010000

        let mut buf = Vec::new();
        write_fixed_16_16(&mut buf, 48000.0).unwrap();
        assert_eq!(buf, vec![0xBB, 0x80, 0x00, 0x00]); // 48000 << 16 in big-endian
    }

    #[test]
    fn test_write_box_header() {
        let mut buf = Vec::new();
        write_box_header(&mut buf, 0x66747970, 20).unwrap(); // "ftyp", size 20
        assert_eq!(
            buf,
            vec![
                0x00, 0x00, 0x00, 0x14, // size = 20
                0x66, 0x74, 0x79, 0x70, // type = "ftyp"
            ]
        );
    }

    #[test]
    fn test_write_box_header_placeholder() {
        let mut cursor = Cursor::new(Vec::new());
        let pos = write_box_header_placeholder(&mut cursor, 0x6D6F6F76).unwrap(); // "moov"
        assert_eq!(pos, 0);
        let buf = cursor.into_inner();
        assert_eq!(
            buf,
            vec![
                0x00, 0x00, 0x00, 0x00, // size = 0 (placeholder)
                0x6D, 0x6F, 0x6F, 0x76, // type = "moov"
            ]
        );
    }

    #[test]
    fn test_update_box_size() {
        let mut cursor = Cursor::new(vec![0u8; 16]);
        write_box_header_placeholder(&mut cursor, 0x6D6F6F76).unwrap();
        // Simulate writing 100 bytes of content
        cursor.seek(SeekFrom::Start(108)).unwrap();
        update_box_size(&mut cursor, 0, 108).unwrap();

        let buf = cursor.into_inner();
        assert_eq!(&buf[0..4], &[0x00, 0x00, 0x00, 0x6C]); // size = 108
    }

    #[test]
    fn test_write_identity_matrix() {
        let mut buf = Vec::new();
        write_identity_matrix(&mut buf).unwrap();
        assert_eq!(buf.len(), 36); // 9 * 4 bytes
    }

    #[test]
    fn test_write_string_fixed() {
        let mut buf = Vec::new();
        write_string_fixed(&mut buf, "test", 8).unwrap();
        assert_eq!(buf, vec![b't', b'e', b's', b't', 0, 0, 0, 0]);
    }
}
