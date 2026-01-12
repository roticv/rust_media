//! WAV container format implementation

mod demuxer;
mod muxer;

pub use demuxer::WavDemuxer;
pub use muxer::WavMuxer;
