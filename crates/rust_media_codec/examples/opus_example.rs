//! Example demonstrating Opus codec usage
//!
//! This shows how to encode and decode audio using the Opus codec:
//! PCM audio → OpusEncoder → Packets → OpusDecoder → PCM audio

use rust_media_codec::{OpusDecoder, OpusEncoder};
use rust_media_core::{Decoder, Encoder, Frame, MediaType, SampleFormat, StreamInfo, StreamParams};

fn main() {
    println!("Opus Codec Example");
    println!("==================\n");

    // Step 1: Create Opus encoder
    let encoder = OpusEncoder::from_params(48000, 2, SampleFormat::S16)
        .expect("Failed to create Opus encoder");

    println!("✓ Opus encoder created");
    println!("  Sample rate: 48000 Hz");
    println!("  Channels: 2 (stereo)");
    println!("  Sample format: S16");
    println!();

    // Step 2: Create audio frame (20ms at 48kHz)
    let samples_per_channel = 960; // 20ms * 48kHz
    let mut frame = Frame::new_audio(48000, 2, SampleFormat::S16, samples_per_channel);

    // Generate a simple test tone (440 Hz sine wave)
    let frame_data = frame.plane_mut(0).expect("Failed to get frame data");
    for i in 0..samples_per_channel {
        // Generate sine wave for both channels
        let t = i as f32 / 48000.0;
        let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 16000.0;
        let sample_i16 = sample as i16;
        let bytes = sample_i16.to_le_bytes();

        // Left channel
        frame_data[i * 4] = bytes[0];
        frame_data[i * 4 + 1] = bytes[1];
        // Right channel
        frame_data[i * 4 + 2] = bytes[0];
        frame_data[i * 4 + 3] = bytes[1];
    }

    let frame = frame.with_pts(0).with_duration(20000); // 20ms in microseconds

    println!("✓ Created audio frame");
    println!("  Samples per channel: {}", samples_per_channel);
    println!("  Duration: 20ms");
    println!("  Data size: {} bytes", frame.plane(0).unwrap().len());
    println!();

    // Step 3: Encode to Opus
    let mut encoder = encoder;
    encoder.send_frame(&frame).expect("Failed to send frame");
    let packet = encoder.receive_packet().expect("Failed to receive packet");

    println!("✓ Encoded to Opus packet");
    println!("  Original size: {} bytes", frame.plane(0).unwrap().len());
    println!("  Compressed size: {} bytes", packet.size());
    println!("  Compression ratio: {:.1}x",
        frame.plane(0).unwrap().len() as f32 / packet.size() as f32);
    println!();

    // Step 4: Create Opus decoder
    let audio_params = rust_media_core::AudioStreamParams::new(48000, 2, SampleFormat::S16);
    let stream_info = StreamInfo::new(0, MediaType::Audio, "opus".to_string())
        .with_params(StreamParams::Audio(audio_params));

    let mut decoder = OpusDecoder::new(stream_info)
        .expect("Failed to create Opus decoder");

    println!("✓ Opus decoder created");
    println!();

    // Step 5: Decode from Opus
    decoder.send_packet(&packet).expect("Failed to send packet");
    let decoded_frame = decoder.receive_frame().expect("Failed to receive frame");

    println!("✓ Decoded from Opus packet");
    println!("  Decoded size: {} bytes", decoded_frame.plane(0).unwrap().len());
    println!("  PTS: {:?} μs", decoded_frame.pts());
    println!("  Duration: {:?} μs", decoded_frame.duration());
    println!();

    // Step 6: Verify the roundtrip
    let original_data = frame.plane(0).unwrap();
    let decoded_data = decoded_frame.plane(0).unwrap();

    // Calculate signal-to-noise ratio as a rough quality metric
    let mut signal_power = 0.0f64;
    let mut noise_power = 0.0f64;

    for i in (0..original_data.len().min(decoded_data.len())).step_by(2) {
        let original_sample = i16::from_le_bytes([original_data[i], original_data[i + 1]]) as f64;
        let decoded_sample = i16::from_le_bytes([decoded_data[i], decoded_data[i + 1]]) as f64;

        signal_power += original_sample * original_sample;
        let error = original_sample - decoded_sample;
        noise_power += error * error;
    }

    let snr = if noise_power > 0.0 {
        10.0 * (signal_power / noise_power).log10()
    } else {
        f64::INFINITY
    };

    println!("Quality metrics:");
    println!("  Signal-to-Noise Ratio: {:.1} dB", snr);
    println!("  (Higher is better, > 30 dB is good quality)");
    println!();

    println!("✅ Opus encode/decode roundtrip complete!");
    println!();
    println!("Summary:");
    println!("--------");
    println!("Opus is a lossy codec optimized for low-latency audio transmission.");
    println!("It provides excellent quality at low bitrates and is widely used in");
    println!("VoIP, video conferencing, and music streaming applications.");
}
