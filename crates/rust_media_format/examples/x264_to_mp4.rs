//! Example: Encode video and audio with x264/AAC and mux to MP4
//!
//! This example demonstrates:
//! 1. Creating synthetic YUV420P video frames
//! 2. Encoding them with the X264Encoder (H.264)
//! 3. Creating synthetic PCM audio samples
//! 4. Encoding them with the FdkAacEncoder (AAC)
//! 5. Muxing both streams into an MP4 container
//!
//! # Running
//!
//! ```bash
//! cargo run -p rust_media_format --features "gpl-x264 fdk-aac" --example x264_to_mp4
//! ```
//!
//! # Output
//!
//! Creates `/tmp/test_x264_output.mp4` - a 5-second video with audio.
//! You can play it with: `ffplay /tmp/test_x264_output.mp4`

#[cfg(feature = "gpl-x264")]
use rust_media_codec::X264Encoder;
#[cfg(feature = "fdk-aac")]
use rust_media_codec::FdkAacEncoder;
use rust_media_core::frame::Frame;
use rust_media_core::stream::{AudioStreamParams, StreamInfo, StreamParams, VideoStreamParams};
use rust_media_core::types::{ColorRange, ColorSpace, MediaType, PixelFormat, SampleFormat};
use rust_media_core::{Encoder, Muxer};
use rust_media_format::mp4::Mp4Muxer;
use std::fs::File;
use std::io::BufWriter;

/// Video configuration
const WIDTH: usize = 640;
const HEIGHT: usize = 480;
const FPS: u32 = 30;
const DURATION_SECS: u32 = 5;
const VIDEO_BITRATE: u64 = 1_000_000; // 1 Mbps

/// Audio configuration
const SAMPLE_RATE: u32 = 48000;
const CHANNELS: usize = 2;
const AUDIO_BITRATE: u64 = 128000; // 128 kbps

/// Generate a YUV420P frame with a moving gradient pattern
fn generate_test_frame(frame_num: u32, width: usize, height: usize) -> Frame {
    // Create a new video frame
    let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);

    // Create a moving gradient pattern
    let offset = (frame_num * 4) as usize % 256;

    // Fill Y plane (luma)
    if let Some(y_plane) = frame.plane_mut(0) {
        for row in 0..height {
            for col in 0..width {
                let idx = row * width + col;
                // Horizontal gradient that moves over time
                let y_val = ((col + offset + row / 2) % 256) as u8;
                y_plane[idx] = y_val.max(16).min(235); // Keep within valid Y range
            }
        }
    }

    // Fill U plane (chroma blue)
    if let Some(u_plane) = frame.plane_mut(1) {
        let uv_width = width / 2;
        let uv_height = height / 2;
        for row in 0..uv_height {
            for col in 0..uv_width {
                let idx = row * uv_width + col;
                // Create a subtle color shift
                let u_val = (128i32 + ((col as i32 - (uv_width / 2) as i32) * 50 / uv_width as i32)) as u8;
                u_plane[idx] = u_val.max(16).min(240);
            }
        }
    }

    // Fill V plane (chroma red)
    if let Some(v_plane) = frame.plane_mut(2) {
        let uv_width = width / 2;
        let uv_height = height / 2;
        for row in 0..uv_height {
            for col in 0..uv_width {
                let idx = row * uv_width + col;
                let v_val = (128i32
                    + (((row + offset / 4) as i32 - (uv_height / 2) as i32) * 50 / uv_height as i32))
                    as u8;
                v_plane[idx] = v_val.max(16).min(240);
            }
        }
    }

    frame.with_pts(frame_num as i64)
}

/// Generate audio samples for one frame worth of video (1/FPS seconds)
/// Creates a stereo sine wave tone
fn generate_audio_samples(start_sample: u64, num_samples: usize, frequency: f64) -> Vec<i16> {
    let mut samples = Vec::with_capacity(num_samples * CHANNELS);

    for i in 0..num_samples {
        let sample_idx = start_sample + i as u64;
        let t = sample_idx as f64 / SAMPLE_RATE as f64;

        // Generate sine wave at the given frequency
        let value = (t * frequency * 2.0 * std::f64::consts::PI).sin();

        // Convert to i16 with reasonable amplitude
        let sample = (value * 16000.0) as i16;

        // Interleaved stereo - same value for both channels
        samples.push(sample);
        samples.push(sample);
    }

    samples
}

