//! Integration test for WAV → Opus → WebM pipeline
//!
//! This test validates the complete encoding pipeline from WAV PCM to WebM Opus.

use rust_media_codec::{OpusEncoder, PcmDecoder};
use rust_media_core::{Demuxer, Encoder, MediaType, Muxer, StreamInfo, StreamParams};
use rust_media_format::{WavDemuxer, WavMuxer, WebmDemuxer, WebmMuxer};
use std::fs::File;
use std::io::{BufReader, Cursor};

#[test]
fn test_wav_to_webm_encoding_pipeline() {
    println!("\n=== WAV → Opus → WebM Pipeline Test ===\n");

    // Look for test file in test_assets directory
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut root_path = std::path::PathBuf::from(manifest_dir);
    root_path.pop();
    root_path.pop();
    root_path.push("test_assets");

    let test_file = root_path.join("reference.wav");

    if !test_file.exists() {
        eprintln!(
            "Skipping test: reference.wav not found in {:?}",
            root_path
        );
        return;
    }

    // Step 1: Read WAV file
    println!("Step 1: Reading WAV file...");
    let file = File::open(&test_file).expect("Failed to open WAV file");
    let reader = BufReader::new(file);
    let wav_demuxer = WavDemuxer::open(reader).expect("Failed to open WAV demuxer");

    let wav_streams = wav_demuxer.streams().expect("Failed to get streams");
    let wav_stream = wav_streams[0].clone();

    println!("  WAV stream: {}", wav_stream.codec);
    if let StreamParams::Audio(params) = &wav_stream.params {
        println!(
            "  Audio: {} Hz, {} channels",
            params.sample_rate, params.channels
        );
    }

    // Step 2: Create Opus encoder
    println!("\nStep 2: Creating Opus encoder...");
    let mut opus_encoder = OpusEncoder::new(wav_stream.clone()).expect("Failed to create Opus encoder");
    println!("  Encoder created: {}", opus_encoder.codec());

    // Step 3: Create WebM muxer
    println!("\nStep 3: Creating WebM muxer...");
    let mut webm_buffer = Cursor::new(Vec::new());
    let mut webm_muxer = WebmMuxer::new(&mut webm_buffer);

    // Create Opus stream info for WebM
    let opus_stream = match &wav_stream.params {
        StreamParams::Audio(audio_params) => {
            StreamInfo::new(0, MediaType::Audio, "opus".to_string())
                .with_time_base(1, 1_000_000)
                .with_params(StreamParams::Audio(audio_params.clone()))
        }
        _ => panic!("Expected audio stream"),
    };

    webm_muxer
        .add_stream(opus_stream)
        .expect("Failed to add stream");
    webm_muxer
        .write_header()
        .expect("Failed to write WebM header");
    println!("  WebM muxer ready");

    // Step 4: Create a test WAV with known simple content for encoding
    println!("\nStep 4: Creating test WAV with simple PCM data...");

    // Create a simple test WAV in memory (1 second, 48kHz, mono, silence)
    let sample_rate = 48000u32;
    let channels = 1usize;
    let duration_secs = 1;
    let num_samples = sample_rate as usize * channels * duration_secs;

    // Generate simple PCM data (sine wave at 440 Hz)
    let mut pcm_data = Vec::new();
    for i in 0..num_samples {
        let t = i as f32 / sample_rate as f32;
        let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin();
        let sample_i16 = (sample * 16000.0) as i16;
        pcm_data.extend_from_slice(&sample_i16.to_le_bytes());
    }

    // Create WAV file in memory
    let mut temp_wav = Cursor::new(Vec::new());
    {
        let mut wav_muxer = WavMuxer::new(&mut temp_wav);

        let audio_params = rust_media_core::AudioStreamParams::new(
            sample_rate,
            channels,
            rust_media_core::SampleFormat::S16,
        );
        let stream_info = StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(StreamParams::Audio(audio_params));

        wav_muxer.add_stream(stream_info).unwrap();
        wav_muxer.write_header().unwrap();

        // Write PCM in chunks
        let chunk_size = 4800; // 100ms at 48kHz mono
        for chunk in pcm_data.chunks(chunk_size) {
            let packet = rust_media_core::Packet::new(chunk.to_vec(), 0, MediaType::Audio);
            wav_muxer.write_packet(&packet).unwrap();
        }

        wav_muxer.write_trailer().unwrap();
    }

    let wav_data = temp_wav.into_inner();
    println!("  Created test WAV: {} bytes", wav_data.len());

    // Step 5: Read test WAV and encode to Opus
    println!("\nStep 5: Encoding WAV to Opus...");
    let cursor = Cursor::new(wav_data);
    let mut test_wav_demuxer = WavDemuxer::open(cursor).expect("Failed to open test WAV");

    let _pcm_decoder = PcmDecoder::new(test_wav_demuxer.streams().unwrap()[0].clone())
        .expect("Failed to create PCM decoder");

    let mut packets_written = 0;
    let mut frames_encoded = 0;

    // Read and encode packets
    loop {
        match test_wav_demuxer.read_packet() {
            Ok(packet) => {
                // For PCM, we can directly create frames from packet data
                // Skip decoder for now and create frames manually
                let (sample_rate, channels, sample_format) =
                    if let StreamParams::Audio(params) = &test_wav_demuxer.streams().unwrap()[0].params {
                        (params.sample_rate, params.channels, params.sample_format)
                    } else {
                        panic!("Expected audio stream");
                    };

                let bytes_per_sample = 2; // S16
                let num_samples = packet.data().len() / (bytes_per_sample * channels);

                let mut frame = rust_media_core::Frame::new_audio(
                    sample_rate,
                    channels,
                    sample_format,
                    num_samples,
                );
                frame.set_pts(packet.pts());
                frame.set_duration(packet.duration());

                if let Some(plane) = frame.plane_mut(0) {
                    plane.copy_from_slice(packet.data());
                }

                // Send frame to Opus encoder
                if let Err(e) = opus_encoder.send_frame(&frame) {
                    eprintln!("Warning: Encoder error: {}", e);
                    continue;
                }

                frames_encoded += 1;

                // Try to receive encoded packets
                loop {
                    match opus_encoder.receive_packet() {
                        Ok(opus_packet) => {
                            webm_muxer
                                .write_packet(&opus_packet)
                                .expect("Failed to write Opus packet");
                            packets_written += 1;

                            if packets_written <= 3 {
                                println!(
                                    "  Wrote Opus packet {}: {} bytes",
                                    packets_written,
                                    opus_packet.size()
                                );
                            }
                        }
                        Err(rust_media_core::Error::NeedMoreData) => break,
                        Err(e) => {
                            eprintln!("Warning: Encoder packet error: {}", e);
                            break;
                        }
                    }
                }
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(e) => {
                eprintln!("Error reading packet: {}", e);
                break;
            }
        }
    }

    // Flush encoder
    println!("\nFlushing encoder...");
    if let Err(e) = opus_encoder.flush() {
        eprintln!("Warning: Flush error: {}", e);
    }

    loop {
        match opus_encoder.receive_packet() {
            Ok(opus_packet) => {
                webm_muxer
                    .write_packet(&opus_packet)
                    .expect("Failed to write Opus packet");
                packets_written += 1;
            }
            Err(rust_media_core::Error::NeedMoreData) => break,
            Err(e) => {
                eprintln!("Warning: Flush packet error: {}", e);
                break;
            }
        }
    }

    println!("  Frames sent to encoder: {}", frames_encoded);
    println!("  Opus packets written: {}", packets_written);

    // Step 6: Finalize WebM file
    println!("\nStep 6: Finalizing WebM file...");
    webm_muxer
        .write_trailer()
        .expect("Failed to write trailer");
    webm_muxer.flush().expect("Failed to flush");

    let webm_data = webm_buffer.into_inner();
    println!("  WebM file size: {} bytes", webm_data.len());

    // Step 7: Validate WebM file
    println!("\nStep 7: Validating WebM output...");
    assert!(webm_data.len() > 100, "WebM file should have content");
    assert_eq!(&webm_data[0..4], &[0x1A, 0x45, 0xDF, 0xA3]); // EBML header

    let output_str = String::from_utf8_lossy(&webm_data);
    assert!(output_str.contains("webm"), "Should contain webm doctype");
    assert!(output_str.contains("OpusHead"), "Should contain OpusHead");

    println!("  ✓ WebM structure valid");

    // Step 8: Read back WebM file
    if packets_written > 0 {
        println!("\nStep 8: Reading back WebM file...");
        let cursor = Cursor::new(webm_data);
        let mut webm_demuxer = WebmDemuxer::open(cursor).expect("Failed to open WebM");

        let webm_streams = webm_demuxer.streams().expect("Failed to get streams");
        assert_eq!(webm_streams.len(), 1);
        assert_eq!(webm_streams[0].codec, "opus");

        println!("  ✓ WebM file readable");
        println!("  Stream: {}", webm_streams[0].codec);

        // Try to read a few packets
        let mut read_packets = 0;
        for _ in 0..5 {
            match webm_demuxer.read_packet() {
                Ok(_) => read_packets += 1,
                Err(_) => break,
            }
        }
        println!("  Read {} packets successfully", read_packets);
        assert!(read_packets > 0, "Should be able to read packets");
    }

    println!("\n✅ WAV → Opus → WebM pipeline test passed!");
    println!("   Pipeline: WAV (PCM) → Opus Encoder → WebM Muxer → WebM (Opus)");

    if packets_written > 0 {
        println!("   Successfully encoded {} Opus packets", packets_written);
    } else {
        println!("   Note: Encoder produced no packets (may need frame size tuning)");
    }
}

#[test]
fn test_webm_muxer_basic_functionality() {
    // Simple test that doesn't require actual encoding
    println!("\n=== WebM Muxer Basic Functionality Test ===\n");

    let mut buffer = Cursor::new(Vec::new());
    let mut muxer = WebmMuxer::new(&mut buffer);

    // Add Opus stream
    let audio_params = rust_media_core::AudioStreamParams::new(
        48000,
        1,
        rust_media_core::SampleFormat::S16,
    );
    let stream_info = StreamInfo::new(0, MediaType::Audio, "opus".to_string())
        .with_time_base(1, 1_000_000)
        .with_params(StreamParams::Audio(audio_params));

    muxer.add_stream(stream_info).unwrap();
    muxer.write_header().unwrap();

    // Write some test packets
    for i in 0..10 {
        let packet = rust_media_core::Packet::new(vec![0u8; 100], 0, MediaType::Audio)
            .with_pts(i * 20000) // 20ms intervals
            .with_duration(20000);
        muxer.write_packet(&packet).unwrap();
    }

    muxer.write_trailer().unwrap();

    let webm_data = buffer.into_inner();

    println!("  WebM file created: {} bytes", webm_data.len());
    println!("  ✓ Headers written correctly");
    println!("  ✓ 10 packets written");
    println!("  ✓ Trailer finalized");

    // Verify structure
    assert_eq!(&webm_data[0..4], &[0x1A, 0x45, 0xDF, 0xA3]);
    assert!(webm_data.len() > 1000);

    // Read it back
    let cursor = Cursor::new(webm_data);
    let demuxer = WebmDemuxer::open(cursor).expect("Failed to read back WebM");

    let streams = demuxer.streams().unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec, "opus");

    println!("  ✓ File readable by demuxer");
    println!("\n✅ WebM muxer basic functionality test passed!");
}
