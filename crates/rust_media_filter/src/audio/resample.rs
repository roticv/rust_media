//! Linear interpolation audio resampler

use rust_media_core::{Frame, SampleFormat};

/// Linear interpolation audio resampler
///
/// Resamples audio frames from one sample rate to another using linear
/// interpolation. Operates on interleaved S16 samples.
///
/// # Example
///
/// ```rust,ignore
/// let resampler = AudioResampler::new(48000);
/// let resampled_frame = resampler.resample(&decoded_frame)?;
/// ```
pub struct AudioResampler {
    target_sample_rate: u32,
}

impl AudioResampler {
    pub fn new(target_sample_rate: u32) -> Self {
        Self { target_sample_rate }
    }

    /// Returns the target sample rate
    pub fn target_sample_rate(&self) -> u32 {
        self.target_sample_rate
    }

    /// Resample an audio frame to the target sample rate.
    /// Uses linear interpolation on interleaved S16 samples.
    pub fn resample(&self, frame: &Frame) -> Result<Frame, Box<dyn std::error::Error>> {
        let params = frame.audio_params().ok_or("Not an audio frame")?;

        let src_rate = params.sample_rate;
        let dst_rate = self.target_sample_rate;

        if src_rate == dst_rate {
            return Ok(frame.clone());
        }

        let channels = params.channels;
        let src_samples = params.num_samples;
        let dst_samples = ((src_samples as u64 * dst_rate as u64) / src_rate as u64) as usize;

        if dst_samples == 0 {
            return Ok(frame.clone());
        }

        // Read source interleaved i16 samples
        let src_data = frame.plane(0).ok_or("Missing audio data")?;
        let src_len = src_samples * channels;
        let mut src_i16 = Vec::with_capacity(src_len);
        for i in 0..src_len {
            let lo = src_data[i * 2];
            let hi = src_data[i * 2 + 1];
            src_i16.push(i16::from_le_bytes([lo, hi]));
        }

        // Resample each channel via linear interpolation
        let mut dst_i16 = vec![0i16; dst_samples * channels];
        let ratio = src_rate as f64 / dst_rate as f64;

        for dst_idx in 0..dst_samples {
            let src_pos = dst_idx as f64 * ratio;
            let src_idx = src_pos as usize;
            let frac = src_pos - src_idx as f64;

            for ch in 0..channels {
                let s0 = src_i16[src_idx * channels + ch] as f64;
                let s1 = if src_idx + 1 < src_samples {
                    src_i16[(src_idx + 1) * channels + ch] as f64
                } else {
                    s0
                };
                let interpolated = s0 + frac * (s1 - s0);
                dst_i16[dst_idx * channels + ch] =
                    interpolated.round().clamp(-32768.0, 32767.0) as i16;
            }
        }

        // Build output frame
        let mut out = Frame::new_audio(dst_rate, channels, SampleFormat::S16, dst_samples);
        let out_data = out.plane_mut(0).ok_or("Missing output data")?;
        for (i, &sample) in dst_i16.iter().enumerate() {
            let bytes = sample.to_le_bytes();
            out_data[i * 2] = bytes[0];
            out_data[i * 2 + 1] = bytes[1];
        }

        // Preserve PTS
        out.set_pts(frame.pts());
        let duration = (dst_samples as u64 * 1_000_000) / dst_rate as u64;
        out = out.with_duration(duration as i64);

        Ok(out)
    }
}
