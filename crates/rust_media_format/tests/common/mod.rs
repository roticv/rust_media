//! Shared test source generators and analysis helpers for format-level tests.
//!
//! Mirrors the helpers in `rust_media_codec/tests/common/mod.rs`. The duplication
//! is intentional — sharing test-only modules across crates requires either a
//! workspace-internal helper crate or git symlinks, neither of which is worth
//! the complexity for ~150 lines of pure functions.

#![allow(dead_code)]

use rust_media_core::{Frame, PixelFormat, SampleFormat};
use std::f64::consts::PI;

// ============================================================================
// Audio test sources
// ============================================================================

/// Generate one audio frame containing a sine wave segment (interleaved S16).
pub fn sine_wave_frame(
    frequency_hz: f64,
    sample_rate: u32,
    channels: usize,
    num_samples: usize,
    start_sample: u64,
    amplitude: f64,
) -> Frame {
    let mut frame = Frame::new_audio(sample_rate, channels, SampleFormat::S16, num_samples);
    let data = frame.plane_mut(0).expect("audio frame missing data plane");

    let peak = 32767.0 * amplitude.clamp(0.0, 1.0);
    for i in 0..num_samples {
        let t = (start_sample + i as u64) as f64 / sample_rate as f64;
        let sample = (peak * (2.0 * PI * frequency_hz * t).sin()).round() as i16;
        let bytes = sample.to_le_bytes();
        for ch in 0..channels {
            let idx = (i * channels + ch) * 2;
            data[idx] = bytes[0];
            data[idx + 1] = bytes[1];
        }
    }

    frame
}

/// Generate a sequence of sine wave frames with monotonic PTS in microseconds.
pub fn sine_wave_frames(
    frequency_hz: f64,
    sample_rate: u32,
    channels: usize,
    total_samples: usize,
    samples_per_frame: usize,
    amplitude: f64,
) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut produced = 0;
    while produced < total_samples {
        let chunk = (total_samples - produced).min(samples_per_frame);
        let mut frame = sine_wave_frame(
            frequency_hz,
            sample_rate,
            channels,
            chunk,
            produced as u64,
            amplitude,
        );
        let pts_us = (produced as i64 * 1_000_000) / sample_rate as i64;
        frame.set_pts(Some(pts_us));
        frames.push(frame);
        produced += chunk;
    }
    frames
}

// ============================================================================
// Video test sources
// ============================================================================

/// Generate a flat-color YUV420P video frame.
pub fn solid_color_frame(width: usize, height: usize, y_value: u8) -> Frame {
    let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);
    frame.plane_mut(0).expect("Y plane").fill(y_value);
    frame.plane_mut(1).expect("U plane").fill(128);
    frame.plane_mut(2).expect("V plane").fill(128);
    frame
}

/// Generate `count` frames each with a unique luma value (cycling through
/// the valid Y range [16, 235]).
pub fn color_ramp_frames(width: usize, height: usize, count: usize) -> Vec<Frame> {
    (0..count)
        .map(|i| {
            let y = 16 + ((i * 8) % 220) as u8;
            solid_color_frame(width, height, y)
        })
        .collect()
}

// ============================================================================
// Audio analysis
// ============================================================================

/// Read interleaved S16 samples from an audio frame as f64 in [-1.0, 1.0].
pub fn read_audio_samples_normalized(frame: &Frame) -> Vec<f64> {
    let params = frame.audio_params().expect("not an audio frame");
    let channels = params.channels;
    let num_samples = params.num_samples;
    let total = num_samples * channels;
    let data = frame.plane(0).expect("audio frame missing data plane");

    (0..total)
        .map(|i| {
            let sample = i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
            sample as f64 / 32768.0
        })
        .collect()
}

/// Compute root-mean-square amplitude in [0.0, 1.0].
pub fn rms(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Estimate dominant frequency via upward zero crossings.
pub fn estimate_frequency_zero_crossings(samples: &[f64], sample_rate: u32) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut crossings = 0;
    for i in 1..samples.len() {
        if samples[i - 1] < 0.0 && samples[i] >= 0.0 {
            crossings += 1;
        }
    }
    let duration_secs = samples.len() as f64 / sample_rate as f64;
    crossings as f64 / duration_secs
}

// ============================================================================
// Video analysis
// ============================================================================

/// Compute the mean Y plane value of a video frame.
pub fn mean_y(frame: &Frame) -> f64 {
    let y = frame.plane(0).expect("Y plane");
    if y.is_empty() {
        return 0.0;
    }
    let sum: u64 = y.iter().map(|&p| p as u64).sum();
    sum as f64 / y.len() as f64
}
