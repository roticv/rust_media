//! Example demonstrating the full pipeline: WAV demuxer → PCM decoder
//!
//! This shows how the streaming architecture works end-to-end:
//! WAV file → WavDemuxer → Packets → PcmDecoder → Frames

use rust_media_codec::PcmDecoder;
use rust_media_core::{Decoder, Demuxer, StreamParams};
use rust_media_format::WavDemuxer;
use std::io::Cursor;

fn create_sample_wav() -> Vec<u8> {
    let mut data = Vec::new();

    // RIFF header
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&100u32.to_le_bytes());
    data.extend_from_slice(b"WAVE");

    // fmt chunk - 48kHz, stereo, 16-bit PCM
    data.extend_from_slice(b"fmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes()); // PCM
    data.extend_from_slice(&2u16.to_le_bytes()); // Stereo
    data.extend_from_slice(&48000u32.to_le_bytes()); // Sample rate
    data.extend_from_slice(&192000u32.to_le_bytes()); // Byte rate
    data.extend_from_slice(&4u16.to_le_bytes()); // Block align
    data.extend_from_slice(&16u16.to_le_bytes()); // Bits per sample

    // data chunk
    data.extend_from_slice(b"data");
    data.extend_from_slice(&64u32.to_le_bytes());

    // Sample audio data (16 frames of stereo 16-bit)
    for i in 0..16 {
        let sample = (i * 1000) as i16;
        data.extend_from_slice(&sample.to_le_bytes());
        data.extend_from_slice(&sample.to_le_bytes());
    }

    data
}

fn main() {
    println!("WAV → PCM Pipeline Example");
    println!("==========================\n");

    // Step 1: Create sample WAV file
    let wav_data = create_sample_wav();
    println!("✓ Created sample WAV file: {} bytes\n", wav_data.len());

    // Step 2: Open with WAV demuxer
    let cursor = Cursor::new(wav_data);
    let mut demuxer = WavDemuxer::open(cursor).expect("Failed to open WAV file");
    println!("✓ WAV demuxer initialized");

    let streams = demuxer.streams().expect("Failed to get streams");
    let stream_info = &streams[0];

    if let StreamParams::Audio(params) = &stream_info.params {
        println!("  Audio format: {} Hz, {} channels, {:?}",
            params.sample_rate,
            params.channels,
            params.sample_format
        );
    }
    println!();

    // Step 3: Create PCM decoder
    let mut decoder = PcmDecoder::new(stream_info.clone())
        .expect("Failed to create PCM decoder");
    println!("✓ PCM decoder initialized");
    println!("  Codec: {}", decoder.codec());
    println!();

    // Step 4: Streaming pipeline
    println!("Pipeline: WAV file → Demuxer → Packets → Decoder → Frames");
    println!("-----------------------------------------------------------\n");

    let mut packet_count = 0;
    let mut total_packet_bytes = 0;

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                packet_count += 1;
                total_packet_bytes += packet.size();

                println!("Step {}: Demuxer produced packet", packet_count);
                println!("  Size: {} bytes", packet.size());
                println!("  PTS: {:?} μs", packet.pts());

                // Send packet to decoder
                match decoder.send_packet(&packet) {
                    Ok(_) => println!("  ✓ Sent to decoder"),
                    Err(e) => println!("  ✗ Decoder error: {}", e),
                }

                // Try to receive frames (in a real implementation, this would work)
                match decoder.receive_frame() {
                    Ok(_frame) => {
                        println!("  ✓ Decoder produced frame");
                    }
                    Err(rust_media_core::Error::NeedMoreData) => {
                        println!("  ⋯ Decoder needs more data (frame buffering not yet implemented)");
                    }
                    Err(e) => {
                        println!("  ✗ Frame error: {}", e);
                    }
                }

                println!();

                if packet_count >= 3 {
                    println!("(Stopping after {} packets for demo)\n", packet_count);
                    break;
                }
            }
            Err(rust_media_core::Error::EndOfStream) => {
                println!("✓ End of stream\n");
                break;
            }
            Err(e) => {
                eprintln!("✗ Error: {}\n", e);
                break;
            }
        }
    }

    println!("Summary:");
    println!("--------");
    println!("  Packets demuxed: {}", packet_count);
    println!("  Total bytes: {}", total_packet_bytes);
    println!();
    println!("✅ Streaming pipeline demonstration complete!");
    println!();
    println!("Note: Full decode requires packet → frame conversion,");
    println!("which will be implemented in the next iteration.");
}
