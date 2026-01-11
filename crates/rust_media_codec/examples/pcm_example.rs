//! Example demonstrating PCM encoder and decoder usage

use rust_media_codec::{PcmDecoder, PcmEncoder};
use rust_media_core::{Decoder, Encoder, SampleFormat};

fn main() {
    println!("PCM Codec Example");
    println!("=================\n");

    // Create a PCM encoder
    let encoder = PcmEncoder::from_params(48000, 2, SampleFormat::S16)
        .expect("Failed to create PCM encoder");

    println!("Created PCM encoder:");
    println!("  Codec: {}", encoder.codec());
    println!("  Sample rate: 48000 Hz");
    println!("  Channels: 2 (stereo)");
    println!("  Format: S16 (signed 16-bit)");
    println!();

    // Show encoder capabilities
    let caps = encoder.capabilities();
    println!("Encoder capabilities:");
    println!("  Hardware acceleration: {}", caps.hardware_acceleration);
    println!("  B-frames: {}", caps.b_frames);
    println!("  Requires alignment: {}", caps.requires_alignment);
    println!();

    // Create a PCM decoder with the same stream info
    let stream_info = encoder.stream_info().clone();
    let decoder = PcmDecoder::new(stream_info).expect("Failed to create PCM decoder");

    println!("Created PCM decoder:");
    println!("  Codec: {}", decoder.codec());
    println!();

    // Show decoder capabilities
    let caps = decoder.capabilities();
    println!("Decoder capabilities:");
    println!("  Hardware acceleration: {}", caps.hardware_acceleration);
    println!("  Reordering: {}", caps.reordering);
    println!("  Requires extra data: {}", caps.requires_extra_data);
    println!();

    println!("✅ PCM codec successfully initialized!");
    println!("\nNote: Full encode/decode cycle requires packet buffering,");
    println!("which will be implemented when container formats are ready.");
}
