//! Container format detection from magic bytes and file extensions
//!
//! Supports detecting format by:
//! - **Magic bytes**: Reads the first few bytes of the file to identify the container
//! - **File extension**: Falls back to extension-based detection
//! - **Combined**: Tries magic bytes first, then extension

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Known container formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerFormat {
    /// MP4 / MOV / M4A / M4V (ISO Base Media File Format)
    Mp4,
    /// WebM (Matroska subset for VP8/VP9/AV1 + Opus/Vorbis)
    WebM,
    /// WAV (RIFF WAVE)
    Wav,
}

impl ContainerFormat {
    /// Common file extensions for this format
    pub fn extensions(&self) -> &[&str] {
        match self {
            ContainerFormat::Mp4 => &["mp4", "m4a", "m4v", "mov"],
            ContainerFormat::WebM => &["webm", "mkv"],
            ContainerFormat::Wav => &["wav"],
        }
    }

    /// Format name for display
    pub fn name(&self) -> &str {
        match self {
            ContainerFormat::Mp4 => "mp4",
            ContainerFormat::WebM => "webm",
            ContainerFormat::Wav => "wav",
        }
    }
}

/// Detect container format by reading magic bytes from the start of the stream.
///
/// Reads up to 12 bytes, then seeks back to the original position.
/// Returns `None` if the format is not recognized.
pub fn detect_format<R: Read + Seek>(reader: &mut R) -> Option<ContainerFormat> {
    let start = reader.stream_position().ok()?;
    let mut buf = [0u8; 12];
    let bytes_read = reader.read(&mut buf).ok()?;
    reader.seek(SeekFrom::Start(start)).ok()?;

    if bytes_read < 4 {
        return None;
    }

    // WAV: starts with "RIFF" ... "WAVE"
    if bytes_read >= 12 && &buf[0..4] == b"RIFF" && &buf[8..12] == b"WAVE" {
        return Some(ContainerFormat::Wav);
    }

    // WebM/MKV: starts with EBML header 0x1A 0x45 0xDF 0xA3
    if buf[0..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return Some(ContainerFormat::WebM);
    }

    // MP4/MOV: has "ftyp" at offset 4 (first box type), or starts with wide/mdat/moov/free
    if bytes_read >= 8 {
        let box_type = &buf[4..8];
        if box_type == b"ftyp"
            || box_type == b"moov"
            || box_type == b"mdat"
            || box_type == b"free"
            || box_type == b"wide"
            || box_type == b"skip"
            || box_type == b"pnot"
        {
            return Some(ContainerFormat::Mp4);
        }
    }

    None
}

/// Detect container format from a file extension string (without the dot).
///
/// Returns `None` if the extension is not recognized.
pub fn detect_format_by_extension(ext: &str) -> Option<ContainerFormat> {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4a" | "m4v" | "mov" => Some(ContainerFormat::Mp4),
        "webm" | "mkv" => Some(ContainerFormat::WebM),
        "wav" => Some(ContainerFormat::Wav),
        _ => None,
    }
}

/// Detect container format from a file path, using extension as fallback.
///
/// Returns `None` if the extension is not recognized.
pub fn detect_format_by_path(path: &Path) -> Option<ContainerFormat> {
    let ext = path.extension()?.to_str()?;
    detect_format_by_extension(ext)
}

/// Detect container format by trying magic bytes first, then falling back to file extension.
///
/// This is the recommended entry point for format detection.
pub fn detect<R: Read + Seek>(reader: &mut R, path: &Path) -> Option<ContainerFormat> {
    detect_format(reader).or_else(|| detect_format_by_path(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_detect_wav() {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(b"RIFF");
        data[4..8].copy_from_slice(&1000u32.to_le_bytes());
        data[8..12].copy_from_slice(b"WAVE");
        let mut cursor = Cursor::new(data);
        assert_eq!(detect_format(&mut cursor), Some(ContainerFormat::Wav));
        // Verify seek back to start
        assert_eq!(cursor.position(), 0);
    }

    #[test]
    fn test_detect_webm() {
        let data = vec![0x1A, 0x45, 0xDF, 0xA3, 0x01, 0x00, 0x00, 0x00];
        let mut cursor = Cursor::new(data);
        assert_eq!(detect_format(&mut cursor), Some(ContainerFormat::WebM));
    }

    #[test]
    fn test_detect_mp4_ftyp() {
        let mut data = vec![0u8; 12];
        data[0..4].copy_from_slice(&8u32.to_be_bytes()); // box size
        data[4..8].copy_from_slice(b"ftyp");
        data[8..12].copy_from_slice(b"isom");
        let mut cursor = Cursor::new(data);
        assert_eq!(detect_format(&mut cursor), Some(ContainerFormat::Mp4));
    }

    #[test]
    fn test_detect_unknown() {
        let data = vec![0xFF, 0xFB, 0x90, 0x00]; // MP3 sync
        let mut cursor = Cursor::new(data);
        assert_eq!(detect_format(&mut cursor), None);
    }

    #[test]
    fn test_detect_by_extension() {
        assert_eq!(detect_format_by_extension("mp4"), Some(ContainerFormat::Mp4));
        assert_eq!(detect_format_by_extension("MOV"), Some(ContainerFormat::Mp4));
        assert_eq!(detect_format_by_extension("webm"), Some(ContainerFormat::WebM));
        assert_eq!(detect_format_by_extension("mkv"), Some(ContainerFormat::WebM));
        assert_eq!(detect_format_by_extension("wav"), Some(ContainerFormat::Wav));
        assert_eq!(detect_format_by_extension("avi"), None);
    }

    #[test]
    fn test_detect_by_path() {
        assert_eq!(
            detect_format_by_path(Path::new("/tmp/video.mp4")),
            Some(ContainerFormat::Mp4)
        );
        assert_eq!(
            detect_format_by_path(Path::new("audio.WAV")),
            Some(ContainerFormat::Wav)
        );
        assert_eq!(detect_format_by_path(Path::new("noext")), None);
    }

    #[test]
    fn test_detect_combined() {
        // Magic bytes take priority over extension
        let mut data = vec![0x1A, 0x45, 0xDF, 0xA3, 0x01, 0x00, 0x00, 0x00];
        data.resize(12, 0);
        let mut cursor = Cursor::new(data);
        // File says .mp4 but magic says WebM — magic wins
        assert_eq!(
            detect(&mut cursor, Path::new("wrong.mp4")),
            Some(ContainerFormat::WebM)
        );
    }

    #[test]
    fn test_detect_fallback_to_extension() {
        // Unknown magic, but extension matches
        let data = vec![0x00; 12];
        let mut cursor = Cursor::new(data);
        assert_eq!(
            detect(&mut cursor, Path::new("video.mp4")),
            Some(ContainerFormat::Mp4)
        );
    }
}
