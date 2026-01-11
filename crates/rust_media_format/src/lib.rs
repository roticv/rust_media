//! Container format implementations (demuxers and muxers)
//!
//! This crate provides implementations of container format demuxers and muxers:
//! - **WAV** (RIFF WAVE) - ✅ Implemented
//! - MP4/MOV (ISO Base Media File Format) - Planned
//! - MKV (Matroska) - Planned
//! - WebM (Matroska subset) - Planned
//!
//! All implementations use streaming APIs for bounded memory usage.

pub mod wav;

// Placeholder modules for future implementations
// pub mod mp4;
// pub mod mkv;
// pub mod webm;

// Re-export commonly used demuxers
pub use wav::WavDemuxer;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
