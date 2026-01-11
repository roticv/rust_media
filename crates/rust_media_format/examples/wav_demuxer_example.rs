//! Example demonstrating WAV demuxer usage

use rust_media_core::{Demuxer, StreamParams};
use rust_media_format::WavDemuxer;
use std::io::Cursor;

fn create_sample_wav() -> Vec<u8> {
    let mut data = Vec::new();

    // RIFF header
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&100u32.to_le_bytes()); // File size - 8
    data.extend_from_slice(b"WAVE");

    // fmt chunk
    data.extend_from_slice(b"fmt ");
    data.extend_from_slice(&16u32.to_le_bytes()); // Chunk size
    data.extend_from_slice(&1u16.to_le_bytes()); // Audio format (PCM)
    data.extend_from_slice(&2u16.to_le_bytes()); // Num channels (stereo)
    data.extend_from_slice(&48000u32.to_le_bytes()); // Sample rate
    data.extend_from_slice(&192000u32.to_le_bytes()); // Byte rate
    data.extend_from_slice(&4u16.to_le_bytes()); // Block align
    data.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample

    // data chunk
    data.extend_from_slice(b"data");
    data.extend_from_slice(&64u32.to_le_bytes()); // Data size

    // Sample audio data (16 frames of stereo 16-bit audio = 64 bytes)
    for i in 0..16 {
        let sample = (i * 1000) as i16;
        data.extend_from_slice(&sample.to_le_bytes()); // Left channel
        data.extend_from_slice(&sample.to_le_bytes()); // Right channel
    }

    data
}

fn main() {
    println!("WAV Demuxer Example");
    println!("===================\n");

    // Create a sample WAV file in memory
    let wav_data = create_sample_wav();
    println!("Created sample WAV file: {} bytes", wav_data.len());
    println!();

    // Open the WAV file
    let cursor = Cursor::new(wav_data);
    let mut demuxer = WavDemuxer::open(cursor).expect("Failed to open WAV file");

    // Get container info
    let container_info = demuxer.container_info().expect("Failed to get container info");
    println!("Container info:");
    println!("  Format: {}", container_info.format_name);
    if let Some(duration) = container_info.duration {
        println!("  Duration: {} microseconds", duration);
    }
    println!();

    // Get stream info
    let streams = demuxer.streams().expect("Failed to get streams");
    println!("Number of streams: {}", streams.len());
    println!();

    for (idx, stream) in streams.iter().enumerate() {
        println!("Stream {}:", idx);
        println!("  Codec: {}", stream.codec);
        println!("  Time base: {}/{}", stream.time_base.0, stream.time_base.1);

        if let StreamParams::Audio(params) = &stream.params {
            println!("  Sample rate: {} Hz", params.sample_rate);
            println!("  Channels: {} ({})", params.channels, params.channel_layout);
            println!("  Sample format: {:?}", params.sample_format);
            println!("  Bits per sample: {}", params.bits_per_sample);
        }
        println!();
    }

    // Read packets (streaming API)
    println!("Reading packets:");
    let mut packet_count = 0;
    let mut total_bytes = 0;

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                packet_count += 1;
                total_bytes += packet.size();

                println!(
                    "  Packet {}: {} bytes, PTS: {:?}, duration: {:?}",
                    packet_count,
                    packet.size(),
                    packet.pts(),
                    packet.duration()
                );

                if packet_count >= 5 {
                    println!("  ... (stopping after 5 packets for demo)");
                    break;
                }
            }
            Err(rust_media_core::Error::EndOfStream) => {
                println!("  End of stream reached");
                break;
            }
            Err(e) => {
                eprintln!("Error reading packet: {}", e);
                break;
            }
        }
    }

    println!();
    println!("Summary:");
    println!("  Total packets read: {}", packet_count);
    println!("  Total bytes: {}", total_bytes);
    println!();
    println!("✅ WAV demuxer successfully read PCM audio data!");
}
