//! Example: Convert WebM Opus audio to WAV PCM
//!
//! This demonstrates the full audio transcoding pipeline:
//! WebM (container) → Opus (compressed) → PCM (uncompressed) → WAV (container)
//!
//! Usage:
//!   cargo run -p rust_media_format --example webm_to_wav <input.webm> <output.wav>

use rust_media_codec::OpusDecoder;
use rust_media_core::{Decoder, Demuxer, Muxer, Packet, StreamInfo, StreamParams};
use rust_media_format::{WavMuxer, WebmDemuxer};
use std::fs::File;
use std::io::{BufReader, BufWriter};

fn main() {
    println!("WebM Opus → WAV PCM Converter");
    println!("=============================\n");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <input.webm> <output.wav>", args[0]);
        eprintln!("\nThis example converts WebM files with Opus audio to WAV PCM.");
        eprintln!("You can use the test file:\n");
        eprintln!("  cargo run -p rust_media_format --example webm_to_wav \\");
        eprintln!("    test_assets/test_sine_opus.webm output.wav");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    println!("Input:  {}", input_path);
    println!("Output: {}\n", output_path);

    // Step 1: Open WebM file and create demuxer
    println!("Opening WebM file...");
    let input_file = File::open(input_path).expect("Failed to open input file");
    let reader = BufReader::new(input_file);
    let mut demuxer = WebmDemuxer::open(reader).expect("Failed to open WebM demuxer");

    // Get container info
    let container_info = demuxer
        .container_info()
        .expect("Failed to get container info");
    println!("  Format: {}", container_info.format_name);
    if let Some(duration_us) = container_info.duration {
        println!("  Duration: {:.2} seconds", duration_us as f64 / 1_000_000.0);
    }

    // Find Opus audio stream
    let streams = demuxer.streams().expect("Failed to get streams");
    let mut audio_stream_idx = None;
    for (idx, stream) in streams.iter().enumerate() {
        if stream.codec == "opus" {
            audio_stream_idx = Some(idx);
            if let StreamParams::Audio(params) = &stream.params {
                println!("  Opus audio: {} Hz, {} channels", params.sample_rate, params.channels);
            }
            break;
        }
    }

    let audio_stream_idx = audio_stream_idx.expect("No Opus audio stream found in WebM file");
    let opus_stream_info = streams[audio_stream_idx].clone();

    // Step 2: Create Opus decoder
    println!("\nCreating Opus decoder...");
    let mut decoder =
        OpusDecoder::new(opus_stream_info.clone()).expect("Failed to create Opus decoder");
    println!("  Decoder ready");

    // Step 3: Create WAV muxer
    println!("\nCreating WAV muxer...");
    let output_file = File::create(output_path).expect("Failed to create output file");
    let writer = BufWriter::new(output_file);
    let mut muxer = WavMuxer::new(writer);

    // Create PCM stream info for the WAV file
    // We need to get the actual output parameters from the decoder
    let pcm_stream_info = match &opus_stream_info.params {
        StreamParams::Audio(audio_params) => {
            // Create new stream info with PCM codec
            StreamInfo::new(0, rust_media_core::MediaType::Audio, "pcm".to_string())
                .with_time_base(1, 1_000_000)
                .with_params(StreamParams::Audio(audio_params.clone()))
        }
        _ => panic!("Expected audio stream"),
    };

    muxer
        .add_stream(pcm_stream_info)
        .expect("Failed to add stream to muxer");
    muxer
        .write_header()
        .expect("Failed to write WAV header");
    println!("  WAV header written");

    // Step 4: Transcoding pipeline
    println!("\nTranscoding pipeline: WebM → Opus → PCM → WAV");
    println!("----------------------------------------------\n");

    let mut packet_count = 0;
    let mut frame_count = 0;
    let mut total_input_bytes = 0;
    let mut total_output_bytes = 0;

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                // Only process packets from our audio stream
                if packet.stream_index() != audio_stream_idx {
                    continue;
                }

                packet_count += 1;
                total_input_bytes += packet.size();

                // Send Opus packet to decoder
                if let Err(e) = decoder.send_packet(&packet) {
                    eprintln!("Warning: Decoder error: {}", e);
                    continue;
                }

                // Receive PCM frame from decoder
                match decoder.receive_frame() {
                    Ok(frame) => {
                        frame_count += 1;

                        // Get PCM data from frame
                        if let Some(pcm_data) = frame.plane(0) {
                            total_output_bytes += pcm_data.len();

                            // Create packet for WAV muxer
                            let pcm_packet = Packet::new(
                                pcm_data.to_vec(),
                                0,
                                rust_media_core::MediaType::Audio,
                            )
                            .with_pts(frame.pts().unwrap_or(0))
                            .with_duration(frame.duration().unwrap_or(0));

                            // Write PCM packet to WAV file
                            muxer
                                .write_packet(&pcm_packet)
                                .expect("Failed to write packet to WAV");

                            if frame_count <= 3 {
                                println!("Frame {}: {} bytes PCM", frame_count, pcm_data.len());
                            } else if frame_count == 4 {
                                println!("... (continuing transcoding)");
                            }
                        }
                    }
                    Err(rust_media_core::Error::NeedMoreData) => {
                        // Decoder needs more data
                    }
                    Err(e) => {
                        eprintln!("Warning: Frame error: {}", e);
                    }
                }
            }
            Err(rust_media_core::Error::EndOfStream) => {
                println!("\nEnd of stream reached");
                break;
            }
            Err(e) => {
                eprintln!("\nError reading packet: {}", e);
                break;
            }
        }
    }

    // Step 5: Finalize WAV file
    println!("\nFinalizing WAV file...");
    muxer.write_trailer().expect("Failed to write WAV trailer");
    muxer.flush().expect("Failed to flush WAV file");
    println!("  WAV trailer written");

    // Summary
    println!("\n✅ Transcoding complete!");
    println!("\nSummary:");
    println!("  Opus packets: {}", packet_count);
    println!("  PCM frames: {}", frame_count);
    println!(
        "  Input (Opus): {:.2} KB",
        total_input_bytes as f64 / 1024.0
    );
    println!(
        "  Output (PCM): {:.2} KB",
        total_output_bytes as f64 / 1024.0
    );
    if total_input_bytes > 0 {
        let compression_ratio = total_output_bytes as f64 / total_input_bytes as f64;
        println!("  Compression ratio: {:.1}x", compression_ratio);
    }
    println!("\nOutput written to: {}", output_path);
}
