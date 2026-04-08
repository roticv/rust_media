//! AAC encode → decode roundtrip integration test (codec level, no muxing)
//!
//! Generates a sine wave, encodes it with FdkAacEncoder, decodes the resulting
//! raw AAC packets with FdkAacDecoder (using AudioSpecificConfig from the
//! encoder), and verifies the decoded signal preserves frequency and amplitude.
//!
//! This test only runs when the `fdk-aac` feature is enabled.

#![cfg(feature = "fdk-aac")]

mod common;

use common::{
    estimate_frequency_zero_crossings, read_audio_samples_normalized, rms, sine_wave_frames,
};
use rust_media_codec::{FdkAacDecoder, FdkAacEncoder};
use rust_media_core::{
    AudioStreamParams, Decoder, Encoder, Error, MediaType, SampleFormat, StreamInfo, StreamParams,
};

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: usize = 2;
const FREQUENCY_HZ: f64 = 1000.0;
const AMPLITUDE: f64 = 0.5;
const DURATION_SECS: usize = 2;
const TOTAL_SAMPLES: usize = SAMPLE_RATE as usize * DURATION_SECS;
const AAC_FRAME_SIZE: usize = 1024;
const BITRATE: u64 = 128_000;

#[test]
fn aac_sine_wave_roundtrip_preserves_frequency_and_amplitude() {
    println!("\n=== AAC Sine Wave Roundtrip Test ===\n");

    // ------------------------------------------------------------------
    // 1. Generate input
    // ------------------------------------------------------------------
    let input_frames = sine_wave_frames(
        FREQUENCY_HZ,
        SAMPLE_RATE,
        CHANNELS,
        TOTAL_SAMPLES,
        AAC_FRAME_SIZE,
        AMPLITUDE,
    );
    println!(
        "Generated {} input frames ({} samples per channel total)",
        input_frames.len(),
        TOTAL_SAMPLES
    );

    // ------------------------------------------------------------------
    // 2. Encode with FdkAacEncoder
    // ------------------------------------------------------------------
    let mut encoder = FdkAacEncoder::from_params(SAMPLE_RATE, CHANNELS, SampleFormat::S16, BITRATE)
        .expect("failed to create AAC encoder");

    // AAC needs AudioSpecificConfig for the decoder when using raw packets
    let asc = encoder.audio_specific_config().to_vec();
    println!("AudioSpecificConfig: {:?}", asc);
    assert!(!asc.is_empty(), "encoder should provide AudioSpecificConfig");

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

    println!("Encoded into {} AAC packets", encoded_packets.len());
    assert!(!encoded_packets.is_empty(), "encoder produced no packets");

    let raw_bytes = TOTAL_SAMPLES * CHANNELS * 2;
    let compressed_bytes: usize = encoded_packets.iter().map(|p| p.size()).sum();
    println!(
        "Compression: {} bytes -> {} bytes ({:.1}% of original)",
        raw_bytes,
        compressed_bytes,
        100.0 * compressed_bytes as f64 / raw_bytes as f64
    );

    // ------------------------------------------------------------------
    // 3. Decode with FdkAacDecoder using AudioSpecificConfig
    // ------------------------------------------------------------------
    let audio_params = AudioStreamParams::new(SAMPLE_RATE, CHANNELS, SampleFormat::S16);
    let mut stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
        .with_params(StreamParams::Audio(audio_params))
        .with_time_base(1, SAMPLE_RATE);
    stream_info.extra_data = asc;

    let mut decoder = FdkAacDecoder::new(stream_info).expect("failed to create AAC decoder");

    let mut decoded_samples_ch0: Vec<f64> = Vec::new();
    for packet in &encoded_packets {
        decoder.send_packet(packet).expect("decoder send_packet failed");
        loop {
            match decoder.receive_frame() {
                Ok(frame) => {
                    let samples = read_audio_samples_normalized(&frame);
                    decoded_samples_ch0.extend(samples.iter().step_by(CHANNELS));
                }
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("decoder receive_frame failed: {:?}", e),
            }
        }
    }

    decoder.flush().expect("decoder flush failed");
    loop {
        match decoder.receive_frame() {
            Ok(frame) => {
                let samples = read_audio_samples_normalized(&frame);
                decoded_samples_ch0.extend(samples.iter().step_by(CHANNELS));
            }
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("decoder flush receive_frame failed: {:?}", e),
        }
    }

    println!("Decoded {} samples per channel", decoded_samples_ch0.len());
    assert!(!decoded_samples_ch0.is_empty(), "decoder produced no samples");

    // ------------------------------------------------------------------
    // 4. Verify the decoded signal is still a sine wave
    // ------------------------------------------------------------------

    // AAC has codec delay (typically ~2048 samples). Skip the first 100ms.
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

    assert!(
        (decoded_freq - FREQUENCY_HZ).abs() < 5.0,
        "decoded frequency {} Hz differs from input {} Hz by more than 5 Hz",
        decoded_freq,
        FREQUENCY_HZ
    );

    let expected_rms = AMPLITUDE / 2.0_f64.sqrt();
    let rms_ratio = decoded_rms / expected_rms;
    assert!(
        rms_ratio > 0.8 && rms_ratio < 1.2,
        "decoded RMS {} differs from expected {} (ratio {:.2})",
        decoded_rms,
        expected_rms,
        rms_ratio
    );

    println!("\n✓ AAC roundtrip test passed");
}
