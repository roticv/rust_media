//! WAV to WebM converter example
//!
//! Converts a WAV file to WebM using Opus encoding.
//!
//! Usage: cargo run --example wav_to_webm <input.wav> <output.webm>

use rust_media_codec::{OpusEncoder, PcmDecoder};
use rust_media_core::{Decoder, Demuxer, Encoder, MediaType, Muxer, StreamInfo, StreamParams};
use rust_media_format::{WavDemuxer, WebmMuxer};
use std::env;
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.wav> <output.webm>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    println!("Converting {} to {}", input_path, output_path);

    // Open WAV file
    let input_file = File::open(input_path)?;
    let reader = BufReader::new(input_file);
    let mut wav_demuxer = WavDemuxer::open(reader)?;

    let wav_streams = wav_demuxer.streams()?;
    let wav_stream = wav_streams[0].clone();

    println!("\nInput WAV:");
    if let StreamParams::Audio(params) = &wav_stream.params {
        println!("  Sample rate: {} Hz", params.sample_rate);
        println!("  Channels: {}", params.channels);
        println!("  Sample format: {:?}", params.sample_format);
    }

    // Create Opus encoder
    let mut opus_encoder = OpusEncoder::new(wav_stream.clone())?;
    println!("\nEncoder: Opus (20ms frames @ {} Hz)",
        if let StreamParams::Audio(p) = &wav_stream.params { p.sample_rate } else { 0 });

    // Create WebM muxer
    let output_file = File::create(output_path)?;
    let mut webm_muxer = WebmMuxer::new(output_file);

    // Create Opus stream info for WebM
    let opus_stream = match &wav_stream.params {
        StreamParams::Audio(audio_params) => {
            StreamInfo::new(0, MediaType::Audio, "opus".to_string())
                .with_time_base(1, 1_000_000)
                .with_params(StreamParams::Audio(audio_params.clone()))
        }
        _ => panic!("Expected audio stream"),
    };

    webm_muxer.add_stream(opus_stream)?;
    webm_muxer.write_header()?;

    println!("\nEncoding...");

    // Create PCM decoder
    let mut pcm_decoder = PcmDecoder::new(wav_stream)?;

    let mut packets_read = 0;
    let mut frames_decoded = 0;
    let mut packets_written = 0;

    // Read, decode, encode, and write
    loop {
        match wav_demuxer.read_packet() {
            Ok(packet) => {
                packets_read += 1;

                // Decode PCM packet to frame
                pcm_decoder.send_packet(&packet)?;
                loop {
                    match pcm_decoder.receive_frame() {
                        Ok(frame) => {
                            frames_decoded += 1;

                            // Encode frame to Opus
                            opus_encoder.send_frame(&frame)?;

                            // Retrieve all available encoded packets
                            loop {
                                match opus_encoder.receive_packet() {
                                    Ok(opus_packet) => {
                                        webm_muxer.write_packet(&opus_packet)?;
                                        packets_written += 1;

                                        if packets_written % 10 == 0 {
                                            print!(".");
                                            std::io::Write::flush(&mut std::io::stdout())?;
                                        }
                                    }
                                    Err(rust_media_core::Error::NeedMoreData) => break,
                                    Err(e) => return Err(e.into()),
                                }
                            }
                        }
                        Err(rust_media_core::Error::NeedMoreData) => break,
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(e) => return Err(e.into()),
        }
    }

    // Flush encoder
    opus_encoder.flush()?;
    loop {
        match opus_encoder.receive_packet() {
            Ok(opus_packet) => {
                webm_muxer.write_packet(&opus_packet)?;
                packets_written += 1;
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(rust_media_core::Error::NeedMoreData) => break,
            Err(e) => return Err(e.into()),
        }
    }

    // Finalize WebM file
    webm_muxer.write_trailer()?;
    webm_muxer.flush()?;

    println!("\n\n✅ Conversion complete!");
    println!("\nStatistics:");
    println!("  WAV packets read: {}", packets_read);
    println!("  PCM frames decoded: {}", frames_decoded);
    println!("  Opus packets encoded: {}", packets_written);

    // Get file sizes
    let input_size = std::fs::metadata(input_path)?.len();
    let output_size = std::fs::metadata(output_path)?.len();
    let compression_ratio = input_size as f64 / output_size as f64;

    println!("\nFile sizes:");
    println!("  Input (WAV): {} bytes ({:.2} KB)", input_size, input_size as f64 / 1024.0);
    println!("  Output (WebM): {} bytes ({:.2} KB)", output_size, output_size as f64 / 1024.0);
    println!("  Compression ratio: {:.1}x", compression_ratio);

    println!("\nOutput written to: {}", output_path);

    Ok(())
}
