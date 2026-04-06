//! Safe Rust wrapper around libvpx for VP8/VP9 encoding and decoding.
//!
//! Bindings are generated at build time via bindgen, so they always match
//! the system-installed libvpx version — no ABI mismatch possible.

mod decoder;
mod encoder;
mod error;
mod image;

pub use decoder::{Decoder, DecoderConfig, DecodedFrame};
pub use encoder::{Encoder, EncoderConfig, EncodedPacket, Deadline, FrameFlags, RateControl};
pub use error::{Error, Result};
pub use image::Codec;
