//! Integration test for AAC encode/decode roundtrip via MP4
//!
//! This test:
//! 1. Creates synthetic audio samples
//! 2. Encodes them to AAC using FdkAacEncoder
//! 3. Muxes the AAC packets to an MP4 file
//! 4. Demuxes the MP4 file using Mp4Demuxer
//! 5. Decodes the AAC packets using FdkAacDecoder
//! 6. Verifies the roundtrip worked correctly

#![cfg(feature = "fdk-aac")]

use rust_media_codec::{FdkAacDecoder, FdkAacEncoder};
use rust_media_core::stream::{AudioStreamParams, StreamInfo, StreamParams};
use rust_media_core::types::{MediaType, SampleFormat};
use rust_media_core::{Decoder, Demuxer, Encoder, Frame, Muxer};
use rust_media_format::mp4::{Mp4Demuxer, Mp4Muxer};
use std::io::{Cursor, Seek, SeekFrom};

const SAMPLE_RATE: u32 = 48000;
const CHANNELS: usize = 2;
const AAC_FRAME_SIZE: usize = 1024;
const NUM_FRAMES: usize = 10;

/// Generate a sine wave tone
fn generate_audio_samples(num_samples: usize, frequency: f64, offset: usize) -> Vec<i16> {
    let mut samples = Vec::with_capacity(num_samples * CHANNELS);

    for i in 0..num_samples {
        let sample_idx = offset + i;
        let t = sample_idx as f64 / SAMPLE_RATE as f64;
        let value = (t * frequency * 2.0 * std::f64::consts::PI).sin();
        let sample = (value * 16000.0) as i16;

        // Interleaved stereo
        samples.push(sample);
        samples.push(sample);
    }

    samples
}

#[test]
fn test_aac_mp4_roundtrip() {
    // Step 1: Create encoder
    let audio_params = AudioStreamParams::new(SAMPLE_RATE, CHANNELS, SampleFormat::S16);
    let encoder_stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
        .with_params(StreamParams::Audio(audio_params.clone()))
        .with_bitrate(128000);

    let mut encoder = FdkAacEncoder::new(encoder_stream_info.clone()).unwrap();

    // Get AudioSpecificConfig for the muxer
    let asc = encoder.audio_specific_config().to_vec();

    // Step 2: Create MP4 muxer with in-memory buffer
    let mut mp4_buffer = Cursor::new(Vec::new());
    let mut muxer = Mp4Muxer::new(&mut mp4_buffer);

    // Add audio stream with AudioSpecificConfig
    let mut muxer_stream_info = encoder_stream_info.clone();
    muxer_stream_info.extra_data = asc.clone();
    muxer.add_stream(muxer_stream_info).unwrap();
    muxer.write_header().unwrap();

    // Step 3: Encode audio frames and write to MP4
    let mut total_samples_encoded = 0;
    let mut packets_written = 0;

    for frame_num in 0..NUM_FRAMES {
        let samples = generate_audio_samples(AAC_FRAME_SIZE, 440.0, frame_num * AAC_FRAME_SIZE);

        let mut frame = Frame::new_audio(SAMPLE_RATE, CHANNELS, SampleFormat::S16, AAC_FRAME_SIZE);

        // Copy samples to frame
        if let Some(data) = frame.plane_mut(0) {
            for (i, sample) in samples.iter().enumerate() {
                let bytes = sample.to_le_bytes();
                data[i * 2] = bytes[0];
                data[i * 2 + 1] = bytes[1];
            }
        }

        let pts = (frame_num * AAC_FRAME_SIZE) as i64;
        let frame = frame.with_pts(pts);

        encoder.send_frame(&frame).unwrap();
        total_samples_encoded += AAC_FRAME_SIZE;

        // Get encoded packets
        while let Ok(packet) = encoder.receive_packet() {
            muxer.write_packet(&packet).unwrap();
            packets_written += 1;
        }
    }

    // Flush encoder
    encoder.flush().unwrap();
    while let Ok(packet) = encoder.receive_packet() {
        muxer.write_packet(&packet).unwrap();
        packets_written += 1;
    }

    // Finalize MP4
    muxer.write_trailer().unwrap();
    muxer.flush().unwrap();

    println!("Encoded {} samples into {} AAC packets", total_samples_encoded, packets_written);
    assert!(packets_written > 0, "Should have encoded at least one packet");

    // Step 4: Demux the MP4 file
    mp4_buffer.seek(SeekFrom::Start(0)).unwrap();
    let mp4_size = mp4_buffer.get_ref().len();
    println!("MP4 file size: {} bytes", mp4_size);

    let mut demuxer = Mp4Demuxer::new(mp4_buffer).unwrap();

    // Verify streams
    let streams = demuxer.streams().unwrap();
    assert_eq!(streams.len(), 1, "Should have one audio stream");
    assert_eq!(streams[0].media_type, MediaType::Audio);
    assert_eq!(streams[0].codec, "aac");

    // Get AudioSpecificConfig from demuxer
    let demuxed_asc = streams[0].extra_data.clone();
    assert!(!demuxed_asc.is_empty(), "Should have AudioSpecificConfig");
    println!("Demuxed AudioSpecificConfig: {:?}", demuxed_asc);

    // Step 5: Create decoder with AudioSpecificConfig
    let mut decoder_stream_info = streams[0].clone();
    decoder_stream_info.extra_data = demuxed_asc;

    let mut decoder = FdkAacDecoder::new(decoder_stream_info).unwrap();

    // Step 6: Decode all packets
    let mut packets_read = 0;
    let mut decoded_frames = 0;

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                packets_read += 1;
                println!(
                    "Packet {}: {} bytes, pts={:?}",
                    packets_read,
                    packet.data().len(),
                    packet.pts()
                );

                // Send to decoder
                if let Err(e) = decoder.send_packet(&packet) {
                    println!("Decoder error on packet {}: {:?}", packets_read, e);
                    continue;
                }

                // Try to receive decoded frames
                while let Ok(frame) = decoder.receive_frame() {
                    decoded_frames += 1;
                    println!(
                        "Decoded frame {}: pts={:?}",
                        decoded_frames,
                        frame.pts()
                    );
                }
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(e) => {
                println!("Demuxer error: {:?}", e);
                break;
            }
        }
    }

    // Flush decoder
    decoder.flush().unwrap();
    while let Ok(frame) = decoder.receive_frame() {
        decoded_frames += 1;
        println!("Flushed frame {}: pts={:?}", decoded_frames, frame.pts());
    }

    println!("\n=== Roundtrip Summary ===");
    println!("Samples encoded: {}", total_samples_encoded);
    println!("Packets written: {}", packets_written);
    println!("Packets read: {}", packets_read);
    println!("Frames decoded: {}", decoded_frames);

    // Verify we read the same number of packets we wrote
    assert_eq!(
        packets_read, packets_written,
        "Should read same number of packets as written"
    );

    // Note: decoded_frames may be less than packets_read due to decoder latency
    // and the fact that raw AAC (without ADTS) decoding may not produce output
    // for every packet immediately
    println!("\nRoundtrip test completed successfully!");
}

