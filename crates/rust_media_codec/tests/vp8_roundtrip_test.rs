//! VP8 encode → decode roundtrip integration test
//!
//! Generates a sequence of solid-color video frames with unique luma values,
//! encodes them with VP8, decodes them back, and verifies that each decoded
//! frame matches its corresponding input within a tolerance for lossy
//! compression.
//!
//! No external test files are needed — all video content is generated in
//! pure Rust via `tests/common/mod.rs`.

mod common;

use common::{color_ramp_frames, mean_y};
use rust_media_codec::{Vp8Decoder, Vp8Encoder};
use rust_media_core::{
    Decoder, Encoder, Error, MediaType, PixelFormat, StreamInfo, StreamParams, VideoStreamParams,
};

const WIDTH: usize = 320;
const HEIGHT: usize = 240;
const FRAME_COUNT: usize = 30;
const TIME_BASE_DEN: u32 = 30; // 30 fps
const BITRATE: u64 = 500_000;

fn make_stream_info(width: usize, height: usize, time_base_den: u32) -> StreamInfo {
    let video_params = VideoStreamParams::new(width, height, PixelFormat::YUV420P);
    StreamInfo::new(0, MediaType::Video, "vp8".to_string())
        .with_params(StreamParams::Video(video_params))
        .with_time_base(1, time_base_den)
}

#[test]
fn vp8_color_ramp_roundtrip_preserves_per_frame_luma() {
    println!("\n=== VP8 Color Ramp Roundtrip Test ===\n");

    // ------------------------------------------------------------------
    // 1. Generate input: 30 frames with unique Y values
    // ------------------------------------------------------------------
    let mut input_frames = color_ramp_frames(WIDTH, HEIGHT, FRAME_COUNT);
    // Assign monotonically increasing PTS so the encoder/decoder pipeline
    // can preserve ordering.
    for (i, frame) in input_frames.iter_mut().enumerate() {
        frame.set_pts(Some(i as i64));
    }

    let input_y_values: Vec<f64> = input_frames.iter().map(mean_y).collect();
    println!(
        "Generated {} input frames (Y values: first={}, last={})",
        FRAME_COUNT, input_y_values[0], input_y_values[FRAME_COUNT - 1]
    );

    // ------------------------------------------------------------------
    // 2. Encode with Vp8Encoder
    // ------------------------------------------------------------------
    let stream_info = make_stream_info(WIDTH, HEIGHT, TIME_BASE_DEN);
    let mut encoder = Vp8Encoder::with_bitrate(stream_info.clone(), BITRATE)
        .expect("failed to create VP8 encoder");

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

    println!("Encoded into {} VP8 packets", encoded_packets.len());
    assert!(!encoded_packets.is_empty(), "encoder produced no packets");

    let raw_bytes = WIDTH * HEIGHT * 3 / 2 * FRAME_COUNT;
    let compressed_bytes: usize = encoded_packets.iter().map(|p| p.size()).sum();
    println!(
        "Compression: {} bytes -> {} bytes ({:.1}% of original)",
        raw_bytes,
        compressed_bytes,
        100.0 * compressed_bytes as f64 / raw_bytes as f64
    );

    // First packet should be a keyframe
    assert!(
        encoded_packets[0].is_keyframe(),
        "first VP8 packet should be a keyframe"
    );

    // ------------------------------------------------------------------
    // 3. Decode with Vp8Decoder
    // ------------------------------------------------------------------
    let mut decoder = Vp8Decoder::new(stream_info).expect("failed to create VP8 decoder");

    let mut decoded_frames = Vec::new();
    for packet in &encoded_packets {
        decoder.send_packet(packet).expect("decoder send_packet failed");
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
    // 4. Validate frame count and per-frame luma
    // ------------------------------------------------------------------
    assert_eq!(
        decoded_frames.len(),
        FRAME_COUNT,
        "decoded frame count mismatch"
    );

    // Check dimensions match
    for (i, frame) in decoded_frames.iter().enumerate() {
        let params = frame.video_params().expect("not a video frame");
        assert_eq!(params.width, WIDTH, "frame {} width mismatch", i);
        assert_eq!(params.height, HEIGHT, "frame {} height mismatch", i);
    }

    // Verify each decoded frame's mean Y is close to the input
    // VP8 with VBR @ 500kbps on flat-color frames should be near-perfect.
    // Allow ±5 luma units of tolerance.
    let mut max_diff: f64 = 0.0;
    for (i, (input_y, decoded_frame)) in input_y_values.iter().zip(decoded_frames.iter()).enumerate() {
        let decoded_y = mean_y(decoded_frame);
        let diff = (input_y - decoded_y).abs();
        if diff > max_diff {
            max_diff = diff;
        }
        assert!(
            diff < 5.0,
            "frame {} mean Y mismatch: input={} decoded={} diff={}",
            i, input_y, decoded_y, diff
        );
    }
    println!("Max per-frame Y difference: {:.2}", max_diff);

    println!("\n✓ VP8 roundtrip test passed");
}
