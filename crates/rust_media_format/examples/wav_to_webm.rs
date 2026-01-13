//! Example: Convert WAV PCM audio to WebM Opus
//!
//! This demonstrates the full audio encoding pipeline:
//! WAV (PCM uncompressed) → PCM Decoder → Opus Encoder → WebM (Opus compressed)
//!
//! Usage:
//!   cargo run -p rust_media_format --example wav_to_webm <input.wav> <output.webm>

use rust_media_codec::OpusEncoder;
use rust_media_core::{Demuxer, Encoder, Frame, MediaType, Muxer, StreamInfo, StreamParams};
use rust_media_format::{WavDemuxer, WebmMuxer};
use std::fs::File;
use std::io::{BufReader, BufWriter};

fn main() {
    println!("WAV PCM → WebM Opus Converter");
    println!("==============================\n");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: {} <input.wav> <output.webm>", args[0]);
        eprintln!("\nThis example converts WAV files with PCM audio to WebM with Opus.");
        eprintln!("You can use the test file:\n");
        eprintln!("  cargo run -p rust_media_format --example wav_to_webm \\");
        eprintln!("    test_assets/reference.wav output.webm");
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    println!("Input:  {}", input_path);
    println!("Output: {}\n", output_path);

    // Step 1: Open WAV file and create demuxer
    println!("Opening WAV file...");
    let input_file = File::open(input_path).expect("Failed to open input file");
    let reader = BufReader::new(input_file);
    let mut demuxer = WavDemuxer::open(reader).expect("Failed to open WAV demuxer");

    // Get container info
    let container_info = demuxer
        .container_info()
        .expect("Failed to get container info");
    println!("  Format: {}", container_info.format_name);
    if let Some(duration_us) = container_info.duration {
        println!("  Duration: {:.2} seconds", duration_us as f64 / 1_000_000.0);
    }

    // Get stream info
    let streams = demuxer.streams().expect("Failed to get streams");
    let pcm_stream_info = streams[0].clone();

    if let StreamParams::Audio(params) = &pcm_stream_info.params {
        println!(
            "  PCM audio: {} Hz, {} channels",
            params.sample_rate, params.channels
        );
    }

    // Step 2: Create Opus encoder
    println!("\nCreating Opus encoder...");
    let mut encoder = OpusEncoder::new(pcm_stream_info.clone()).expect("Failed to create Opus encoder");
    println!("  Encoder ready");
    println!("  Codec: {}", encoder.codec());

    // Step 3: Create WebM muxer
    println!("\nCreating WebM muxer...");
    let output_file = File::create(output_path).expect("Failed to create output file");
    let writer = BufWriter::new(output_file);
    let mut muxer = WebmMuxer::new(writer);

    // Create Opus stream info for the WebM file
    let opus_stream_info = match &pcm_stream_info.params {
        StreamParams::Audio(audio_params) => {
            StreamInfo::new(0, MediaType::Audio, "opus".to_string())
                .with_time_base(1, 1_000_000)
                .with_params(StreamParams::Audio(audio_params.clone()))
        }
        _ => panic!("Expected audio stream"),
    };

    muxer
        .add_stream(opus_stream_info)
        .expect("Failed to add stream to muxer");
    muxer
        .write_header()
        .expect("Failed to write WebM header");
    println!("  WebM header written");

    // Step 4: Encoding pipeline
    println!("\nEncoding pipeline: WAV → PCM → Opus → WebM");
    println!("-------------------------------------------\n");

    let mut packet_count = 0;
    let mut frame_count = 0;
    let mut opus_packet_count = 0;
    let mut total_input_bytes = 0;
    let mut total_output_bytes = 0;

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                packet_count += 1;
                total_input_bytes += packet.size();

                // Convert PCM packet data to frame
                let (sample_rate, channels, sample_format) = if let StreamParams::Audio(params) = &pcm_stream_info.params {
                    (params.sample_rate, params.channels, params.sample_format)
                } else {
                    panic!("Expected audio stream");
                };

                // Calculate number of samples
                let bytes_per_sample = match sample_format {
                    rust_media_core::SampleFormat::U8 => 1,
                    rust_media_core::SampleFormat::S16 => 2,
                    rust_media_core::SampleFormat::S32 => 4,
                    _ => 2,
                };
                let num_samples = packet.data().len() / (bytes_per_sample * channels);

                let mut frame = Frame::new_audio(sample_rate, channels, sample_format, num_samples);
                frame.set_pts(packet.pts());
                frame.set_duration(packet.duration());

                // Copy PCM data to frame
                if let Some(plane) = frame.plane_mut(0) {
                    plane.copy_from_slice(packet.data());
                }

                frame_count += 1;

                // Encode PCM frame to Opus
                if let Err(e) = encoder.send_frame(&frame) {
                    eprintln!("Warning: Encoder error: {}", e);
                    continue;
                }

                // Try to receive opus packet (encoder may buffer)
                loop {
                    match encoder.receive_packet() {
                        Ok(opus_packet) => {
                            opus_packet_count += 1;
                            total_output_bytes += opus_packet.size();

                            // Write Opus packet to WebM file
                            muxer
                                .write_packet(&opus_packet)
                                .expect("Failed to write packet to WebM");

                            if opus_packet_count <= 3 {
                                println!(
                                    "Opus packet {}: {} bytes",
                                    opus_packet_count,
                                    opus_packet.size()
                                );
                            } else if opus_packet_count == 4 {
                                println!("... (continuing encoding)");
                            }
                        }
                        Err(rust_media_core::Error::NeedMoreData) => {
                            break; // No more packets available
                        }
                        Err(e) => {
                            eprintln!("Warning: Encoder packet error: {}", e);
                            break;
                        }
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

    // Flush encoder to get any remaining packets
    println!("\nFlushing encoder...");
    if let Err(e) = encoder.flush() {
        eprintln!("Warning: Encoder flush error: {}", e);
    }

    loop {
        match encoder.receive_packet() {
            Ok(opus_packet) => {
                opus_packet_count += 1;
                total_output_bytes += opus_packet.size();

                muxer
                    .write_packet(&opus_packet)
                    .expect("Failed to write packet to WebM");
            }
            Err(rust_media_core::Error::NeedMoreData) => break,
            Err(e) => {
                eprintln!("Warning: Flush packet error: {}", e);
                break;
            }
        }
    }

    // Step 5: Finalize WebM file
    println!("Finalizing WebM file...");
    muxer
        .write_trailer()
        .expect("Failed to write WebM trailer");
    muxer.flush().expect("Failed to flush WebM file");
    println!("  WebM trailer written");

    // Summary
    println!("\n✅ Encoding complete!");
    println!("\nSummary:");
    println!("  PCM packets: {}", packet_count);
    println!("  PCM frames: {}", frame_count);
    println!("  Opus packets: {}", opus_packet_count);
    println!(
        "  Input (PCM): {:.2} KB",
        total_input_bytes as f64 / 1024.0
    );
    println!(
        "  Output (Opus): {:.2} KB",
        total_output_bytes as f64 / 1024.0
    );
    if total_output_bytes > 0 {
        let compression_ratio = total_input_bytes as f64 / total_output_bytes as f64;
        println!("  Compression ratio: {:.1}x", compression_ratio);
    }
    println!("\nOutput written to: {}", output_path);
}
