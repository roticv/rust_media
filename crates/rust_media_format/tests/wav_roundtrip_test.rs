//! Integration test for WAV roundtrip: PCM → WAV Muxer → WAV Demuxer → PCM
//!
//! This test validates that we can write PCM data to WAV and read it back correctly.

use rust_media_core::{
    AudioStreamParams, Demuxer, MediaType, Muxer, Packet, SampleFormat, StreamInfo, StreamParams,
};
use rust_media_format::{WavDemuxer, WavMuxer};
use std::io::Cursor;

#[test]
fn test_wav_muxer_demuxer_roundtrip() {
    println!("\n=== WAV Roundtrip Test ===\n");

    // Create test PCM data (sine wave-like pattern)
    let sample_rate = 48000;
    let channels: usize = 2;
    let duration_seconds = 1;
    let num_samples = sample_rate as usize * channels * duration_seconds;

    let mut pcm_data = Vec::new();
    for i in 0..num_samples {
        // Create a simple pattern (not a real sine wave, just test data)
        let value = ((i % 256) as i16) * 100;
        pcm_data.extend_from_slice(&value.to_le_bytes());
    }

    let original_size = pcm_data.len();
    println!("Original PCM data: {} bytes", original_size);

    // Step 1: Create WAV file using muxer
    println!("\nWriting WAV file...");
    let mut wav_buffer = Cursor::new(Vec::new());

    {
        let mut muxer = WavMuxer::new(&mut wav_buffer);

        // Add stream
        let audio_params = AudioStreamParams::new(sample_rate, channels, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(StreamParams::Audio(audio_params));

        muxer.add_stream(stream_info).expect("Failed to add stream");
        muxer.write_header().expect("Failed to write header");

        // Write PCM data in chunks (simulating streaming)
        let chunk_size = 4096;
        for chunk in pcm_data.chunks(chunk_size) {
            let packet = Packet::new(chunk.to_vec(), 0, MediaType::Audio);
            muxer.write_packet(&packet).expect("Failed to write packet");
        }

        muxer.write_trailer().expect("Failed to write trailer");
        muxer.flush().expect("Failed to flush");
    }

    let wav_data = wav_buffer.into_inner();
    println!("  WAV file size: {} bytes", wav_data.len());
    println!("  WAV header overhead: {} bytes", wav_data.len() - original_size);

    // Verify WAV structure
    assert_eq!(&wav_data[0..4], b"RIFF", "Should have RIFF header");
    assert_eq!(&wav_data[8..12], b"WAVE", "Should have WAVE format");
    assert_eq!(&wav_data[12..16], b"fmt ", "Should have fmt chunk");
    assert_eq!(&wav_data[36..40], b"data", "Should have data chunk");

    // Step 2: Read WAV file back using demuxer
    println!("\nReading WAV file...");
    let cursor = Cursor::new(wav_data);
    let mut demuxer = WavDemuxer::open(cursor).expect("Failed to open WAV demuxer");

    // Verify container info
    let container_info = demuxer
        .container_info()
        .expect("Failed to get container info");
    assert_eq!(container_info.format_name, "wav");
    println!("  Container format: {}", container_info.format_name);

    // Verify stream info
    let streams = demuxer.streams().expect("Failed to get streams");
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].codec, "pcm");

    if let StreamParams::Audio(params) = &streams[0].params {
        assert_eq!(params.sample_rate, sample_rate);
        assert_eq!(params.channels, channels);
        assert_eq!(params.sample_format, SampleFormat::S16);
        println!(
            "  Audio params: {} Hz, {} channels, {:?}",
            params.sample_rate, params.channels, params.sample_format
        );
    } else {
        panic!("Expected audio stream");
    }

    // Step 3: Read all packets and reconstruct PCM data
    println!("\nReading PCM packets...");
    let mut reconstructed_pcm = Vec::new();
    let mut packet_count = 0;

    loop {
        match demuxer.read_packet() {
            Ok(packet) => {
                packet_count += 1;
                reconstructed_pcm.extend_from_slice(packet.data());
            }
            Err(rust_media_core::Error::EndOfStream) => break,
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    println!("  Read {} packets", packet_count);
    println!("  Reconstructed PCM: {} bytes", reconstructed_pcm.len());

    // Step 4: Verify reconstructed data matches original
    assert_eq!(
        reconstructed_pcm.len(),
        original_size,
        "Reconstructed PCM size should match original"
    );

    assert_eq!(
        reconstructed_pcm, pcm_data,
        "Reconstructed PCM data should exactly match original"
    );

    println!("\n✅ WAV roundtrip test passed!");
    println!("   Original → WAV Muxer → WAV Demuxer → Reconstructed");
    println!("   Data integrity: 100% match");
}

#[test]
fn test_wav_muxer_different_formats() {
    // Test S16 format
    {
        let mut buffer = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(&mut buffer);

        let audio_params = AudioStreamParams::new(44100, 1, SampleFormat::S16);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_params(StreamParams::Audio(audio_params));

        muxer.add_stream(stream_info).unwrap();
        muxer.write_header().unwrap();
        muxer.write_trailer().unwrap();

        let wav_data = buffer.into_inner();
        assert_eq!(&wav_data[0..4], b"RIFF");
    }

    // Test S32 format
    {
        let mut buffer = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(&mut buffer);

        let audio_params = AudioStreamParams::new(48000, 2, SampleFormat::S32);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_params(StreamParams::Audio(audio_params));

        muxer.add_stream(stream_info).unwrap();
        muxer.write_header().unwrap();
        muxer.write_trailer().unwrap();

        let wav_data = buffer.into_inner();
        assert_eq!(&wav_data[0..4], b"RIFF");
    }

    // Test U8 format
    {
        let mut buffer = Cursor::new(Vec::new());
        let mut muxer = WavMuxer::new(&mut buffer);

        let audio_params = AudioStreamParams::new(22050, 1, SampleFormat::U8);
        let stream_info = StreamInfo::new(0, MediaType::Audio, "pcm".to_string())
            .with_params(StreamParams::Audio(audio_params));

        muxer.add_stream(stream_info).unwrap();
        muxer.write_header().unwrap();
        muxer.write_trailer().unwrap();

        let wav_data = buffer.into_inner();
        assert_eq!(&wav_data[0..4], b"RIFF");
    }
}
