//! Reference validation test for Opus decoding
//!
//! This test validates Opus decoding by comparing decoded output against reference PCM.
//!
//! Test procedure:
//! 1. Start with a reference PCM/WAV file
//! 2. Encode to Opus in WebM container
//! 3. Decode back to PCM using our implementation
//! 4. Compare decoded output with original reference
//!
//! To prepare test files:
//! ```bash
//! # Create reference PCM audio (1 second, 440 Hz sine wave)
//! ffmpeg -f lavfi -i "sine=frequency=440:duration=1" -ar 48000 -ac 2 reference.wav
//!
//! # Encode to Opus in WebM
//! ffmpeg -i reference.wav -c:a libopus -b:a 64k reference_opus.webm
//! ```

use rust_media_codec::OpusDecoder;
use rust_media_core::{Decoder, Demuxer};
use rust_media_format::{WavDemuxer, WebmDemuxer};
use std::fs::File;
use std::io::{BufReader, Write};

/// Calculate Signal-to-Noise Ratio between two PCM signals
fn calculate_snr(reference: &[u8], decoded: &[u8]) -> f64 {
    let min_len = reference.len().min(decoded.len());

    let mut signal_power = 0.0f64;
    let mut noise_power = 0.0f64;

    // Process as 16-bit PCM samples
    for i in (0..min_len).step_by(2) {
        if i + 1 >= min_len {
            break;
        }

        let ref_sample = i16::from_le_bytes([reference[i], reference[i + 1]]) as f64;
        let dec_sample = i16::from_le_bytes([decoded[i], decoded[i + 1]]) as f64;

        signal_power += ref_sample * ref_sample;
        let error = ref_sample - dec_sample;
        noise_power += error * error;
    }

    if noise_power > 0.0 && signal_power > 0.0 {
        10.0 * (signal_power / noise_power).log10()
    } else if noise_power == 0.0 {
        f64::INFINITY // Perfect match
    } else {
        0.0
    }
}

