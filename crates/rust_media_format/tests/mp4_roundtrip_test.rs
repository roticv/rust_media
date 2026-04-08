//! End-to-end MP4 roundtrip integration test
//!
//! This test exercises the **full pipeline** that codec-only tests skip:
//!
//! ```text
//! generate frames → encode → mux → MP4 bytes →
//! demux → packets → decode → verify content
//! ```
//!
//! It validates that:
//! - The MP4 muxer correctly writes sample tables (stts, stsc, stsz, stco, stss)
//! - Codec configuration records (vpcC for VP9, dOps for Opus) are written/read correctly
//! - Stream metadata round-trips through the container
//! - Decoded content matches the original input within codec tolerance
//!
//! Uses VP9 video and Opus audio (both available with default features), with
//! all test content generated in pure Rust via `tests/common/mod.rs`. No
//! external fixtures or ffmpeg dependency.

mod common;

use common::{
    color_ramp_frames, estimate_frequency_zero_crossings, mean_y,
    read_audio_samples_normalized, rms, sine_wave_frames,
};
use rust_media_codec::{OpusDecoder, OpusEncoder, Vp9Decoder, Vp9Encoder};
use rust_media_core::{
    AudioStreamParams, Decoder, Demuxer, Encoder, Error, Frame, MediaType, Muxer, PixelFormat,
    SampleFormat, StreamInfo, StreamParams, VideoStreamParams,
};
use rust_media_format::{Mp4Demuxer, Mp4Muxer};
use std::io::Cursor;

// Video config
const WIDTH: usize = 320;
const HEIGHT: usize = 240;
const FRAME_COUNT: usize = 30;
const VIDEO_TIMEBASE_DEN: u32 = 30; // 30 fps
const VIDEO_BITRATE: u64 = 500_000;

// Audio config
const SAMPLE_RATE: u32 = 48000;
const CHANNELS: usize = 2;
const FREQUENCY_HZ: f64 = 1000.0;
const AMPLITUDE: f64 = 0.5;
const AUDIO_DURATION_SECS: usize = 1;
const TOTAL_AUDIO_SAMPLES: usize = SAMPLE_RATE as usize * AUDIO_DURATION_SECS;
const OPUS_FRAME_SAMPLES: usize = 960; // 20ms at 48kHz

