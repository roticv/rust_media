//! AV1 encode → decode roundtrip integration test (rav1e + dav1d)
//!
//! Generates a sequence of solid-color video frames with unique luma values,
//! encodes them with rav1e (Av1Encoder), decodes them back with dav1d
//! (Av1Decoder), and verifies that the decoded frames match the input within
//! a tolerance for lossy compression.
//!
//! No external test files are needed — all video content is generated in
//! pure Rust via `tests/common/mod.rs`.

mod common;

use common::{color_ramp_frames, mean_y};
use rust_media_codec::{Av1Decoder, Av1Encoder};
use rust_media_core::{
    Decoder, Encoder, Error, MediaType, PixelFormat, StreamInfo, StreamParams, VideoStreamParams,
};

const WIDTH: usize = 160;
const HEIGHT: usize = 120;
const FRAME_COUNT: usize = 8;
const TIME_BASE_DEN: u32 = 30; // 30 fps
const BITRATE: u64 = 500_000;

fn make_stream_info(width: usize, height: usize, time_base_den: u32) -> StreamInfo {
    let video_params = VideoStreamParams::new(width, height, PixelFormat::YUV420P);
    StreamInfo::new(0, MediaType::Video, "av1".to_string())
        .with_params(StreamParams::Video(video_params))
        .with_time_base(1, time_base_den)
}

#[test]
fn av1_color_ramp_roundtrip_preserves_per_frame_luma() {
    println!("\n=== AV1 Color Ramp Roundtrip Test ===\n");

    // ------------------------------------------------------------------
    // 1. Generate input
    // ------------------------------------------------------------------
    // Use a small frame count and small dimensions — rav1e at default speed 6
    // is slow enough that 30 frames at 320x240 would dominate the test suite.
    let mut input_frames = color_ramp_frames(WIDTH, HEIGHT, FRAME_COUNT);
    for (i, frame) in input_frames.iter_mut().enumerate() {
        frame.set_pts(Some(i as i64));
    }

    let input_y_values: Vec<f64> = input_frames.iter().map(mean_y).collect();
    println!(
        "Generated {} input frames (Y values: first={}, last={})",
        FRAME_COUNT,
        input_y_values[0],
        input_y_values[FRAME_COUNT - 1]
    );

    // ------------------------------------------------------------------
    // 2. Encode with Av1Encoder (rav1e)
    // ------------------------------------------------------------------
    let stream_info = make_stream_info(WIDTH, HEIGHT, TIME_BASE_DEN);
    let mut encoder = Av1Encoder::with_bitrate(stream_info.clone(), BITRATE)
        .expect("failed to create AV1 encoder");

    // The codec config should be available immediately, before any frames
    // have been pushed through the encoder.
    let av1c = encoder.codec_config();
    assert!(!av1c.is_empty(), "codec_config() returned empty bytes");
    // First byte of av1C is `marker (1) | version (7)` = 0x81.
    assert_eq!(
        av1c[0], 0x81,
        "av1C first byte should be marker+version 0x81, got {:#x}",
        av1c[0]
    );
    println!("av1C: {} bytes, first byte 0x{:02x}", av1c.len(), av1c[0]);

    let mut encoded_packets = Vec::new();
    for frame in &input_frames {
        encoder.send_frame(frame).expect("encoder send_frame failed");
        loop {
            match encoder.receive_packet() {
                Ok(packet) => encoded_packets.push(packet),
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("encoder receive_packet failed: {:?}", e),
            }
        }
    }

    encoder.flush().expect("encoder flush failed");
    loop {
        match encoder.receive_packet() {
            Ok(packet) => encoded_packets.push(packet),
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("encoder flush receive_packet failed: {:?}", e),
        }
    }

    println!("Encoded into {} AV1 packets", encoded_packets.len());
    assert!(!encoded_packets.is_empty(), "encoder produced no packets");
    assert_eq!(
        encoded_packets.len(),
        FRAME_COUNT,
        "encoded packet count should match input frame count"
    );

    let raw_bytes = WIDTH * HEIGHT * 3 / 2 * FRAME_COUNT;
    let compressed_bytes: usize = encoded_packets.iter().map(|p| p.size()).sum();
    println!(
        "Compression: {} bytes -> {} bytes ({:.1}% of original)",
        raw_bytes,
        compressed_bytes,
        100.0 * compressed_bytes as f64 / raw_bytes as f64
    );

    assert!(
        encoded_packets[0].is_keyframe(),
        "first AV1 packet should be a keyframe"
    );

    // ------------------------------------------------------------------
    // 3. Decode with Av1Decoder (dav1d)
    // ------------------------------------------------------------------
    let mut decoder = Av1Decoder::new(stream_info).expect("failed to create AV1 decoder");

    let mut decoded_frames = Vec::new();
    for packet in &encoded_packets {
        decoder
            .send_packet(packet)
            .expect("decoder send_packet failed");
        loop {
            match decoder.receive_frame() {
                Ok(frame) => decoded_frames.push(frame),
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("decoder receive_frame failed: {:?}", e),
            }
        }
    }

    decoder.flush().expect("decoder flush failed");
    loop {
        match decoder.receive_frame() {
            Ok(frame) => decoded_frames.push(frame),
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("decoder flush receive_frame failed: {:?}", e),
        }
    }

    println!("Decoded {} frames", decoded_frames.len());

    // ------------------------------------------------------------------
    // 4. Validate frame count, dimensions, and per-frame luma
    // ------------------------------------------------------------------
    assert_eq!(
        decoded_frames.len(),
        FRAME_COUNT,
        "decoded frame count mismatch"
    );

    for (i, frame) in decoded_frames.iter().enumerate() {
        let params = frame.video_params().expect("not a video frame");
        assert_eq!(params.width, WIDTH, "frame {} width mismatch", i);
        assert_eq!(params.height, HEIGHT, "frame {} height mismatch", i);
        assert_eq!(
            params.format,
            PixelFormat::YUV420P,
            "frame {} format mismatch",
            i
        );
    }

    // Per-frame mean Y should match the input. AV1 at 500 kbps on small
    // flat-color frames is essentially lossless for the luma DC component.
    // Allow ±5 luma units of tolerance to be safe.
    let mut max_diff: f64 = 0.0;
    for (i, (input_y, decoded_frame)) in
        input_y_values.iter().zip(decoded_frames.iter()).enumerate()
    {
        let decoded_y = mean_y(decoded_frame);
        let diff = (input_y - decoded_y).abs();
        if diff > max_diff {
            max_diff = diff;
        }
        assert!(
            diff < 5.0,
            "frame {} mean Y mismatch: input={} decoded={} diff={}",
            i,
            input_y,
            decoded_y,
            diff
        );
    }
    println!("Max per-frame Y difference: {:.2}", max_diff);

    println!("\n✓ AV1 roundtrip test passed");
}
