//! Example: Encode video frames with x264 and mux to MP4
//!
//! This example demonstrates:
//! 1. Creating synthetic YUV420P video frames
//! 2. Encoding them with the X264Encoder (H.264)
//! 3. Muxing the encoded packets into an MP4 container
//!
//! # Running
//!
//! ```bash
//! cargo run -p rust_media_format --features gpl-x264 --example x264_to_mp4
//! ```
//!
//! # Output
//!
//! Creates `/tmp/test_x264_output.mp4` - a 5-second video with a moving gradient pattern.
//! You can play it with: `ffplay /tmp/test_x264_output.mp4`

#[cfg(feature = "gpl-x264")]
use rust_media_codec::X264Encoder;
use rust_media_core::frame::Frame;
use rust_media_core::stream::{StreamInfo, StreamParams, VideoStreamParams};
use rust_media_core::types::{ColorRange, ColorSpace, MediaType, PixelFormat};
use rust_media_core::{Encoder, Muxer};
use rust_media_format::mp4::Mp4Muxer;
use std::fs::File;
use std::io::BufWriter;

/// Video configuration
const WIDTH: usize = 640;
const HEIGHT: usize = 480;
const FPS: u32 = 30;
const DURATION_SECS: u32 = 5;
const BITRATE: u64 = 1_000_000; // 1 Mbps

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

#[cfg(feature = "gpl-x264")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== X264 to MP4 Encoding Example ===\n");
    println!("Configuration:");
    println!("  Resolution: {}x{}", WIDTH, HEIGHT);
    println!("  Frame rate: {} fps", FPS);
    println!("  Duration:   {} seconds", DURATION_SECS);
    println!("  Bitrate:    {} kbps", BITRATE / 1000);
    println!();

    let total_frames = FPS * DURATION_SECS;
    let output_path = "/tmp/test_x264_output.mp4";

    // Create stream info for H.264 video
    let stream_info = StreamInfo {
        index: 0,
        media_type: MediaType::Video,
        codec: "h264".to_string(),
        time_base: (1, FPS),
        duration: Some(total_frames as i64),
        bitrate: Some(BITRATE),
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
        extra_data: vec![], // x264 will provide SPS/PPS in packet headers
    };

    // Create X264 encoder
    println!("Creating X264 encoder...");
    let mut encoder = X264Encoder::new(stream_info.clone())?;
    println!("  Encoder created successfully");

    // Create MP4 muxer
    println!("Creating MP4 muxer...");
    let output_file = File::create(output_path)?;
    let writer = BufWriter::new(output_file);
    let mut muxer = Mp4Muxer::new(writer);

    // Add stream and write header
    muxer.add_stream(stream_info)?;
    muxer.write_header()?;
    println!("  MP4 header written");

    println!("\nEncoding {} frames...", total_frames);

    let mut frames_encoded = 0;
    let mut packets_written = 0;

    // Encode frames
    for frame_num in 0..total_frames {
        // Generate test frame
        let frame = generate_test_frame(frame_num, WIDTH, HEIGHT);

        // Send frame to encoder
        encoder.send_frame(&frame)?;

        // Receive encoded packets
        loop {
            match encoder.receive_packet() {
                Ok(packet) => {
                    muxer.write_packet(&packet)?;
                    packets_written += 1;
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

    // Flush encoder
    println!("\n\nFlushing encoder...");
    encoder.flush()?;

    // Get remaining packets after flush
    loop {
        match encoder.receive_packet() {
            Ok(packet) => {
                muxer.write_packet(&packet)?;
                packets_written += 1;
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
    println!("  Frames encoded:  {}", frames_encoded);
    println!("  Packets written: {}", packets_written);
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

#[cfg(not(feature = "gpl-x264"))]
fn main() {
    eprintln!("This example requires the 'gpl-x264' feature.");
    eprintln!();
    eprintln!("Run with:");
    eprintln!("  cargo run -p rust_media_format --features gpl-x264 --example x264_to_mp4");
    eprintln!();
    eprintln!("Note: Enabling gpl-x264 changes the license of the compiled binary to GPL v2+.");
    std::process::exit(1);
}
