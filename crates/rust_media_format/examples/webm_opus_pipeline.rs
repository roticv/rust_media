//! Example demonstrating the full pipeline: WebM demuxer → Opus decoder → PCM
//!
//! This shows how to extract and decode Opus audio from a WebM file:
//! WebM file → WebmDemuxer → Opus Packets → OpusDecoder → PCM Frames

use rust_media_codec::OpusDecoder;
use rust_media_core::{Decoder, Demuxer};
use rust_media_format::WebmDemuxer;
use std::fs::File;
use std::io::BufReader;

fn main() {
    println!("WebM → Opus → PCM Pipeline Example");
    println!("===================================\n");

    // Get WebM file path from command line
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <webm_file>", args[0]);
        eprintln!("\nThis example reads a WebM file with Opus audio and decodes it to PCM.");
        eprintln!("You can use the test file:");
        eprintln!("  cargo run -p rust_media_format --example webm_opus_pipeline test_assets/test_sine_opus.webm");
        eprintln!("\nOr create your own with:");
        eprintln!("  ffmpeg -f lavfi -i \"sine=frequency=440:duration=1\" -c:a libopus test.webm");
        std::process::exit(1);
    }

    let webm_path = &args[1];

    // Step 1: Open WebM file
    let file = File::open(webm_path).expect("Failed to open WebM file");
    let reader = BufReader::new(file);
    let mut demuxer = WebmDemuxer::open(reader).expect("Failed to open WebM demuxer");

    println!("✓ WebM demuxer initialized");

    // Get container info
    let container_info = demuxer.container_info().expect("Failed to get container info");
    println!("  Format: {}", container_info.format_name);
    if let Some(duration) = container_info.duration {
        println!("  Duration: {:.2} seconds", duration as f64 / 1_000_000.0);
    }
    println!();

    // Get stream info
    let streams = demuxer.streams().expect("Failed to get streams");
    println!("Number of streams: {}", streams.len());
    println!();

    // Find first audio stream
    let mut audio_stream_idx = None;
    for (idx, stream) in streams.iter().enumerate() {
        println!("Stream {}:", idx);
        println!("  Codec: {}", stream.codec);
        println!("  Time base: {}/{}", stream.time_base.0, stream.time_base.1);

        if let rust_media_core::StreamParams::Audio(params) = &stream.params {
            println!("  Sample rate: {} Hz", params.sample_rate);
            println!("  Channels: {} ({})", params.channels, params.channel_layout);
            println!("  Sample format: {:?}", params.sample_format);

            if stream.codec == "opus" && audio_stream_idx.is_none() {
                audio_stream_idx = Some(idx);
                println!("  → Selected for decoding");
            }
        }
        println!();
    }

    let audio_stream_idx = audio_stream_idx.expect("No Opus audio stream found");

    // Step 2: Create Opus decoder
    let stream_info = streams[audio_stream_idx].clone();
    let mut decoder = OpusDecoder::new(stream_info.clone())
        .expect("Failed to create Opus decoder");

    println!("✓ Opus decoder initialized");
    println!("  Codec: {}", decoder.codec());
    println!();

    // Step 3: Streaming pipeline
    println!("Pipeline: WebM file → Demuxer → Opus Packets → Decoder → PCM Frames");
    println!("-----------------------------------------------------------------------\n");

    let mut packet_count = 0;
    let mut frame_count = 0;
    let mut total_packet_bytes = 0;
    let mut total_frame_bytes = 0;

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                // Only process packets from our audio stream
                if packet.stream_index() != audio_stream_idx {
                    continue;
                }

                packet_count += 1;
                total_packet_bytes += packet.size();

                if packet_count <= 3 {
                    println!("Packet {}:", packet_count);
                    println!("  Size: {} bytes", packet.size());
                    println!("  PTS: {:?} μs", packet.pts());
                }

                // Send packet to decoder
                match decoder.send_packet(&packet) {
                    Ok(_) => {
                        if packet_count <= 3 {
                            println!("  ✓ Sent to decoder");
                        }
                    }
                    Err(e) => {
                        println!("  ✗ Decoder error: {}", e);
                        continue;
                    }
                }

                // Try to receive frames
                match decoder.receive_frame() {
                    Ok(frame) => {
                        frame_count += 1;
                        let frame_size = frame.plane(0).map(|p| p.len()).unwrap_or(0);
                        total_frame_bytes += frame_size;

                        if packet_count <= 3 {
                            println!("  ✓ Decoder produced frame");
                            println!("    Frame size: {} bytes", frame_size);
                            println!("    PTS: {:?} μs", frame.pts());
                        }
                    }
                    Err(rust_media_core::Error::NeedMoreData) => {
                        if packet_count <= 3 {
                            println!("  ⋯ Decoder needs more data");
                        }
                    }
                    Err(e) => {
                        println!("  ✗ Frame error: {}", e);
                    }
                }

                if packet_count <= 3 {
                    println!();
                }

                if packet_count == 3 {
                    println!("... (continuing to process remaining packets)\n");
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
    println!("  Total packet bytes: {} ({:.2} KB)", total_packet_bytes, total_packet_bytes as f64 / 1024.0);
    println!("  Frames decoded: {}", frame_count);
    println!("  Total frame bytes: {} ({:.2} KB)", total_frame_bytes, total_frame_bytes as f64 / 1024.0);

    if total_packet_bytes > 0 {
        let compression_ratio = total_frame_bytes as f64 / total_packet_bytes as f64;
        println!("  Compression ratio: {:.1}x", compression_ratio);
    }

    println!();
    println!("✅ WebM → Opus → PCM pipeline demonstration complete!");
    println!();
    println!("This demonstrates the full streaming pipeline:");
    println!("  1. WebM demuxer extracts Opus packets from the container");
    println!("  2. Opus decoder decompresses packets to PCM audio frames");
    println!("  3. PCM frames can be written to WAV or processed further");
}
