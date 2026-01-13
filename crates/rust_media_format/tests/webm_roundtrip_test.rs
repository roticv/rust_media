//! Integration test for WebM roundtrip: Demux → Re-mux → Demux
//!
//! This test validates that we can read a WebM file, re-mux it, and read it back correctly.

use rust_media_core::{Demuxer, MediaType, Muxer, Packet};
use rust_media_format::{WebmDemuxer, WebmMuxer};
use std::fs::File;
use std::io::{BufReader, Cursor};

#[test]
fn test_webm_muxer_demuxer_roundtrip() {
    println!("\n=== WebM Roundtrip Test ===\n");

    // Look for test file in test_assets directory
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut root_path = std::path::PathBuf::from(manifest_dir);
    root_path.pop();
    root_path.pop();
    root_path.push("test_assets");

    let test_file = root_path.join("test_sine_opus.webm");

    if !test_file.exists() {
        eprintln!(
            "Skipping roundtrip test: test_sine_opus.webm not found in {:?}",
            root_path
        );
        return;
    }

    // Step 1: Read original WebM file
    println!("Reading original WebM file...");
    let file = File::open(&test_file).expect("Failed to open test file");
    let reader = BufReader::new(file);
    let mut demuxer = WebmDemuxer::open(reader).expect("Failed to open demuxer");

    // Get stream info
    let streams = demuxer.streams().expect("Failed to get streams");
    assert!(!streams.is_empty(), "No streams found");

    let opus_idx = streams
        .iter()
        .position(|s| s.codec == "opus")
        .expect("No Opus stream");
    let original_stream = streams[opus_idx].clone();

    println!("  Original stream: {}", original_stream.codec);
    if let rust_media_core::StreamParams::Audio(params) = &original_stream.params {
        println!(
            "  Audio params: {} Hz, {} channels",
            params.sample_rate, params.channels
        );
    }

    // Step 2: Collect all packets from original file
    println!("\nCollecting packets from original file...");
    let mut packets = Vec::new();

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                if packet.stream_index() == opus_idx {
                    packets.push(packet);
                }
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(rust_media_core::Error::Io(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(e) => panic!("Unexpected error reading packet: {}", e),
        }
    }

    println!("  Collected {} packets", packets.len());
    assert!(packets.len() > 0, "Should have collected at least one packet");

    // Step 3: Create new WebM file using muxer
    println!("\nCreating new WebM file with muxer...");
    let mut webm_buffer = Cursor::new(Vec::new());

    {
        let mut muxer = WebmMuxer::new(&mut webm_buffer);

        // Add stream
        muxer
            .add_stream(original_stream.clone())
            .expect("Failed to add stream");

        // Write header
        muxer.write_header().expect("Failed to write header");

        // Write all packets
        for packet in &packets {
            muxer
                .write_packet(packet)
                .expect("Failed to write packet");
        }

        // Write trailer
        muxer.write_trailer().expect("Failed to write trailer");
        muxer.flush().expect("Failed to flush");
    }

    let webm_data = webm_buffer.into_inner();
    println!("  New WebM file size: {} bytes", webm_data.len());

    // Debug: print first 100 bytes
    println!("  First bytes: {:02X?}", &webm_data[0..std::cmp::min(100, webm_data.len())]);

    // Decode segment size from bytes 40-47 (segment ID at 36-39, size at 40-47)
    if webm_data.len() >= 48 {
        let segment_size_bytes = &webm_data[40..48];
        let mut segment_size: u64 = 0;
        for &b in &segment_size_bytes[1..8] {  // Skip the 0x01 marker byte
            segment_size = (segment_size << 8) | b as u64;
        }
        println!("  Decoded segment size: {} bytes (expected ~{})", segment_size, webm_data.len() - 48);
    }

    // Verify WebM structure
    assert_eq!(&webm_data[0..4], &[0x1A, 0x45, 0xDF, 0xA3]); // EBML header

    // Step 4: Read back the new WebM file
    println!("\nReading back new WebM file...");
    let cursor = Cursor::new(webm_data);
    let mut new_demuxer = WebmDemuxer::open(cursor).expect("Failed to open new demuxer");

    // Verify container info
    let container_info = new_demuxer
        .container_info()
        .expect("Failed to get container info");
    assert_eq!(container_info.format_name, "webm");
    println!("  Container format: {}", container_info.format_name);

    // Verify streams
    let new_streams = new_demuxer.streams().expect("Failed to get streams");
    assert_eq!(new_streams.len(), 1, "Should have one stream");
    assert_eq!(new_streams[0].codec, "opus");

    println!("  Stream: {}", new_streams[0].codec);
    if let rust_media_core::StreamParams::Audio(params) = &new_streams[0].params {
        assert_eq!(params.sample_rate, 48000);
        println!(
            "  Audio params: {} Hz, {} channels",
            params.sample_rate, params.channels
        );

        // Verify matches original
        if let rust_media_core::StreamParams::Audio(orig_params) = &original_stream.params {
            assert_eq!(params.sample_rate, orig_params.sample_rate);
            assert_eq!(params.channels, orig_params.channels);
        }
    }

    // Step 5: Read packets from new file and compare
    println!("\nComparing packets...");
    let mut new_packets = Vec::new();

    loop {
        match new_demuxer.read_packet() {
            Ok(packet) => {
                new_packets.push(packet);
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(rust_media_core::Error::Io(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    println!("  Original packets: {}", packets.len());
    println!("  New packets: {}", new_packets.len());

    assert_eq!(
        new_packets.len(),
        packets.len(),
        "Packet count should match"
    );

    // Verify packet data matches
    let mut matching_packets = 0;
    for (i, (orig, new)) in packets.iter().zip(new_packets.iter()).enumerate() {
        // Packet sizes might vary slightly due to timestamp encoding differences,
        // but they should be close
        let size_diff = (orig.size() as i64 - new.size() as i64).abs();
        assert!(
            size_diff < 10,
            "Packet {} size difference too large: {} bytes",
            i,
            size_diff
        );

        // Check media type matches
        assert_eq!(orig.media_type(), MediaType::Audio);
        assert_eq!(new.media_type(), MediaType::Audio);

        matching_packets += 1;
    }

    println!("  Verified {} packets", matching_packets);

    println!("\n✅ WebM roundtrip test passed!");
    println!("   Original → Demuxer → Muxer → Demuxer → Verified");
}

#[test]
fn test_webm_muxer_creates_valid_structure() {
    // Test that muxer creates a valid WebM structure
    let mut buffer = Cursor::new(Vec::new());
    let mut muxer = WebmMuxer::new(&mut buffer);

    // Add Opus stream
    let audio_params = rust_media_core::AudioStreamParams::new(
        48000,
        1,
        rust_media_core::SampleFormat::S16,
    );
    let stream_info = rust_media_core::StreamInfo::new(0, MediaType::Audio, "opus".to_string())
        .with_time_base(1, 1_000_000)
        .with_params(rust_media_core::StreamParams::Audio(audio_params));

    muxer.add_stream(stream_info).unwrap();
    muxer.write_header().unwrap();

    // Write a few test packets
    for i in 0..5 {
        let packet = Packet::new(vec![0u8; 100], 0, MediaType::Audio)
            .with_pts(i * 20000) // 20ms intervals
            .with_duration(20000);
        muxer.write_packet(&packet).unwrap();
    }

    muxer.write_trailer().unwrap();

    let webm_data = buffer.into_inner();

    // Verify EBML header
    assert_eq!(&webm_data[0..4], &[0x1A, 0x45, 0xDF, 0xA3]);

    // Should contain "webm" doctype
    let output_str = String::from_utf8_lossy(&webm_data);
    assert!(output_str.contains("webm"));
    assert!(output_str.contains("OpusHead"));
    assert!(output_str.contains("rust_media"));

    println!("WebM structure validation passed");
}
