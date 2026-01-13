//! EBML writer utilities for WebM muxing

use byteorder::{BigEndian, WriteBytesExt};
use rust_media_core::Result;
use std::io::Write;

/// Writes a variable-length integer (VINT) for element IDs
///
/// Element IDs keep the length marker bit as part of the value
pub fn write_element_id<W: Write>(writer: &mut W, id: u64) -> Result<()> {
    // Determine the number of bytes needed based on the ID value
    let bytes = if id <= 0xFF {
        vec![(id & 0xFF) as u8]
    } else if id <= 0xFFFF {
        vec![((id >> 8) & 0xFF) as u8, (id & 0xFF) as u8]
    } else if id <= 0xFFFFFF {
        vec![
            ((id >> 16) & 0xFF) as u8,
            ((id >> 8) & 0xFF) as u8,
            (id & 0xFF) as u8,
        ]
    } else {
        vec![
            ((id >> 24) & 0xFF) as u8,
            ((id >> 16) & 0xFF) as u8,
            ((id >> 8) & 0xFF) as u8,
            (id & 0xFF) as u8,
        ]
    };

    writer.write_all(&bytes)?;
    Ok(())
}

/// Writes a variable-length size (VINT size)
///
/// For sizes, the length marker bit is removed from the value
pub fn write_vint_size<W: Write>(writer: &mut W, size: u64) -> Result<()> {
    // Determine the number of bytes needed
    let (bytes, marker) = if size < 0x7F {
        // 1 byte: 0xxxxxxx (7 bits of data)
        (1, 0x80)
    } else if size < 0x3FFF {
        // 2 bytes: 01xxxxxx xxxxxxxx (14 bits of data)
        (2, 0x4000)
    } else if size < 0x1FFFFF {
        // 3 bytes: 001xxxxx xxxxxxxx xxxxxxxx (21 bits of data)
        (3, 0x200000)
    } else if size < 0x0FFFFFFF {
        // 4 bytes: 0001xxxx ... (28 bits of data)
        (4, 0x10000000)
    } else if size < 0x07FFFFFFFF {
        // 5 bytes (35 bits of data)
        (5, 0x0800000000)
    } else if size < 0x03FFFFFFFFFF {
        // 6 bytes (42 bits of data)
        (6, 0x040000000000)
    } else if size < 0x01FFFFFFFFFFFF {
        // 7 bytes (49 bits of data)
        (7, 0x02000000000000)
    } else {
        // 8 bytes (56 bits of data)
        (8, 0x0100000000000000)
    };

    let value = size | marker;

    // Write the bytes in big-endian order
    for i in (0..bytes).rev() {
        writer.write_u8(((value >> (i * 8)) & 0xFF) as u8)?;
    }

    Ok(())
}

/// Writes an unsigned integer element
pub fn write_uint_element<W: Write>(writer: &mut W, id: u64, value: u64) -> Result<()> {
    write_element_id(writer, id)?;

    // Determine minimal byte representation
    let bytes = if value == 0 {
        vec![0]
    } else if value <= 0xFF {
        vec![value as u8]
    } else if value <= 0xFFFF {
        vec![(value >> 8) as u8, value as u8]
    } else if value <= 0xFFFFFF {
        vec![(value >> 16) as u8, (value >> 8) as u8, value as u8]
    } else if value <= 0xFFFFFFFF {
        vec![
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ]
    } else if value <= 0xFFFFFFFFFF {
        vec![
            (value >> 32) as u8,
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ]
    } else if value <= 0xFFFFFFFFFFFF {
        vec![
            (value >> 40) as u8,
            (value >> 32) as u8,
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ]
    } else if value <= 0xFFFFFFFFFFFFFF {
        vec![
            (value >> 48) as u8,
            (value >> 40) as u8,
            (value >> 32) as u8,
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ]
    } else {
        vec![
            (value >> 56) as u8,
            (value >> 48) as u8,
            (value >> 40) as u8,
            (value >> 32) as u8,
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ]
    };

    write_vint_size(writer, bytes.len() as u64)?;
    writer.write_all(&bytes)?;
    Ok(())
}

