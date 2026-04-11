//! Audio codec implementations

pub mod mp3;
pub mod opus;
pub mod pcm;
pub mod vorbis;

// FDK-AAC codec (optional feature)
#[cfg(feature = "fdk-aac")]
pub mod fdk_aac;