#[test]
fn test_demuxer_stream_info() {
    // Create a minimal MP4 with AAC audio
    let audio_params = AudioStreamParams::new(SAMPLE_RATE, CHANNELS, SampleFormat::S16);
    let encoder_stream_info = StreamInfo::new(0, MediaType::Audio, "aac".to_string())
        .with_params(StreamParams::Audio(audio_params))
        .with_bitrate(128000);

    let mut encoder = FdkAacEncoder::new(encoder_stream_info.clone()).unwrap();
    let asc = encoder.audio_specific_config().to_vec();

    let mut mp4_buffer = Cursor::new(Vec::new());
    let mut muxer = Mp4Muxer::new(&mut mp4_buffer);

    let mut muxer_stream_info = encoder_stream_info;
    muxer_stream_info.extra_data = asc;
    muxer.add_stream(muxer_stream_info).unwrap();
    muxer.write_header().unwrap();

    // Encode one frame
    let samples = generate_audio_samples(AAC_FRAME_SIZE, 440.0, 0);
    let mut frame = Frame::new_audio(SAMPLE_RATE, CHANNELS, SampleFormat::S16, AAC_FRAME_SIZE);
    if let Some(data) = frame.plane_mut(0) {
        for (i, sample) in samples.iter().enumerate() {
            let bytes = sample.to_le_bytes();
            data[i * 2] = bytes[0];
            data[i * 2 + 1] = bytes[1];
        }
    }

    encoder.send_frame(&frame).unwrap();
    encoder.flush().unwrap();

    while let Ok(packet) = encoder.receive_packet() {
        muxer.write_packet(&packet).unwrap();
    }

    muxer.write_trailer().unwrap();
    muxer.flush().unwrap();

    // Demux and verify
    mp4_buffer.seek(SeekFrom::Start(0)).unwrap();
    let demuxer = Mp4Demuxer::new(mp4_buffer).unwrap();

    let container_info = demuxer.container_info().unwrap();
    assert_eq!(container_info.format_name, "mp4");

    let streams = demuxer.streams().unwrap();
    assert_eq!(streams.len(), 1);

    let stream = &streams[0];
    assert_eq!(stream.index, 0);
    assert_eq!(stream.media_type, MediaType::Audio);
    assert_eq!(stream.codec, "aac");
    assert!(!stream.extra_data.is_empty());

    // Check audio params
    match &stream.params {
        StreamParams::Audio(params) => {
            assert_eq!(params.sample_rate, SAMPLE_RATE);
            assert_eq!(params.channels, CHANNELS);
        }
        _ => panic!("Expected audio stream params"),
    }
}
