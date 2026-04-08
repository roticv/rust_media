//! Opus encode → decode roundtrip integration test
//!
//! This test validates the full Opus codec pipeline by:
//! 1. Generating a 1 kHz sine wave in pure Rust (no fixtures)
//! 2. Encoding it with `OpusEncoder`
//! 3. Decoding the resulting packets with `OpusDecoder`
//! 4. Verifying the decoded signal is still a 1 kHz sine wave with the right
//!    amplitude (within tolerance for lossy compression)
//!
//! No external test files are needed — the test source is generated entirely
//! from `tests/common/mod.rs`.

mod common;

use common::{
    estimate_frequency_zero_crossings, read_audio_samples_normalized, rms, sine_wave_frames,
};
use rust_media_codec::{OpusDecoder, OpusEncoder};
use rust_media_core::{
    AudioStreamParams, Decoder, Encoder, Error, MediaType, SampleFormat, StreamInfo, StreamParams,
};

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: usize = 2;
const FREQUENCY_HZ: f64 = 1000.0;
const AMPLITUDE: f64 = 0.5;
const DURATION_SECS: usize = 2;
const TOTAL_SAMPLES: usize = SAMPLE_RATE as usize * DURATION_SECS;

#[test]
fn opus_sine_wave_roundtrip_preserves_frequency_and_amplitude() {
    println!("\n=== Opus Sine Wave Roundtrip Test ===\n");

    // ------------------------------------------------------------------
    // 1. Generate input: 2 seconds of 1 kHz stereo sine wave at 48 kHz
    // ------------------------------------------------------------------
    let input_frames = sine_wave_frames(
        FREQUENCY_HZ,
        SAMPLE_RATE,
        CHANNELS,
        TOTAL_SAMPLES,
        960, // 20ms chunks at 48kHz (matches Opus frame size)
        AMPLITUDE,
    );
    println!(
        "Generated {} input frames ({} samples per channel total)",
        input_frames.len(),
        TOTAL_SAMPLES
    );

    // Verify the input is what we expect
    let input_samples_planar: Vec<f64> = input_frames
        .iter()
        .flat_map(read_audio_samples_normalized)
        .step_by(CHANNELS) // take only the first channel for analysis
        .collect();
    let input_freq = estimate_frequency_zero_crossings(&input_samples_planar, SAMPLE_RATE);
    let input_rms = rms(&input_samples_planar);
    println!(
        "Input:  freq={:.2} Hz, rms={:.4} (expected {:.4})",
        input_freq,
        input_rms,
        AMPLITUDE / 2.0_f64.sqrt()
    );
    assert!(
        (input_freq - FREQUENCY_HZ).abs() < 1.0,
        "input frequency should be ~{} Hz, got {}",
        FREQUENCY_HZ,
        input_freq
    );

    // ------------------------------------------------------------------
    // 2. Encode with OpusEncoder
    // ------------------------------------------------------------------
    let mut encoder = OpusEncoder::from_params(SAMPLE_RATE, CHANNELS, SampleFormat::S16)
        .expect("failed to create Opus encoder");

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

    // Flush
    encoder.flush().expect("encoder flush failed");
    loop {
        match encoder.receive_packet() {
            Ok(packet) => encoded_packets.push(packet),
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("encoder flush receive_packet failed: {:?}", e),
        }
    }

    println!("Encoded into {} Opus packets", encoded_packets.len());
    assert!(
        !encoded_packets.is_empty(),
        "encoder produced no packets"
    );

    // Total compressed size
    let compressed_bytes: usize = encoded_packets.iter().map(|p| p.size()).sum();
    let raw_bytes = TOTAL_SAMPLES * CHANNELS * 2; // 2 bytes per i16 sample
    println!(
        "Compression: {} bytes -> {} bytes ({:.1}% of original)",
        raw_bytes,
        compressed_bytes,
        100.0 * compressed_bytes as f64 / raw_bytes as f64
    );

    // ------------------------------------------------------------------
    // 3. Decode with OpusDecoder
    // ------------------------------------------------------------------
    let audio_params = AudioStreamParams::new(SAMPLE_RATE, CHANNELS, SampleFormat::S16);
    let stream_info = StreamInfo::new(0, MediaType::Audio, "opus".to_string())
        .with_params(StreamParams::Audio(audio_params))
        .with_time_base(1, SAMPLE_RATE);
    let mut decoder = OpusDecoder::new(stream_info).expect("failed to create Opus decoder");

    let mut decoded_samples_ch0: Vec<f64> = Vec::new();
    for packet in &encoded_packets {
        decoder.send_packet(packet).expect("decoder send_packet failed");
        loop {
            match decoder.receive_frame() {
                Ok(frame) => {
                    let samples = read_audio_samples_normalized(&frame);
                    // Take only the first channel for analysis
                    decoded_samples_ch0.extend(samples.iter().step_by(CHANNELS));
                }
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("decoder receive_frame failed: {:?}", e),
            }
        }
    }

    println!("Decoded {} samples per channel", decoded_samples_ch0.len());
    assert!(
        !decoded_samples_ch0.is_empty(),
        "decoder produced no samples"
    );

    // Allow for ~1% sample count difference (Opus has codec delay / lookahead)
    let sample_count_diff = (decoded_samples_ch0.len() as i64 - TOTAL_SAMPLES as i64).abs();
    let tolerance = (TOTAL_SAMPLES as f64 * 0.01) as i64;
    assert!(
        sample_count_diff <= tolerance,
        "decoded sample count {} differs from input {} by more than {}",
        decoded_samples_ch0.len(),
        TOTAL_SAMPLES,
        tolerance
    );

    // ------------------------------------------------------------------
    // 4. Verify the decoded signal is still a recognizable sine wave
    // ------------------------------------------------------------------

    // Skip the first 100ms to avoid Opus codec delay artifacts at the start
    let skip = (SAMPLE_RATE as usize) / 10;
    let analysis_samples = if decoded_samples_ch0.len() > skip {
        &decoded_samples_ch0[skip..]
    } else {
        &decoded_samples_ch0[..]
    };

    let decoded_freq = estimate_frequency_zero_crossings(analysis_samples, SAMPLE_RATE);
    let decoded_rms = rms(analysis_samples);
    println!(
        "Output: freq={:.2} Hz, rms={:.4}",
        decoded_freq, decoded_rms
    );

    // Frequency should be within 5 Hz of the input (very generous, Opus is good)
    assert!(
        (decoded_freq - FREQUENCY_HZ).abs() < 5.0,
        "decoded frequency {} Hz differs from input {} Hz by more than 5 Hz",
        decoded_freq,
        FREQUENCY_HZ
    );

    // RMS should be within 20% of input (Opus normalizes loudness slightly)
    let expected_rms = AMPLITUDE / 2.0_f64.sqrt();
    let rms_ratio = decoded_rms / expected_rms;
    assert!(
        rms_ratio > 0.8 && rms_ratio < 1.2,
        "decoded RMS {} differs from expected {} (ratio {:.2})",
        decoded_rms,
        expected_rms,
        rms_ratio
    );

    println!("\n✓ Opus roundtrip test passed");
}