#[cfg(all(feature = "gpl-x264", feature = "fdk-aac"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== X264/AAC to MP4 Encoding Example ===\n");
    println!("Video Configuration:");
    println!("  Resolution: {}x{}", WIDTH, HEIGHT);
    println!("  Frame rate: {} fps", FPS);
    println!("  Duration:   {} seconds", DURATION_SECS);
    println!("  Bitrate:    {} kbps", VIDEO_BITRATE / 1000);
    println!();
    println!("Audio Configuration:");
    println!("  Sample rate: {} Hz", SAMPLE_RATE);
    println!("  Channels:    {}", CHANNELS);
    println!("  Bitrate:     {} kbps", AUDIO_BITRATE / 1000);
    println!();

    let total_frames = FPS * DURATION_SECS;
    let output_path = "/tmp/test_x264_output.mp4";

    // Create stream info for H.264 video (stream index 0)
    let video_stream_info = StreamInfo {
        index: 0,
        media_type: MediaType::Video,
        codec: "h264".to_string(),
        time_base: (1, FPS),
        duration: Some(total_frames as i64),
        bitrate: Some(VIDEO_BITRATE),
        params: StreamParams::Video(VideoStreamParams {
            width: WIDTH,
            height: HEIGHT,
            pixel_format: PixelFormat::YUV420P,
            frame_rate: (FPS, 1),
            color_space: ColorSpace::BT709,
            color_range: ColorRange::Limited,
            sample_aspect_ratio: (1, 1),
            bit_depth: 8,
        }),
        extra_data: vec![],
    };

    // Create stream info for AAC audio (stream index 1)
    let audio_stream_info = StreamInfo {
        index: 1,
        media_type: MediaType::Audio,
        codec: "aac".to_string(),
        time_base: (1, SAMPLE_RATE),
        duration: Some((SAMPLE_RATE * DURATION_SECS) as i64),
        bitrate: Some(AUDIO_BITRATE),
        params: StreamParams::Audio(AudioStreamParams::new(
            SAMPLE_RATE,
            CHANNELS,
            SampleFormat::S16,
        )),
        extra_data: vec![],
    };

    // Create X264 encoder
    println!("Creating X264 video encoder...");
    let mut video_encoder = X264Encoder::new(video_stream_info.clone())?;
    println!("  Video encoder created successfully");

    // Create AAC encoder
    println!("Creating AAC audio encoder...");
    let mut audio_encoder = FdkAacEncoder::new(audio_stream_info.clone())?;
    println!("  Audio encoder created successfully");

    // Update audio stream info with AudioSpecificConfig from encoder
    let mut audio_stream_with_config = audio_stream_info.clone();
    audio_stream_with_config.extra_data = audio_encoder.audio_specific_config().to_vec();

    // Create MP4 muxer
    println!("Creating MP4 muxer...");
    let output_file = File::create(output_path)?;
    let writer = BufWriter::new(output_file);
    let mut muxer = Mp4Muxer::new(writer);

    // Add streams and write header
    muxer.add_stream(video_stream_info)?;
    muxer.add_stream(audio_stream_with_config)?;
    muxer.write_header()?;
    println!("  MP4 header written");

    println!("\nEncoding {} video frames with audio...", total_frames);

    let mut frames_encoded = 0;
    let mut video_packets_written = 0;
    let mut audio_packets_written = 0;
    let mut audio_sample_pos: u64 = 0;

    // Samples per video frame (for synchronized audio generation)
    let samples_per_video_frame = SAMPLE_RATE / FPS;

    // Tone frequency - A4 note (440 Hz)
    let tone_frequency = 440.0;

    // Encode frames
    for frame_num in 0..total_frames {
        // === Video ===
        // Generate and encode video frame
        let video_frame = generate_test_frame(frame_num, WIDTH, HEIGHT);
        video_encoder.send_frame(&video_frame)?;

        // Receive encoded video packets
        loop {
            match video_encoder.receive_packet() {
                Ok(packet) => {
                    muxer.write_packet(&packet)?;
                    video_packets_written += 1;
                }
                Err(rust_media_core::Error::NeedMoreData) => break,
                Err(e) => return Err(e.into()),
            }
        }

        // === Audio ===
        // Generate audio samples for this video frame's duration
        let audio_samples = generate_audio_samples(
            audio_sample_pos,
            samples_per_video_frame as usize,
            tone_frequency,
        );
        audio_sample_pos += samples_per_video_frame as u64;

        // Create audio frame with interleaved samples
        let mut audio_frame = Frame::new_audio(
            SAMPLE_RATE,
            CHANNELS,
            SampleFormat::S16,
            samples_per_video_frame as usize,
        );

        // Copy samples to frame
        if let Some(data) = audio_frame.plane_mut(0) {
            for (i, sample) in audio_samples.iter().enumerate() {
                let bytes = sample.to_le_bytes();
                data[i * 2] = bytes[0];
                data[i * 2 + 1] = bytes[1];
            }
        }

        // Set PTS in audio timebase (samples)
        let audio_pts = (frame_num as u64 * samples_per_video_frame as u64) as i64;
        let audio_frame = audio_frame.with_pts(audio_pts);

        // Send to encoder
        audio_encoder.send_frame(&audio_frame)?;

        // Receive encoded audio packets
        loop {
            match audio_encoder.receive_packet() {
                Ok(mut packet) => {
                    // Set stream index for audio
                    packet.set_stream_index(1);
                    muxer.write_packet(&packet)?;
                    audio_packets_written += 1;
                }
                Err(rust_media_core::Error::NeedMoreData) => break,
                Err(e) => return Err(e.into()),
            }
        }

        frames_encoded += 1;

        // Progress indicator
        if frame_num % 30 == 0 || frame_num == total_frames - 1 {
            let progress = (frame_num + 1) as f32 / total_frames as f32 * 100.0;
            print!(
                "\r  Progress: {:.1}% ({}/{} frames)",
                progress,
                frame_num + 1,
                total_frames
            );
            std::io::Write::flush(&mut std::io::stdout())?;
        }
    }

    // Flush video encoder
    println!("\n\nFlushing video encoder...");
    video_encoder.flush()?;

    // Get remaining video packets after flush
    loop {
        match video_encoder.receive_packet() {
            Ok(packet) => {
                muxer.write_packet(&packet)?;
                video_packets_written += 1;
            }
            Err(rust_media_core::Error::NeedMoreData) | Err(rust_media_core::Error::EndOfStream) => {
                break
            }
            Err(e) => return Err(e.into()),
        }
    }

    // Flush audio encoder
    println!("Flushing audio encoder...");
    audio_encoder.flush()?;

    // Get remaining audio packets after flush
    loop {
        match audio_encoder.receive_packet() {
            Ok(mut packet) => {
                packet.set_stream_index(1);
                muxer.write_packet(&packet)?;
                audio_packets_written += 1;
            }
            Err(rust_media_core::Error::NeedMoreData) | Err(rust_media_core::Error::EndOfStream) => {
                break
            }
            Err(e) => return Err(e.into()),
        }
    }

    // Finalize MP4
    muxer.write_trailer()?;
    muxer.flush()?;

    // Print results
    let output_size = std::fs::metadata(output_path)?.len();

    println!("\n=== Encoding Complete ===");
    println!("  Video frames encoded: {}", frames_encoded);
    println!("  Video packets written: {}", video_packets_written);
    println!("  Audio packets written: {}", audio_packets_written);
    println!("  Output file:     {}", output_path);
    println!(
        "  Output size:     {} bytes ({:.2} MB)",
        output_size,
        output_size as f64 / 1024.0 / 1024.0
    );

    // Calculate actual bitrate
    let actual_bitrate = output_size as f64 * 8.0 / DURATION_SECS as f64;
    println!("  Actual bitrate:  {:.0} kbps", actual_bitrate / 1000.0);

    println!("\nTo play the output:");
    println!("  ffplay {}", output_path);
    println!("  # or");
    println!("  vlc {}", output_path);

    println!("\nTo inspect the file:");
    println!("  ffprobe {}", output_path);

    Ok(())
}

#[cfg(all(feature = "gpl-x264", not(feature = "fdk-aac")))]
fn main() {
    eprintln!("This example requires the 'fdk-aac' feature for audio encoding.");
    eprintln!();
    eprintln!("Run with:");
    eprintln!("  cargo run -p rust_media_format --features \"gpl-x264 fdk-aac\" --example x264_to_mp4");
    std::process::exit(1);
}

#[cfg(not(feature = "gpl-x264"))]
fn main() {
    eprintln!("This example requires the 'gpl-x264' feature for video encoding.");
    eprintln!();
    eprintln!("Run with:");
    eprintln!("  cargo run -p rust_media_format --features \"gpl-x264 fdk-aac\" --example x264_to_mp4");
    eprintln!();
    eprintln!("Note: Enabling gpl-x264 changes the license of the compiled binary to GPL v2+.");
    std::process::exit(1);
}
