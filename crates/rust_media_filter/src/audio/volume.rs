//! Volume (gain) audio filter
//!
//! Adjusts audio volume by multiplying samples by a gain factor.
//! Supports both linear gain and dB specification.
//!
//! # Examples
//!
//! ```rust,ignore
//! // Double the volume
//! let filter = VolumeFilter::new(2.0);
//!
//! // Halve the volume
//! let filter = VolumeFilter::new(0.5);
//!
//! // Set volume from dB (+6 dB ≈ 2x, -6 dB ≈ 0.5x)
//! let filter = VolumeFilter::from_db(6.0);
//!
//! let output = filter.process(&frame)?;
//! ```

use rust_media_core::{Frame, SampleFormat};

/// Audio volume filter
///
/// Multiplies all audio samples by a linear gain factor.
/// Clamps output to prevent clipping.
pub struct VolumeFilter {
    /// Linear gain factor (1.0 = no change, 2.0 = double, 0.5 = half)
    gain: f64,
}

impl VolumeFilter {
    /// Create a volume filter with a linear gain factor.
    ///
    /// - `1.0` = no change
    /// - `2.0` = double volume (+6 dB)
    /// - `0.5` = half volume (-6 dB)
    /// - `0.0` = mute
    pub fn new(gain: f64) -> Self {
        Self { gain }
    }

    /// Create a volume filter from a dB value.
    ///
    /// - `0.0` dB = no change
    /// - `+6.0` dB ≈ double volume
    /// - `-6.0` dB ≈ half volume
    /// - `-inf` dB = mute
    pub fn from_db(db: f64) -> Self {
        Self {
            gain: 10.0f64.powf(db / 20.0),
        }
    }

    /// Returns the current linear gain factor.
    pub fn gain(&self) -> f64 {
        self.gain
    }

    /// Returns the current gain in dB.
    pub fn gain_db(&self) -> f64 {
        if self.gain <= 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * self.gain.log10()
        }
    }

    /// Apply volume adjustment to an audio frame.
    ///
    /// Operates on interleaved S16 samples. Clamps output to i16 range.
    pub fn process(&self, frame: &Frame) -> Result<Frame, Box<dyn std::error::Error>> {
        let params = frame.audio_params().ok_or("Not an audio frame")?;

        // Fast path: no change
        if (self.gain - 1.0).abs() < f64::EPSILON {
            return Ok(frame.clone());
        }

        let channels = params.channels;
        let num_samples = params.num_samples;
        let sample_rate = params.sample_rate;
        let total_samples = num_samples * channels;

        let src_data = frame.plane(0).ok_or("Missing audio data")?;

        let mut out = Frame::new_audio(sample_rate, channels, SampleFormat::S16, num_samples);
        let out_data = out.plane_mut(0).ok_or("Missing output data")?;

        // Mute fast path
        if self.gain == 0.0 {
            out_data.fill(0);
        } else {
            for i in 0..total_samples {
                let sample = i16::from_le_bytes([src_data[i * 2], src_data[i * 2 + 1]]);
                let adjusted = (sample as f64 * self.gain).round().clamp(-32768.0, 32767.0) as i16;
                let bytes = adjusted.to_le_bytes();
                out_data[i * 2] = bytes[0];
                out_data[i * 2 + 1] = bytes[1];
            }
        }

        out.set_pts(frame.pts());
        if let Some(dur) = frame.duration() {
            out = out.with_duration(dur);
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_frame(samples: &[i16]) -> Frame {
        let num_samples = samples.len();
        let mut frame = Frame::new_audio(48000, 1, SampleFormat::S16, num_samples);
        let data = frame.plane_mut(0).unwrap();
        for (i, &s) in samples.iter().enumerate() {
            let bytes = s.to_le_bytes();
            data[i * 2] = bytes[0];
            data[i * 2 + 1] = bytes[1];
        }
        frame
    }

    fn read_samples(frame: &Frame) -> Vec<i16> {
        let data = frame.plane(0).unwrap();
        let params = frame.audio_params().unwrap();
        let total = params.num_samples * params.channels;
        (0..total)
            .map(|i| i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]))
            .collect()
    }

    #[test]
    fn test_unity_gain() {
        let frame = make_test_frame(&[1000, -1000, 0, 32767]);
        let filter = VolumeFilter::new(1.0);
        let out = filter.process(&frame).unwrap();
        assert_eq!(read_samples(&out), vec![1000, -1000, 0, 32767]);
    }

    #[test]
    fn test_double_volume() {
        let frame = make_test_frame(&[1000, -1000, 100]);
        let filter = VolumeFilter::new(2.0);
        let out = filter.process(&frame).unwrap();
        assert_eq!(read_samples(&out), vec![2000, -2000, 200]);
    }

    #[test]
    fn test_half_volume() {
        let frame = make_test_frame(&[1000, -1000, 100]);
        let filter = VolumeFilter::new(0.5);
        let out = filter.process(&frame).unwrap();
        assert_eq!(read_samples(&out), vec![500, -500, 50]);
    }

    #[test]
    fn test_clipping() {
        let frame = make_test_frame(&[32767, -32768]);
        let filter = VolumeFilter::new(2.0);
        let out = filter.process(&frame).unwrap();
        let samples = read_samples(&out);
        assert_eq!(samples[0], 32767); // clamped
        assert_eq!(samples[1], -32768); // clamped
    }

    #[test]
    fn test_mute() {
        let frame = make_test_frame(&[1000, -1000, 32767]);
        let filter = VolumeFilter::new(0.0);
        let out = filter.process(&frame).unwrap();
        assert_eq!(read_samples(&out), vec![0, 0, 0]);
    }

    #[test]
    fn test_from_db() {
        // +6 dB ≈ 2x gain
        let filter = VolumeFilter::from_db(6.0);
        assert!((filter.gain() - 1.9953).abs() < 0.01);

        // -6 dB ≈ 0.5x gain
        let filter = VolumeFilter::from_db(-6.0);
        assert!((filter.gain() - 0.5012).abs() < 0.01);

        // 0 dB = unity
        let filter = VolumeFilter::from_db(0.0);
        assert!((filter.gain() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gain_db_roundtrip() {
        let filter = VolumeFilter::from_db(3.5);
        assert!((filter.gain_db() - 3.5).abs() < 0.001);
    }

    #[test]
    fn test_preserves_pts() {
        let mut frame = make_test_frame(&[100]);
        frame.set_pts(Some(12345));
        let filter = VolumeFilter::new(2.0);
        let out = filter.process(&frame).unwrap();
        assert_eq!(out.pts(), Some(12345));
    }
}