#[test]
fn mp4_vp9_opus_full_pipeline_roundtrip() {
    println!("\n=== MP4 Full Pipeline Roundtrip Test ===\n");
    println!("Pipeline: generate → encode → mux → demux → decode → verify");

    // ------------------------------------------------------------------
    // 1. Generate input content
    // ------------------------------------------------------------------
    let mut video_frames = color_ramp_frames(WIDTH, HEIGHT, FRAME_COUNT);
    for (i, frame) in video_frames.iter_mut().enumerate() {
        frame.set_pts(Some(i as i64));
    }
    let input_y_values: Vec<f64> = video_frames.iter().map(mean_y).collect();
    println!(
        "\n[1] Generated {} video frames @ {}x{} ({} fps)",
        FRAME_COUNT, WIDTH, HEIGHT, VIDEO_TIMEBASE_DEN
    );

    let audio_frames = sine_wave_frames(
        FREQUENCY_HZ,
        SAMPLE_RATE,
        CHANNELS,
        TOTAL_AUDIO_SAMPLES,
        OPUS_FRAME_SAMPLES,
        AMPLITUDE,
    );
    println!(
        "    Generated {} audio frames ({} samples @ {} Hz, {} ch)",
        audio_frames.len(),
        TOTAL_AUDIO_SAMPLES,
        SAMPLE_RATE,
        CHANNELS
    );

    // ------------------------------------------------------------------
    // 2. Encode video and audio
    // ------------------------------------------------------------------
    let video_stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
        .with_params(StreamParams::Video(VideoStreamParams::new(
            WIDTH,
            HEIGHT,
            PixelFormat::YUV420P,
        )))
        .with_time_base(1, VIDEO_TIMEBASE_DEN)
        .with_bitrate(VIDEO_BITRATE);

    let audio_stream_info = StreamInfo::new(1, MediaType::Audio, "opus".to_string())
        .with_params(StreamParams::Audio(AudioStreamParams::new(
            SAMPLE_RATE,
            CHANNELS,
            SampleFormat::S16,
        )))
        .with_time_base(1, SAMPLE_RATE);

    let mut video_encoder =
        Vp9Encoder::with_bitrate(video_stream_info.clone(), VIDEO_BITRATE).unwrap();
    let video_packets = encode_all_video(&mut video_encoder, &video_frames);
    println!(
        "\n[2] Encoded {} video frames into {} VP9 packets",
        FRAME_COUNT,
        video_packets.len()
    );

    let mut audio_encoder = OpusEncoder::from_params(SAMPLE_RATE, CHANNELS, SampleFormat::S16).unwrap();
    let audio_packets = encode_all_audio(&mut audio_encoder, &audio_frames);
    println!(
        "    Encoded {} audio frames into {} Opus packets",
        audio_frames.len(),
        audio_packets.len()
    );

    // ------------------------------------------------------------------
    // 3. Mux into MP4 (in-memory)
    // ------------------------------------------------------------------
    let mut mp4_buffer = Cursor::new(Vec::new());
    {
        let mut muxer = Mp4Muxer::new(&mut mp4_buffer);

        // Add streams (assign output indices 0=video, 1=audio)
        let mut video_out = video_stream_info.clone();
        video_out.index = 0;
        muxer.add_stream(video_out).unwrap();

        let mut audio_out = audio_stream_info.clone();
        audio_out.index = 1;
        muxer.add_stream(audio_out).unwrap();

        muxer.write_header().unwrap();

        // Write all video packets
        for mut pkt in video_packets.iter().cloned() {
            pkt.set_stream_index(0);
            muxer.write_packet(&pkt).unwrap();
        }

        // Write all audio packets
        for mut pkt in audio_packets.iter().cloned() {
            pkt.set_stream_index(1);
            muxer.write_packet(&pkt).unwrap();
        }

        muxer.write_trailer().unwrap();
        muxer.flush().unwrap();
    }

    let mp4_size = mp4_buffer.get_ref().len();
    println!("\n[3] Muxed into MP4: {} bytes", mp4_size);
    assert!(mp4_size > 0);

    // Sanity check: starts with ftyp box
    let bytes = mp4_buffer.get_ref();
    assert_eq!(&bytes[4..8], b"ftyp", "MP4 should start with ftyp box");

    // ------------------------------------------------------------------
    // 4. Demux the MP4
    // ------------------------------------------------------------------
    mp4_buffer.set_position(0);
    let mut demuxer = Mp4Demuxer::new(mp4_buffer).unwrap();

    let streams = demuxer.streams().unwrap();
    println!("\n[4] Demuxed MP4: {} streams", streams.len());
    assert_eq!(streams.len(), 2, "should have video + audio streams");

    // Verify stream metadata round-tripped correctly
    let video_stream = streams.iter().find(|s| s.media_type == MediaType::Video).unwrap();
    assert_eq!(video_stream.codec, "vp9", "video codec should be vp9");
    if let StreamParams::Video(ref vp) = video_stream.params {
        assert_eq!(vp.width, WIDTH);
        assert_eq!(vp.height, HEIGHT);
    } else {
        panic!("video stream missing video params");
    }

    let audio_stream = streams.iter().find(|s| s.media_type == MediaType::Audio).unwrap();
    assert_eq!(audio_stream.codec, "opus", "audio codec should be opus");
    if let StreamParams::Audio(ref ap) = audio_stream.params {
        assert_eq!(ap.sample_rate, SAMPLE_RATE);
        assert_eq!(ap.channels, CHANNELS);
    } else {
        panic!("audio stream missing audio params");
    }
    println!("    Video: {} {}x{}", video_stream.codec, WIDTH, HEIGHT);
    println!("    Audio: {} {} Hz, {} ch", audio_stream.codec, SAMPLE_RATE, CHANNELS);

    let video_idx = video_stream.index;
    let audio_idx = audio_stream.index;
    let video_stream_clone = video_stream.clone();
    let audio_stream_clone = audio_stream.clone();

    // Read all packets, separated by stream
    let mut demuxed_video_packets = Vec::new();
    let mut demuxed_audio_packets = Vec::new();
    loop {
        match demuxer.read_packet() {
            Ok(pkt) => {
                if pkt.stream_index() == video_idx {
                    demuxed_video_packets.push(pkt);
                } else if pkt.stream_index() == audio_idx {
                    demuxed_audio_packets.push(pkt);
                }
            }
            Err(Error::EndOfStream) => break,
            Err(e) => panic!("demuxer read_packet failed: {:?}", e),
        }
    }
    println!(
        "    Read {} video packets, {} audio packets",
        demuxed_video_packets.len(),
        demuxed_audio_packets.len()
    );

    assert_eq!(
        demuxed_video_packets.len(),
        video_packets.len(),
        "demuxed video packet count should match what was muxed"
    );
    assert_eq!(
        demuxed_audio_packets.len(),
        audio_packets.len(),
        "demuxed audio packet count should match what was muxed"
    );

    // ------------------------------------------------------------------
    // 5. Decode video and audio
    // ------------------------------------------------------------------
    let mut video_decoder = Vp9Decoder::new(video_stream_clone).unwrap();
    let decoded_video_frames = decode_all_video(&mut video_decoder, &demuxed_video_packets);
    println!(
        "\n[5] Decoded {} video frames",
        decoded_video_frames.len()
    );

    let mut audio_decoder = OpusDecoder::new(audio_stream_clone).unwrap();
    let decoded_audio_samples_ch0 = decode_all_audio_ch0(&mut audio_decoder, &demuxed_audio_packets);
    println!("    Decoded {} audio samples (channel 0)", decoded_audio_samples_ch0.len());

    // ------------------------------------------------------------------
    // 6. Verify content matches input
    // ------------------------------------------------------------------
    println!("\n[6] Verifying decoded content...");

    // Video: frame count and per-frame luma
    assert_eq!(
        decoded_video_frames.len(),
        FRAME_COUNT,
        "decoded video frame count mismatch"
    );

    let mut max_y_diff: f64 = 0.0;
    for (i, (input_y, decoded_frame)) in input_y_values
        .iter()
        .zip(decoded_video_frames.iter())
        .enumerate()
    {
        let decoded_y = mean_y(decoded_frame);
        let diff = (input_y - decoded_y).abs();
        if diff > max_y_diff {
            max_y_diff = diff;
        }
        assert!(
            diff < 5.0,
            "video frame {} mean Y mismatch: input={} decoded={} diff={}",
            i, input_y, decoded_y, diff
        );
    }
    println!("    Video: max per-frame Y diff = {:.2}", max_y_diff);

    // Audio: skip codec delay, verify frequency + RMS
    let skip = (SAMPLE_RATE as usize) / 10; // skip first 100ms (Opus delay)
    let analysis = if decoded_audio_samples_ch0.len() > skip {
        &decoded_audio_samples_ch0[skip..]
    } else {
        &decoded_audio_samples_ch0[..]
    };
    let decoded_freq = estimate_frequency_zero_crossings(analysis, SAMPLE_RATE);
    let decoded_rms = rms(analysis);
    let expected_rms = AMPLITUDE / 2.0_f64.sqrt();

    println!(
        "    Audio: freq={:.2} Hz (expected {:.2}), rms={:.4} (expected {:.4})",
        decoded_freq, FREQUENCY_HZ, decoded_rms, expected_rms
    );

    assert!(
        (decoded_freq - FREQUENCY_HZ).abs() < 5.0,
        "audio frequency mismatch: got {} Hz, expected {} Hz",
        decoded_freq,
        FREQUENCY_HZ
    );
    let rms_ratio = decoded_rms / expected_rms;
    assert!(
        rms_ratio > 0.8 && rms_ratio < 1.2,
        "audio RMS ratio {} out of tolerance",
        rms_ratio
    );

    println!("\n✓ MP4 full pipeline roundtrip test passed");
}

