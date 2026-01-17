//! Audio codec implementations

pub mod opus;
pub mod pcm;

// FDK-AAC codec (optional feature)
#[cfg(feature = "fdk-aac")]
pub mod fdk_aac;
