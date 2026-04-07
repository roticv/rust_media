//! High-quality audio resampler using sinc interpolation (via rubato)
//!
//! Uses a Kaiser-windowed sinc polyphase filter, comparable to ffmpeg's
//! libswresample. Replaces the previous linear interpolation resampler.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use rust_media_core::{Frame, SampleFormat};

/// Sinc interpolation audio resampler
///
/// Resamples audio frames from one sample rate to another using a
/// Kaiser-windowed sinc polyphase filter (via the rubato crate).
///
/// The resampler is lazily initialized on the first frame, since the
/// channel count is not known until then.
///
/// # Example
///
/// ```rust,ignore
/// let mut resampler = AudioResampler::new(48000);
/// let resampled_frame = resampler.resample(&decoded_frame)?;
/// ```
pub struct AudioResampler {
    target_sample_rate: u32,
    /// Lazily initialized on first frame
    inner: Option<ResamplerState>,
}

struct ResamplerState {
    resampler: SincFixedIn<f64>,
    channels: usize,
    src_rate: u32,
    chunk_size: usize,
}

impl AudioResampler {
    pub fn new(target_sample_rate: u32) -> Self {
        Self {
            target_sample_rate,
            inner: None,
        }
    }

    /// Returns the target sample rate
    pub fn target_sample_rate(&self) -> u32 {
        self.target_sample_rate
    }

    /// Initialize the rubato resampler for the given source parameters.
    fn init_resampler(
        &mut self,
        src_rate: u32,
        channels: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let ratio = self.target_sample_rate as f64 / src_rate as f64;

        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };

        // Use 1024 as the chunk size — a reasonable default for audio processing
        let chunk_size = 1024;

        let resampler = SincFixedIn::<f64>::new(ratio, 2.0, params, chunk_size, channels)?;

        self.inner = Some(ResamplerState {
            resampler,
            channels,
            src_rate,
            chunk_size,
        });

        Ok(())
    }

    /// Resample an audio frame to the target sample rate.
    pub fn resample(&mut self, frame: &Frame) -> Result<Frame, Box<dyn std::error::Error>> {
        let params = frame.audio_params().ok_or("Not an audio frame")?;

        let src_rate = params.sample_rate;
        let dst_rate = self.target_sample_rate;

        if src_rate == dst_rate {
            return Ok(frame.clone());
        }

        let channels = params.channels;
        let src_samples = params.num_samples;

        // Lazy init or reinit if source parameters changed
        let needs_init = match &self.inner {
            Some(state) => state.src_rate != src_rate || state.channels != channels,
            None => true,
        };
        if needs_init {
            self.init_resampler(src_rate, channels)?;
        }

        let state = self.inner.as_mut().unwrap();

        // Convert interleaved S16 bytes → planar f64
        let src_data = frame.plane(0).ok_or("Missing audio data")?;
        let mut planar_in: Vec<Vec<f64>> = vec![Vec::with_capacity(src_samples); channels];
        for i in 0..src_samples {
            for (ch, plane) in planar_in.iter_mut().enumerate() {
                let byte_idx = (i * channels + ch) * 2;
                let sample = i16::from_le_bytes([src_data[byte_idx], src_data[byte_idx + 1]]);
                plane.push(sample as f64 / 32768.0);
            }
        }

        // Process through rubato in chunks
        let chunk_size = state.chunk_size;
        let mut planar_out: Vec<Vec<f64>> = vec![Vec::new(); channels];

        let mut pos = 0;
        while pos < src_samples {
            let end = (pos + chunk_size).min(src_samples);
            let chunk: Vec<Vec<f64>> = planar_in
                .iter()
                .map(|ch| {
                    let mut c = ch[pos..end].to_vec();
                    // Pad last chunk to required size
                    c.resize(chunk_size, 0.0);
                    c
                })
                .collect();

            let out = state.resampler.process(&chunk, None)?;
            for (out_ch, resampled_ch) in planar_out.iter_mut().zip(out.into_iter()) {
                out_ch.extend(resampled_ch);
            }

            pos += chunk_size;
        }

        // Calculate expected output samples and trim padding artifacts
        let dst_samples = ((src_samples as u64 * dst_rate as u64) / src_rate as u64) as usize;
        for out_ch in &mut planar_out {
            out_ch.truncate(dst_samples);
        }

        let actual_dst_samples = planar_out[0].len();
        if actual_dst_samples == 0 {
            return Ok(frame.clone());
        }

        // Convert planar f64 → interleaved S16 bytes
        let mut out = Frame::new_audio(dst_rate, channels, SampleFormat::S16, actual_dst_samples);
        let out_data = out.plane_mut(0).ok_or("Missing output data")?;
        for i in 0..actual_dst_samples {
            for (ch, out_ch) in planar_out.iter().enumerate() {
                let sample_f64 = out_ch[i];
                let sample_i16 = (sample_f64 * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
                let byte_idx = (i * channels + ch) * 2;
                let bytes = sample_i16.to_le_bytes();
                out_data[byte_idx] = bytes[0];
                out_data[byte_idx + 1] = bytes[1];
            }
        }

        // Preserve PTS
        out.set_pts(frame.pts());
        let duration = (actual_dst_samples as u64 * 1_000_000) / dst_rate as u64;
        out = out.with_duration(duration as i64);

        Ok(out)
    }
}
