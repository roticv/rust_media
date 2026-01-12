//! Integration test for WebM demuxer + Opus decoder pipeline
//!
//! This test demonstrates the complete pipeline:
//! WebM file → Demuxer → Opus packets → Decoder → PCM frames

use rust_media_codec::OpusDecoder;
use rust_media_core::{Decoder, Demuxer};
use rust_media_format::WebmDemuxer;
use std::fs::File;
use std::io::BufReader;

#[test]
fn test_webm_opus_pipeline() {
    // This test requires a test WebM file with Opus audio
    // Create it with: ffmpeg -f lavfi -i "sine=frequency=440:duration=1" -c:a libopus test_sine_opus.webm

    // Look for test file in workspace root
    let webm_path = std::env::var("CARGO_MANIFEST_DIR")
        .map(|dir| {
            let mut path = std::path::PathBuf::from(dir);
            path.pop(); // Go up from rust_media_format to crates
            path.pop(); // Go up from crates to rust_media root
            path.push("test_sine_opus.webm");
            path
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("test_sine_opus.webm"));

    // Skip test if file doesn't exist
    if !webm_path.exists() {
        eprintln!("Skipping test: {:?} not found", webm_path);
        eprintln!("Create it with: ffmpeg -f lavfi -i \"sine=frequency=440:duration=1\" -c:a libopus test_sine_opus.webm");
        return;
    }

    // Open WebM file
    let file = File::open(&webm_path).expect("Failed to open WebM file");

    let reader = BufReader::new(file);
    let mut demuxer = WebmDemuxer::open(reader).expect("Failed to open WebM demuxer");

    // Verify container info
    let container_info = demuxer.container_info().expect("Failed to get container info");
    assert_eq!(container_info.format_name, "webm");
    assert!(container_info.duration.is_some());

    // Get streams
    let streams = demuxer.streams().expect("Failed to get streams");
    assert!(!streams.is_empty(), "No streams found");

    // Find Opus audio stream
    let mut audio_stream_idx = None;
    for (idx, stream) in streams.iter().enumerate() {
        if stream.codec == "opus" {
            audio_stream_idx = Some(idx);
            break;
        }
    }

    let audio_stream_idx = audio_stream_idx.expect("No Opus audio stream found");
    let stream_info = streams[audio_stream_idx].clone();

    // Verify audio parameters
    if let rust_media_core::StreamParams::Audio(params) = &stream_info.params {
        assert_eq!(params.sample_rate, 48000, "Opus should be 48kHz");
        assert!(params.channels > 0, "Should have at least one channel");
    } else {
        panic!("Expected audio stream parameters");
    }

    // Create Opus decoder
    let mut decoder = OpusDecoder::new(stream_info).expect("Failed to create Opus decoder");
    assert_eq!(decoder.codec(), "opus");

    // Process packets through the pipeline
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

                // Send packet to decoder
                decoder.send_packet(&packet).expect("Failed to send packet");

                // Receive frame
                match decoder.receive_frame() {
                    Ok(frame) => {
                        frame_count += 1;
                        let frame_size = frame.plane(0).map(|p| p.len()).unwrap_or(0);
                        total_frame_bytes += frame_size;

                        // Verify frame has valid PTS
                        assert!(frame.pts().is_some(), "Frame should have PTS");
                    }
                    Err(rust_media_core::Error::NeedMoreData) => {
                        // Decoder needs more data (buffering)
                    }
                    Err(e) => panic!("Unexpected decoder error: {}", e),
                }
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(rust_media_core::Error::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => panic!("Unexpected demuxer error: {}", e),
        }
    }

    // Verify we processed data
    assert!(packet_count > 0, "Should have demuxed at least one packet");
    assert!(frame_count > 0, "Should have decoded at least one frame");
    assert!(total_packet_bytes > 0, "Should have processed packet data");
    assert!(total_frame_bytes > 0, "Should have produced frame data");

    // Verify compression ratio (Opus should achieve at least 5x)
    let compression_ratio = total_frame_bytes as f64 / total_packet_bytes as f64;
    assert!(compression_ratio >= 5.0, "Compression ratio should be at least 5x, got {:.1}x", compression_ratio);

    println!("WebM → Opus → PCM pipeline test passed!");
    println!("  Packets: {}", packet_count);
    println!("  Frames: {}", frame_count);
    println!("  Compression ratio: {:.1}x", compression_ratio);
}

#[test]
fn test_webm_demuxer_invalid_file() {
    let invalid_data = vec![0u8; 100];
    let cursor = std::io::Cursor::new(invalid_data);
    let result = WebmDemuxer::open(cursor);
    assert!(result.is_err(), "Should reject invalid WebM file");
}
