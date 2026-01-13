//! Test WebM remuxing: Read Opus packets from WebM and write to new WebM

use rust_media_core::{Demuxer, Muxer};
use rust_media_format::{WebmDemuxer, WebmMuxer};
use std::fs::File;
use std::io::{BufReader, BufWriter};

#[test]
fn test_webm_remux_to_file() {
    println!("\n=== WebM Remux Test (File Output) ===\n");

    // Look for test file
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut root_path = std::path::PathBuf::from(manifest_dir);
    root_path.pop();
    root_path.pop();
    root_path.push("test_assets");

    let input_file = root_path.join("test_sine_opus.webm");
    if !input_file.exists() {
        eprintln!("Skipping: test_sine_opus.webm not found");
        return;
    }

    let output_file = std::env::temp_dir().join("test_remuxed.webm");

    // Step 1: Read original WebM
    println!("Reading: {:?}", input_file);
    let file = File::open(&input_file).expect("Failed to open input");
    let reader = BufReader::new(file);
    let mut demuxer = WebmDemuxer::open(reader).expect("Failed to open demuxer");

    let streams = demuxer.streams().expect("Failed to get streams");
    let opus_idx = streams
        .iter()
        .position(|s| s.codec == "opus")
        .expect("No Opus stream");
    let stream_info = streams[opus_idx].clone();

    println!("  Stream: {} ({}Hz, {} ch)",
        stream_info.codec,
        if let rust_media_core::StreamParams::Audio(p) = &stream_info.params { p.sample_rate } else { 0 },
        if let rust_media_core::StreamParams::Audio(p) = &stream_info.params { p.channels } else { 0 }
    );

    // Step 2: Collect packets
    let mut packets = Vec::new();
    loop {
        match demuxer.read_packet() {
            Ok(packet) if packet.stream_index() == opus_idx => packets.push(packet),
            Ok(_) => continue,
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(rust_media_core::Error::Io(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(e) => panic!("Error: {}", e),
        }
    }

    println!("  Collected {} packets\n", packets.len());
    assert!(packets.len() > 0);

    // Step 3: Write to new WebM file
    println!("Writing: {:?}", output_file);
    let out_file = File::create(&output_file).expect("Failed to create output");
    let writer = BufWriter::new(out_file);
    let mut muxer = WebmMuxer::new(writer);

    muxer
        .add_stream(stream_info)
        .expect("Failed to add stream");
    muxer.write_header().expect("Failed to write header");

    for packet in &packets {
        muxer
            .write_packet(packet)
            .expect("Failed to write packet");
    }

    muxer.write_trailer().expect("Failed to write trailer");
    muxer.flush().expect("Failed to flush");

    println!("  Wrote {} packets\n", packets.len());

    // Step 4: Verify output file
    let metadata = std::fs::metadata(&output_file).expect("Failed to get metadata");
    println!("Output file size: {} bytes", metadata.len());
    assert!(metadata.len() > 100);

    // Step 5: Read back the file
    println!("\nReading back output file...");
    let file = File::open(&output_file).expect("Failed to open output");
    let reader = BufReader::new(file);
    let mut verify_demuxer = WebmDemuxer::open(reader).expect("Failed to open output for verification");

    let verify_streams = verify_demuxer.streams().expect("Failed to get streams");
    assert_eq!(verify_streams.len(), 1);
    assert_eq!(verify_streams[0].codec, "opus");
    println!("  ✓ File structure valid");

    let mut read_packets = 0;
    loop {
        match verify_demuxer.read_packet() {
            Ok(_) => read_packets += 1,
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(rust_media_core::Error::Io(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(e) => panic!("Error reading: {}", e),
        }
    }

    println!("  ✓ Read {} packets back", read_packets);
    assert_eq!(read_packets, packets.len());

    // Clean up
    std::fs::remove_file(&output_file).ok();

    println!("\n✅ WebM remux test passed!");
    println!("   {} packets: Input → Muxer → Output → Verified", packets.len());
}
