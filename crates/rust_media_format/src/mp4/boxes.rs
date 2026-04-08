//! MP4 box (atom) type constants
//!
//! MP4 files are composed of boxes (also called atoms), each identified by
//! a 4-byte type code (fourcc). This module defines constants for all
//! box types used by the muxer.

/// Convert a 4-character string to a u32 box type
pub const fn fourcc(chars: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*chars)
}

// File-level boxes
/// File type box - identifies file brand and compatibility
pub const FTYP: u32 = fourcc(b"ftyp");
/// Movie box - contains all metadata
pub const MOOV: u32 = fourcc(b"moov");
/// Media data box - contains actual media samples
pub const MDAT: u32 = fourcc(b"mdat");
/// Free space box
pub const FREE: u32 = fourcc(b"free");

// Movie-level boxes (inside moov)
/// Movie header box - overall movie information
pub const MVHD: u32 = fourcc(b"mvhd");
/// Track box - container for a single track
pub const TRAK: u32 = fourcc(b"trak");

// Track-level boxes (inside trak)
/// Track header box - track characteristics
pub const TKHD: u32 = fourcc(b"tkhd");
/// Edit box - edit list container
pub const EDTS: u32 = fourcc(b"edts");
/// Edit list box
pub const ELST: u32 = fourcc(b"elst");
/// Media box - media information container
pub const MDIA: u32 = fourcc(b"mdia");

// Media-level boxes (inside mdia)
/// Media header box - media-specific header
pub const MDHD: u32 = fourcc(b"mdhd");
/// Handler reference box - media handler type
pub const HDLR: u32 = fourcc(b"hdlr");
/// Media information box - media info container
pub const MINF: u32 = fourcc(b"minf");

// Media information boxes (inside minf)
/// Video media header box
pub const VMHD: u32 = fourcc(b"vmhd");
/// Sound media header box
pub const SMHD: u32 = fourcc(b"smhd");
/// Data information box - data reference container
pub const DINF: u32 = fourcc(b"dinf");
/// Data reference box - data location references
pub const DREF: u32 = fourcc(b"dref");
/// URL data entry
pub const URL: u32 = fourcc(b"url ");
/// Sample table box - sample information container
pub const STBL: u32 = fourcc(b"stbl");

// Sample table boxes (inside stbl)
/// Sample description box - codec configuration
pub const STSD: u32 = fourcc(b"stsd");
/// Time-to-sample box - sample timing
pub const STTS: u32 = fourcc(b"stts");
/// Composition time-to-sample box - presentation time offsets
pub const CTTS: u32 = fourcc(b"ctts");
/// Sample-to-chunk box - sample grouping
pub const STSC: u32 = fourcc(b"stsc");
/// Sample size box - individual sample sizes
pub const STSZ: u32 = fourcc(b"stsz");
/// Chunk offset box (32-bit)
pub const STCO: u32 = fourcc(b"stco");
/// Chunk offset box (64-bit)
pub const CO64: u32 = fourcc(b"co64");
/// Sync sample box - keyframe indices
pub const STSS: u32 = fourcc(b"stss");

// Video codec boxes
/// AVC (H.264) sample entry
pub const AVC1: u32 = fourcc(b"avc1");
/// AVC decoder configuration
pub const AVCC: u32 = fourcc(b"avcC");
/// VP9 sample entry
pub const VP09: u32 = fourcc(b"vp09");
/// VP9 codec configuration
pub const VPCC: u32 = fourcc(b"vpcC");
/// AV1 sample entry
pub const AV01: u32 = fourcc(b"av01");
/// AV1 codec configuration
pub const AV1C: u32 = fourcc(b"av1C");
/// HEVC (H.265) sample entry
pub const HVC1: u32 = fourcc(b"hvc1");
/// HEVC (H.265) sample entry (alternative)
pub const HEV1: u32 = fourcc(b"hev1");
/// HEVC decoder configuration
pub const HVCC: u32 = fourcc(b"hvcC");

// Audio codec boxes
/// AAC audio sample entry
pub const MP4A: u32 = fourcc(b"mp4a");
/// Elementary stream descriptor
pub const ESDS: u32 = fourcc(b"esds");
/// Opus audio sample entry
pub const OPUS: u32 = fourcc(b"Opus");
/// Opus specific box
pub const DOPS: u32 = fourcc(b"dOps");
/// MP3 audio sample entry
pub const DOT_MP3: u32 = fourcc(b".mp3");

// Handler types
/// Video handler type
pub const HANDLER_VIDEO: u32 = fourcc(b"vide");
/// Sound handler type
pub const HANDLER_SOUND: u32 = fourcc(b"soun");

// Brand identifiers for ftyp box
/// ISO base media file format
pub const BRAND_ISOM: [u8; 4] = *b"isom";
/// ISO base media file format version 2
pub const BRAND_ISO2: [u8; 4] = *b"iso2";
/// MP4 version 1
pub const BRAND_MP41: [u8; 4] = *b"mp41";
/// MP4 version 2
pub const BRAND_MP42: [u8; 4] = *b"mp42";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fourcc() {
        assert_eq!(FTYP, 0x66747970); // "ftyp" in big-endian
        assert_eq!(MOOV, 0x6D6F6F76); // "moov" in big-endian
        assert_eq!(MDAT, 0x6D646174); // "mdat" in big-endian
    }

    #[test]
    fn test_brand_bytes() {
        assert_eq!(&BRAND_ISOM, b"isom");
        assert_eq!(&BRAND_MP41, b"mp41");
    }
}