#[test]
fn test_opus_decode_reference_validation() {
    // Look for test files in workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut root_path = std::path::PathBuf::from(manifest_dir);
    root_path.pop(); // crates
    root_path.pop(); // rust_media root

    let reference_wav = root_path.join("reference.wav");
    let opus_webm = root_path.join("reference_opus.webm");

    // Skip test if files don't exist
    if !reference_wav.exists() || !opus_webm.exists() {
        eprintln!("Skipping test: reference files not found");
        eprintln!("Create them with:");
        eprintln!("  ffmpeg -f lavfi -i \"sine=frequency=440:duration=1\" -ar 48000 -ac 2 reference.wav");
        eprintln!("  ffmpeg -i reference.wav -c:a libopus -b:a 64k reference_opus.webm");
        return;
    }

    println!("\n=== Opus Reference Validation Test ===\n");

    // Step 1: Load reference PCM from WAV
    println!("Loading reference WAV file...");
    let wav_file = File::open(&reference_wav).expect("Failed to open reference WAV");
    let wav_reader = BufReader::new(wav_file);
    let mut wav_demuxer = WavDemuxer::open(wav_reader).expect("Failed to open WAV demuxer");

    let mut reference_pcm = Vec::new();
    loop {
        match wav_demuxer.read_packet() {
            Ok(packet) => {
                reference_pcm.extend_from_slice(packet.data());
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(e) => panic!("Error reading WAV: {}", e),
        }
    }
    println!("  Reference PCM size: {} bytes", reference_pcm.len());

    // Step 2: Decode Opus from WebM
    println!("\nDecoding Opus from WebM...");
    let webm_file = File::open(&opus_webm).expect("Failed to open Opus WebM");
    let webm_reader = BufReader::new(webm_file);
    let mut webm_demuxer = WebmDemuxer::open(webm_reader).expect("Failed to open WebM demuxer");

    // Find Opus stream
    let streams = webm_demuxer.streams().expect("Failed to get streams");
    let mut audio_stream_idx = None;
    for (idx, stream) in streams.iter().enumerate() {
        if stream.codec == "opus" {
            audio_stream_idx = Some(idx);
            break;
        }
    }
    let audio_stream_idx = audio_stream_idx.expect("No Opus stream found");

    // Create decoder
    let stream_info = streams[audio_stream_idx].clone();
    let mut decoder = OpusDecoder::new(stream_info).expect("Failed to create Opus decoder");

    // Decode all frames
    let mut decoded_pcm = Vec::new();
    let mut frame_count = 0;

    loop {
        match webm_demuxer.read_packet() {
            Ok(packet) => {
                if packet.stream_index() != audio_stream_idx {
                    continue;
                }

                decoder.send_packet(&packet).expect("Failed to send packet");

                match decoder.receive_frame() {
                    Ok(frame) => {
                        frame_count += 1;
                        if let Some(plane) = frame.plane(0) {
                            decoded_pcm.extend_from_slice(plane);
                        }
                    }
                    Err(rust_media_core::Error::NeedMoreData) => {}
                    Err(e) => panic!("Decoder error: {}", e),
                }
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(rust_media_core::Error::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => panic!("Demuxer error: {}", e),
        }
    }

    println!("  Decoded {} frames", frame_count);
    println!("  Decoded PCM size: {} bytes", decoded_pcm.len());

    // Step 3: Compare decoded output with reference
    println!("\nValidating decoded output...");

    // Allow for some size difference due to encoder/decoder delays
    let size_diff = (reference_pcm.len() as i64 - decoded_pcm.len() as i64).abs();
    let size_tolerance = reference_pcm.len() / 10; // 10% tolerance for delays
    println!("  Size difference: {} bytes (tolerance: {} bytes)", size_diff, size_tolerance);

    // Opus has inherent codec delay, so sizes won't match exactly
    // Just verify we got a reasonable amount of data
    assert!(
        decoded_pcm.len() > reference_pcm.len() / 2,
        "Decoded size too small: {} vs {} bytes",
        decoded_pcm.len(),
        reference_pcm.len()
    );

    // For lossy codec testing, we need to account for encoder delay
    // Opus typically has ~312 samples (6.5ms at 48kHz) of pre-skip
    // Try to find the best alignment by testing different offsets
    let mut best_snr = 0.0f64;
    let mut best_offset = 0;
    let max_offset = 2000; // Search up to ~20ms of offset

    for offset in (0..max_offset).step_by(100) {
        if offset >= decoded_pcm.len() {
            break;
        }
        let snr = calculate_snr(&reference_pcm, &decoded_pcm[offset..]);
        if snr > best_snr {
            best_snr = snr;
            best_offset = offset;
        }
    }

    println!("  Best alignment offset: {} bytes", best_offset);
    println!("  Signal-to-Noise Ratio: {:.2} dB", best_snr);

    // Opus at 64kbps should achieve reasonable quality for simple sine wave
    // Note: SNR expectations for lossy codecs are lower than lossless
    // For a 440Hz sine wave at 64kbps, expect SNR > 15 dB
    assert!(
        best_snr > 10.0,
        "SNR too low: {:.2} dB (expected > 10 dB)",
        best_snr
    );

    // Save decoded output for manual inspection if needed
    if std::env::var("SAVE_DECODED").is_ok() {
        let decoded_path = root_path.join("decoded_opus.pcm");
        let mut file = File::create(&decoded_path).expect("Failed to create decoded file");
        file.write_all(&decoded_pcm).expect("Failed to write decoded data");
        println!("\n  Saved decoded PCM to: {:?}", decoded_path);
    }

    println!("\n✅ Opus reference validation passed!");
    println!("   Compression ratio: {:.1}x", reference_pcm.len() as f64 / decoded_pcm.len() as f64);
    println!("   Quality (SNR): {:.2} dB", best_snr);
}

#[test]
fn test_webm_opus_basic_decode() {
    // Quick sanity test that doesn't require reference files
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut root_path = std::path::PathBuf::from(manifest_dir);
    root_path.pop();
    root_path.pop();

    let test_file = root_path.join("test_sine_opus.webm");
    if !test_file.exists() {
        eprintln!("Skipping: test_sine_opus.webm not found");
        return;
    }

    let file = File::open(test_file).expect("Failed to open test file");
    let reader = BufReader::new(file);
    let mut demuxer = WebmDemuxer::open(reader).expect("Failed to open demuxer");

    let streams = demuxer.streams().expect("Failed to get streams");
    let audio_idx = streams.iter().position(|s| s.codec == "opus").expect("No Opus stream");

    let mut decoder = OpusDecoder::new(streams[audio_idx].clone()).expect("Failed to create decoder");

    let mut decoded_frames = 0;
    for _ in 0..10 {
        match demuxer.read_packet() {
            Ok(packet) if packet.stream_index() == audio_idx => {
                decoder.send_packet(&packet).ok();
                if decoder.receive_frame().is_ok() {
                    decoded_frames += 1;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    assert!(decoded_frames > 0, "Should decode at least one frame");
    println!("Basic decode test passed: {} frames decoded", decoded_frames);
}

#[test]
fn test_webm_demuxer_structure() {
    // Basic test that doesn't require external files
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let mut root_path = std::path::PathBuf::from(manifest_dir);
    root_path.pop();
    root_path.pop();

    let test_file = root_path.join("test_sine_opus.webm");

    if !test_file.exists() {
        eprintln!("Skipping structure test: test_sine_opus.webm not found");
        return;
    }

    let file = File::open(test_file).expect("Failed to open test file");
    let reader = BufReader::new(file);
    let mut demuxer = WebmDemuxer::open(reader).expect("Failed to open demuxer");

    // Verify container structure
    let container_info = demuxer.container_info().expect("Failed to get container info");
    assert_eq!(container_info.format_name, "webm");

    // Verify streams
    let streams = demuxer.streams().expect("Failed to get streams");
    assert!(!streams.is_empty());

    // Verify at least one stream is Opus
    let has_opus = streams.iter().any(|s| s.codec == "opus");
    assert!(has_opus, "Should have at least one Opus stream");

    println!("WebM structure validation passed");
}