/// Writes a float element
pub fn write_float_element<W: Write>(writer: &mut W, id: u64, value: f64) -> Result<()> {
    write_element_id(writer, id)?;
    write_vint_size(writer, 8)?; // Doubles are 8 bytes
    writer.write_f64::<BigEndian>(value)?;
    Ok(())
}

/// Writes a string element
pub fn write_string_element<W: Write>(writer: &mut W, id: u64, value: &str) -> Result<()> {
    write_element_id(writer, id)?;
    let bytes = value.as_bytes();
    write_vint_size(writer, bytes.len() as u64)?;
    writer.write_all(bytes)?;
    Ok(())
}

/// Writes a binary element
pub fn write_binary_element<W: Write>(writer: &mut W, id: u64, data: &[u8]) -> Result<()> {
    write_element_id(writer, id)?;
    write_vint_size(writer, data.len() as u64)?;
    writer.write_all(data)?;
    Ok(())
}

/// Writes a master element header (element that contains other elements)
pub fn write_master_header<W: Write>(writer: &mut W, id: u64, size: u64) -> Result<()> {
    write_element_id(writer, id)?;
    write_vint_size(writer, size)?;
    Ok(())
}

/// Writes a master element header with unknown size (for streaming)
pub fn write_master_header_unknown_size<W: Write>(writer: &mut W, id: u64) -> Result<()> {
    write_element_id(writer, id)?;
    // Unknown size: all data bits set to 1 (0x01FFFFFFFFFFFFFF for 8-byte VINT)
    writer.write_all(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_write_element_id() {
        let mut buf = Cursor::new(Vec::new());

        // 1-byte ID
        write_element_id(&mut buf, 0xAE).unwrap();
        assert_eq!(buf.get_ref(), &[0xAE]);

        // 2-byte ID
        buf = Cursor::new(Vec::new());
        write_element_id(&mut buf, 0x4286).unwrap();
        assert_eq!(buf.get_ref(), &[0x42, 0x86]);

        // 4-byte ID
        buf = Cursor::new(Vec::new());
        write_element_id(&mut buf, 0x1A45DFA3).unwrap();
        assert_eq!(buf.get_ref(), &[0x1A, 0x45, 0xDF, 0xA3]);
    }

    #[test]
    fn test_write_vint_size() {
        let mut buf = Cursor::new(Vec::new());

        // 1-byte size
        write_vint_size(&mut buf, 42).unwrap();
        assert_eq!(buf.get_ref(), &[0x80 | 42]);

        // 2-byte size
        buf = Cursor::new(Vec::new());
        write_vint_size(&mut buf, 200).unwrap();
        assert_eq!(buf.get_ref(), &[0x40, 200]);

        // 3-byte size (0x100000 < 0x1FFFFF, so fits in 3 bytes)
        buf = Cursor::new(Vec::new());
        write_vint_size(&mut buf, 0x100000).unwrap();
        // 0x100000 with 3-byte marker: 0x200000 | 0x100000 = 0x300000
        assert_eq!(buf.get_ref(), &[0x30, 0x00, 0x00]);
    }

    #[test]
    fn test_write_uint_element() {
        let mut buf = Cursor::new(Vec::new());
        write_uint_element(&mut buf, 0xD7, 1).unwrap();

        // Should write: ID (0xD7), size (0x81 = 1 byte), value (1)
        assert_eq!(buf.get_ref(), &[0xD7, 0x81, 0x01]);
    }

    #[test]
    fn test_write_string_element() {
        let mut buf = Cursor::new(Vec::new());
        write_string_element(&mut buf, 0x4282, "webm").unwrap();

        // Should write: ID (0x4282), size (0x84 = 4 bytes), "webm"
        assert_eq!(buf.get_ref(), &[0x42, 0x82, 0x84, b'w', b'e', b'b', b'm']);
    }

    #[test]
    fn test_write_binary_element() {
        let mut buf = Cursor::new(Vec::new());
        write_binary_element(&mut buf, 0x63A2, &[0x01, 0x02, 0x03]).unwrap();

        // Should write: ID (0x63A2), size (0x83 = 3 bytes), data
        assert_eq!(buf.get_ref(), &[0x63, 0xA2, 0x83, 0x01, 0x02, 0x03]);
    }
}
