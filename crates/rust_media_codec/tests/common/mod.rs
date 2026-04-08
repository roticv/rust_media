//! Shared helpers and test source generators for codec integration tests
//!
//! This module is included from individual test files via `mod common;` and
//! provides:
//! - Audio test source generators (sine wave, silence)
//! - Video test source generators (color ramp, solid color)
//! - Audio analysis helpers (RMS, dominant frequency, sample comparison)
//! - Video analysis helpers (mean Y, frame similarity)
//!
//! The goal is to enable encode → decode roundtrip tests with **no committed
//! binary fixtures** — all test content is generated in pure Rust.

#![allow(dead_code)] // Different test files use different subsets

use rust_media_core::{Frame, PixelFormat, SampleFormat};
use std::f64::consts::PI;

// ============================================================================
// Audio test sources
// ============================================================================

/// Generate one audio frame containing a sine wave segment.
///
/// Produces interleaved S16 samples at the given sample rate. The sine wave
/// phase is determined by `start_sample` so consecutive frames are continuous.
///
/// # Arguments
///
/// * `frequency_hz` - sine wave frequency (e.g., 1000.0 for 1 kHz)
/// * `sample_rate` - samples per second (e.g., 48000)
/// * `channels` - 1 (mono) or 2 (stereo)
/// * `num_samples` - samples per channel in this frame
/// * `start_sample` - sample index of the first sample (for phase continuity)
/// * `amplitude` - peak amplitude in [0.0, 1.0] range; 0.5 is a comfortable level
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

/// Generate a sequence of sine wave frames covering `total_samples` samples
/// per channel, split into chunks of `samples_per_frame`.
///
/// Useful for feeding into encoders that require specific buffer sizes.
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
        // Set PTS in microseconds for consistency with downstream pipeline
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

/// Generate a video frame with a flat (solid) color in YUV420P format.
///
/// All pixels of the Y plane have value `y_value`, U and V planes are 128
/// (neutral chroma). Useful as a deterministic per-frame identifier: encode
/// a sequence of unique Y values and verify each decoded frame matches.
pub fn solid_color_frame(width: usize, height: usize, y_value: u8) -> Frame {
    let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);
    frame
        .plane_mut(0)
        .expect("video frame missing Y plane")
        .fill(y_value);
    frame
        .plane_mut(1)
        .expect("video frame missing U plane")
        .fill(128);
    frame
        .plane_mut(2)
        .expect("video frame missing V plane")
        .fill(128);
    frame
}

/// Generate a sequence of color-ramp frames where each frame has a unique
/// luma value derived from the frame index.
///
/// Y values cycle through [16, 235] (the valid luma range for video). Each
/// decoded frame can be identified by its mean luma, even after lossy
/// compression (with tolerance).
pub fn color_ramp_frames(width: usize, height: usize, count: usize) -> Vec<Frame> {
    (0..count)
        .map(|i| {
            // Cycle through valid Y range [16, 235]
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

/// Compute root-mean-square amplitude of a sample buffer (in [0.0, 1.0]).
pub fn rms(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Estimate the dominant frequency of an audio signal via zero-crossing rate.
///
/// Counts upward zero crossings and divides by the signal duration. This is
/// fast and works well for clean periodic signals like sine waves. For real
/// audio you'd want an FFT, but for sine wave roundtrip tests this is enough.
///
/// Returns frequency in Hz.
pub fn estimate_frequency_zero_crossings(samples: &[f64], sample_rate: u32) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }

    // Count upward zero crossings (negative → positive transitions)
    let mut crossings = 0;
    for i in 1..samples.len() {
        if samples[i - 1] < 0.0 && samples[i] >= 0.0 {
            crossings += 1;
        }
    }

    // Each upward crossing = one full period
    let duration_secs = samples.len() as f64 / sample_rate as f64;
    crossings as f64 / duration_secs
}

// ============================================================================
// Video analysis
// ============================================================================

/// Compute the mean luma (Y plane) value of a video frame.
pub fn mean_y(frame: &Frame) -> f64 {
    let y = frame.plane(0).expect("video frame missing Y plane");
    if y.is_empty() {
        return 0.0;
    }
    let sum: u64 = y.iter().map(|&p| p as u64).sum();
    sum as f64 / y.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_wave_has_correct_frequency_via_zero_crossings() {
        // Generate 1 second of 1000 Hz sine at 48000 Hz
        let frame = sine_wave_frame(1000.0, 48000, 1, 48000, 0, 0.5);
        let samples = read_audio_samples_normalized(&frame);
        let freq = estimate_frequency_zero_crossings(&samples, 48000);
        // Tolerance ±2 Hz: zero-crossing counting has off-by-one at window
        // edges, so a 1-second window of 1 kHz may report 999 or 1000.
        assert!(
            (freq - 1000.0).abs() <= 2.0,
            "expected ~1000 Hz, got {}",
            freq
        );
    }

    #[test]
    fn sine_wave_rms_matches_amplitude() {
        // RMS of a sine wave is amplitude / sqrt(2)
        let frame = sine_wave_frame(1000.0, 48000, 1, 48000, 0, 0.5);
        let samples = read_audio_samples_normalized(&frame);
        let r = rms(&samples);
        let expected = 0.5 / 2.0_f64.sqrt();
        assert!(
            (r - expected).abs() < 0.01,
            "expected RMS ~{}, got {}",
            expected,
            r
        );
    }

    #[test]
    fn solid_color_frame_has_correct_mean_y() {
        let frame = solid_color_frame(64, 64, 200);
        assert_eq!(mean_y(&frame), 200.0);
    }

    #[test]
    fn color_ramp_frames_have_unique_y_values() {
        let frames = color_ramp_frames(32, 32, 10);
        let y_values: Vec<f64> = frames.iter().map(mean_y).collect();
        // First few frames should have distinct values
        assert_ne!(y_values[0], y_values[1]);
        assert_ne!(y_values[1], y_values[2]);
    }
}