// ============================================================================
// Helpers
// ============================================================================

fn encode_all_video(
    encoder: &mut Vp9Encoder,
    frames: &[Frame],
) -> Vec<rust_media_core::Packet> {
    let mut packets = Vec::new();
    for frame in frames {
        encoder.send_frame(frame).unwrap();
        loop {
            match encoder.receive_packet() {
                Ok(pkt) => packets.push(pkt),
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("encoder receive_packet failed: {:?}", e),
            }
        }
    }
    encoder.flush().unwrap();
    loop {
        match encoder.receive_packet() {
            Ok(pkt) => packets.push(pkt),
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("encoder flush failed: {:?}", e),
        }
    }
    packets
}

fn encode_all_audio(
    encoder: &mut OpusEncoder,
    frames: &[Frame],
) -> Vec<rust_media_core::Packet> {
    let mut packets = Vec::new();
    for frame in frames {
        encoder.send_frame(frame).unwrap();
        loop {
            match encoder.receive_packet() {
                Ok(pkt) => packets.push(pkt),
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("encoder receive_packet failed: {:?}", e),
            }
        }
    }
    encoder.flush().unwrap();
    loop {
        match encoder.receive_packet() {
            Ok(pkt) => packets.push(pkt),
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("encoder flush failed: {:?}", e),
        }
    }
    packets
}

fn decode_all_video(
    decoder: &mut Vp9Decoder,
    packets: &[rust_media_core::Packet],
) -> Vec<Frame> {
    let mut frames = Vec::new();
    for pkt in packets {
        decoder.send_packet(pkt).unwrap();
        loop {
            match decoder.receive_frame() {
                Ok(f) => frames.push(f),
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("decoder receive_frame failed: {:?}", e),
            }
        }
    }
    decoder.flush().unwrap();
    loop {
        match decoder.receive_frame() {
            Ok(f) => frames.push(f),
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("decoder flush failed: {:?}", e),
        }
    }
    frames
}

fn decode_all_audio_ch0(
    decoder: &mut OpusDecoder,
    packets: &[rust_media_core::Packet],
) -> Vec<f64> {
    let mut samples = Vec::new();
    for pkt in packets {
        decoder.send_packet(pkt).unwrap();
        loop {
            match decoder.receive_frame() {
                Ok(f) => {
                    let s = read_audio_samples_normalized(&f);
                    samples.extend(s.iter().step_by(CHANNELS));
                }
                Err(Error::NeedMoreData) => break,
                Err(e) => panic!("decoder receive_frame failed: {:?}", e),
            }
        }
    }
    decoder.flush().unwrap();
    loop {
        match decoder.receive_frame() {
            Ok(f) => {
                let s = read_audio_samples_normalized(&f);
                samples.extend(s.iter().step_by(CHANNELS));
            }
            Err(Error::EndOfStream) | Err(Error::NeedMoreData) => break,
            Err(e) => panic!("decoder flush failed: {:?}", e),
        }
    }
    samples
}
