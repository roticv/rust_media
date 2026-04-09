//! Bilinear video scaler
//!
//! Scales YUV420P video frames to a target width and height using bilinear
//! interpolation. Each plane (Y, U, V) is scaled independently. Chroma planes
//! are at half resolution (4:2:0 subsampling).
//!
//! # Algorithm
//!
//! For each output pixel `(x, y)`:
//! 1. Map to source position: `x_src = x * (src_w - 1) / (dst_w - 1)`
//! 2. Take the 4 surrounding source pixels
//! 3. Linearly interpolate using fractional offsets
//!
//! Equivalent to FFmpeg's `scale=W:H` filter with `flags=bilinear`.
//!
//! # Example
//!
//! ```rust,ignore
//! let scaler = ScaleFilter::new(640, 480);
//! let scaled = scaler.process(&input_frame)?;
//! ```

use rust_media_core::{Frame, PixelFormat};

/// Bilinear video scaler
///
/// Resizes YUV420P frames to a fixed target dimension. The output dimensions
/// must be even (required for 4:2:0 chroma subsampling).
pub struct ScaleFilter {
    target_width: usize,
    target_height: usize,
}

impl ScaleFilter {
    /// Create a new bilinear scaler.
    ///
    /// # Errors
    ///
    /// Returns an error if width or height is zero, or if either dimension
    /// is odd (YUV420P requires even dimensions for chroma subsampling).
    pub fn new(target_width: usize, target_height: usize) -> Result<Self, String> {
        if target_width == 0 || target_height == 0 {
            return Err(format!(
                "scale dimensions must be positive, got {}x{}",
                target_width, target_height
            ));
        }
        if !target_width.is_multiple_of(2) || !target_height.is_multiple_of(2) {
            return Err(format!(
                "scale dimensions must be even for YUV420P, got {}x{}",
                target_width, target_height
            ));
        }
        Ok(Self {
            target_width,
            target_height,
        })
    }

    pub fn target_width(&self) -> usize {
        self.target_width
    }

    pub fn target_height(&self) -> usize {
        self.target_height
    }

    /// Scale an input frame to the target dimensions.
    ///
    /// Returns an error if the input is not a YUV420P or YUV420P10LE video frame.
    pub fn process(&self, frame: &Frame) -> Result<Frame, Box<dyn std::error::Error>> {
        let params = frame.video_params().ok_or("Not a video frame")?;

        if params.format != PixelFormat::YUV420P && params.format != PixelFormat::YUV420P10LE {
            return Err(format!(
                "scale filter only supports YUV420P and YUV420P10LE, got {:?}",
                params.format
            )
            .into());
        }

        let src_w = params.width;
        let src_h = params.height;
        let format = params.format;

        // Identity fast path
        if src_w == self.target_width && src_h == self.target_height {
            return Ok(frame.clone());
        }

        let dst_w = self.target_width;
        let dst_h = self.target_height;

        // Allocate output frame in the same format as input
        let mut out = Frame::new_video(dst_w, dst_h, format);

        // Dispatch to 8-bit or 10-bit scaler based on format
        let is_10bit = format == PixelFormat::YUV420P10LE;

        // Scale Y plane (full resolution)
        {
            let src_y = frame.plane(0).ok_or("missing Y plane")?;
            let src_y_owned: Vec<u8> = src_y.to_vec();
            let dst_y = out.plane_mut(0).ok_or("missing output Y plane")?;
            if is_10bit {
                scale_plane_bilinear_u16(&src_y_owned, src_w, src_h, dst_y, dst_w, dst_h);
            } else {
                scale_plane_bilinear(&src_y_owned, src_w, src_h, dst_y, dst_w, dst_h);
            }
        }

        // Scale U plane (half resolution in both dims)
        {
            let src_u = frame.plane(1).ok_or("missing U plane")?;
            let src_u_owned: Vec<u8> = src_u.to_vec();
            let dst_u = out.plane_mut(1).ok_or("missing output U plane")?;
            if is_10bit {
                scale_plane_bilinear_u16(
                    &src_u_owned,
                    src_w / 2,
                    src_h / 2,
                    dst_u,
                    dst_w / 2,
                    dst_h / 2,
                );
            } else {
                scale_plane_bilinear(
                    &src_u_owned,
                    src_w / 2,
                    src_h / 2,
                    dst_u,
                    dst_w / 2,
                    dst_h / 2,
                );
            }
        }

        // Scale V plane (half resolution in both dims)
        {
            let src_v = frame.plane(2).ok_or("missing V plane")?;
            let src_v_owned: Vec<u8> = src_v.to_vec();
            let dst_v = out.plane_mut(2).ok_or("missing output V plane")?;
            if is_10bit {
                scale_plane_bilinear_u16(
                    &src_v_owned,
                    src_w / 2,
                    src_h / 2,
                    dst_v,
                    dst_w / 2,
                    dst_h / 2,
                );
            } else {
                scale_plane_bilinear(
                    &src_v_owned,
                    src_w / 2,
                    src_h / 2,
                    dst_v,
                    dst_w / 2,
                    dst_h / 2,
                );
            }
        }

        // Preserve PTS and duration
        out.set_pts(frame.pts());
        if let Some(dur) = frame.duration() {
            out = out.with_duration(dur);
        }

        Ok(out)
    }
}

/// Scale a single 10-bit grayscale plane using bilinear interpolation.
///
/// Both `src` and `dst` are byte slices containing little-endian u16 samples
/// (2 bytes per pixel). Only the lower 10 bits are valid (0..1024 range), but
/// since we work in floating point internally, all 16 bits round-trip safely.
fn scale_plane_bilinear_u16(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return;
    }

    let read_u16 = |bytes: &[u8], offset: usize| -> u16 {
        u16::from_le_bytes([bytes[offset * 2], bytes[offset * 2 + 1]])
    };

    let x_ratio = if dst_w > 1 {
        (src_w - 1) as f64 / (dst_w - 1) as f64
    } else {
        0.0
    };
    let y_ratio = if dst_h > 1 {
        (src_h - 1) as f64 / (dst_h - 1) as f64
    } else {
        0.0
    };

    for y_dst in 0..dst_h {
        let y_src_f = y_dst as f64 * y_ratio;
        let y_src = y_src_f as usize;
        let fy = y_src_f - y_src as f64;
        let y_src_next = (y_src + 1).min(src_h - 1);

        let row0_off = y_src * src_w;
        let row1_off = y_src_next * src_w;
        let dst_row_off = y_dst * dst_w;

        for x_dst in 0..dst_w {
            let x_src_f = x_dst as f64 * x_ratio;
            let x_src = x_src_f as usize;
            let fx = x_src_f - x_src as f64;
            let x_src_next = (x_src + 1).min(src_w - 1);

            let p00 = read_u16(src, row0_off + x_src) as f64;
            let p10 = read_u16(src, row0_off + x_src_next) as f64;
            let p01 = read_u16(src, row1_off + x_src) as f64;
            let p11 = read_u16(src, row1_off + x_src_next) as f64;

            let top = p00 * (1.0 - fx) + p10 * fx;
            let bottom = p01 * (1.0 - fx) + p11 * fx;
            // 10-bit values clamp to [0, 1023] (the valid 10-bit range)
            let val = top * (1.0 - fy) + bottom * fy;
            let sample = val.round().clamp(0.0, 1023.0) as u16;

            let dst_idx = (dst_row_off + x_dst) * 2;
            let bytes = sample.to_le_bytes();
            dst[dst_idx] = bytes[0];
            dst[dst_idx + 1] = bytes[1];
        }
    }
}

/// Scale a single 8-bit grayscale plane using bilinear interpolation.
///
/// `src` and `dst` are tightly packed (no stride padding).
fn scale_plane_bilinear(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return;
    }

    // Compute scaling ratios so we can map output pixels to source positions.
    // Using (src - 1) / (dst - 1) ensures the corners line up exactly.
    let x_ratio = if dst_w > 1 {
        (src_w - 1) as f64 / (dst_w - 1) as f64
    } else {
        0.0
    };
    let y_ratio = if dst_h > 1 {
        (src_h - 1) as f64 / (dst_h - 1) as f64
    } else {
        0.0
    };

    for y_dst in 0..dst_h {
        let y_src_f = y_dst as f64 * y_ratio;
        let y_src = y_src_f as usize;
        let fy = y_src_f - y_src as f64;
        let y_src_next = (y_src + 1).min(src_h - 1);

        let row0_off = y_src * src_w;
        let row1_off = y_src_next * src_w;
        let dst_row_off = y_dst * dst_w;

        for x_dst in 0..dst_w {
            let x_src_f = x_dst as f64 * x_ratio;
            let x_src = x_src_f as usize;
            let fx = x_src_f - x_src as f64;
            let x_src_next = (x_src + 1).min(src_w - 1);

            let p00 = src[row0_off + x_src] as f64;
            let p10 = src[row0_off + x_src_next] as f64;
            let p01 = src[row1_off + x_src] as f64;
            let p11 = src[row1_off + x_src_next] as f64;

            // Bilinear interpolation
            let top = p00 * (1.0 - fx) + p10 * fx;
            let bottom = p01 * (1.0 - fx) + p11 * fx;
            let val = top * (1.0 - fy) + bottom * fy;

            dst[dst_row_off + x_dst] = val.round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::{Frame, PixelFormat};

    fn make_solid_frame(width: usize, height: usize, y: u8, u: u8, v: u8) -> Frame {
        let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);
        frame.plane_mut(0).unwrap().fill(y);
        frame.plane_mut(1).unwrap().fill(u);
        frame.plane_mut(2).unwrap().fill(v);
        frame
    }

    fn mean_y(frame: &Frame) -> f64 {
        let y = frame.plane(0).unwrap();
        let sum: u64 = y.iter().map(|&p| p as u64).sum();
        sum as f64 / y.len() as f64
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert!(ScaleFilter::new(0, 480).is_err());
        assert!(ScaleFilter::new(640, 0).is_err());
    }

    #[test]
    fn rejects_odd_dimensions() {
        assert!(ScaleFilter::new(641, 480).is_err());
        assert!(ScaleFilter::new(640, 481).is_err());
    }

    #[test]
    fn identity_scale_returns_clone() {
        let frame = make_solid_frame(320, 240, 100, 50, 200);
        let scaler = ScaleFilter::new(320, 240).unwrap();
        let scaled = scaler.process(&frame).unwrap();

        let params = scaled.video_params().unwrap();
        assert_eq!(params.width, 320);
        assert_eq!(params.height, 240);
        assert_eq!(mean_y(&scaled), 100.0);
    }

    #[test]
    fn solid_color_scales_preserved_under_2x_upscale() {
        let frame = make_solid_frame(64, 64, 128, 64, 192);
        let scaler = ScaleFilter::new(128, 128).unwrap();
        let scaled = scaler.process(&frame).unwrap();

        let params = scaled.video_params().unwrap();
        assert_eq!(params.width, 128);
        assert_eq!(params.height, 128);

        // Solid color should still be uniform after scaling
        let y = scaled.plane(0).unwrap();
        for &pixel in y {
            assert_eq!(pixel, 128, "Y plane should be uniform 128 after upscale");
        }
        let u = scaled.plane(1).unwrap();
        for &pixel in u {
            assert_eq!(pixel, 64);
        }
        let v = scaled.plane(2).unwrap();
        for &pixel in v {
            assert_eq!(pixel, 192);
        }
    }

    #[test]
    fn solid_color_scales_preserved_under_2x_downscale() {
        let frame = make_solid_frame(128, 128, 200, 100, 50);
        let scaler = ScaleFilter::new(64, 64).unwrap();
        let scaled = scaler.process(&frame).unwrap();

        let params = scaled.video_params().unwrap();
        assert_eq!(params.width, 64);
        assert_eq!(params.height, 64);

        let y = scaled.plane(0).unwrap();
        for &pixel in y {
            assert_eq!(pixel, 200, "downscaled Y should still be 200");
        }
    }

    #[test]
    fn nonsquare_scale_works() {
        let frame = make_solid_frame(320, 240, 100, 50, 50);
        let scaler = ScaleFilter::new(160, 90).unwrap();
        let scaled = scaler.process(&frame).unwrap();

        let params = scaled.video_params().unwrap();
        assert_eq!(params.width, 160);
        assert_eq!(params.height, 90);
        assert_eq!(mean_y(&scaled), 100.0);
    }

    #[test]
    fn gradient_scale_preserves_average_value() {
        // Create a horizontal gradient from 0 to 255
        let mut frame = Frame::new_video(64, 64, PixelFormat::YUV420P);
        let y_plane = frame.plane_mut(0).unwrap();
        for y in 0..64 {
            for x in 0..64 {
                y_plane[y * 64 + x] = ((x * 255) / 63) as u8;
            }
        }
        frame.plane_mut(1).unwrap().fill(128);
        frame.plane_mut(2).unwrap().fill(128);

        let original_mean = mean_y(&frame);

        let scaler = ScaleFilter::new(32, 32).unwrap();
        let scaled = scaler.process(&frame).unwrap();
        let scaled_mean = mean_y(&scaled);

        // Mean should be approximately preserved (within 1 luma unit)
        let diff = (original_mean - scaled_mean).abs();
        assert!(
            diff < 1.0,
            "mean luma should be preserved: original={} scaled={}",
            original_mean, scaled_mean
        );
    }

    #[test]
    fn rejects_non_yuv420p_input() {
        // Create a non-YUV420P/non-YUV420P10LE frame
        let frame = Frame::new_video(64, 64, PixelFormat::NV12);
        let scaler = ScaleFilter::new(32, 32).unwrap();
        let result = scaler.process(&frame);
        assert!(result.is_err(), "should reject non-YUV420P input");
    }

    fn make_solid_10bit_frame(width: usize, height: usize, y10: u16) -> Frame {
        let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P10LE);

        let bytes = y10.to_le_bytes();
        let y_plane = frame.plane_mut(0).unwrap();
        for chunk in y_plane.chunks_exact_mut(2) {
            chunk[0] = bytes[0];
            chunk[1] = bytes[1];
        }
        // U/V neutral chroma at 10-bit = 512
        let neutral = 512u16.to_le_bytes();
        for plane_idx in [1, 2] {
            let plane = frame.plane_mut(plane_idx).unwrap();
            for chunk in plane.chunks_exact_mut(2) {
                chunk[0] = neutral[0];
                chunk[1] = neutral[1];
            }
        }
        frame
    }

    fn read_10bit_sample(plane: &[u8], idx: usize) -> u16 {
        u16::from_le_bytes([plane[idx * 2], plane[idx * 2 + 1]])
    }

    #[test]
    fn scales_10bit_solid_color_preserved() {
        let frame = make_solid_10bit_frame(64, 64, 800);
        let scaler = ScaleFilter::new(32, 32).unwrap();
        let scaled = scaler.process(&frame).unwrap();

        let params = scaled.video_params().unwrap();
        assert_eq!(params.format, PixelFormat::YUV420P10LE);
        assert_eq!(params.width, 32);
        assert_eq!(params.height, 32);

        // Verify every Y sample is still 800
        let y = scaled.plane(0).unwrap();
        assert_eq!(y.len(), 32 * 32 * 2);
        for i in 0..(32 * 32) {
            assert_eq!(read_10bit_sample(y, i), 800);
        }
    }

    #[test]
    fn scales_10bit_upscale() {
        let frame = make_solid_10bit_frame(32, 32, 1023);
        let scaler = ScaleFilter::new(64, 64).unwrap();
        let scaled = scaler.process(&frame).unwrap();
        let y = scaled.plane(0).unwrap();
        assert_eq!(y.len(), 64 * 64 * 2);
        for i in 0..(64 * 64) {
            assert_eq!(read_10bit_sample(y, i), 1023);
        }
    }

    #[test]
    fn scales_10bit_clamps_to_10bit_range() {
        // After scaling a uniform image, samples should never exceed 10-bit max
        let frame = make_solid_10bit_frame(64, 64, 1023);
        let scaler = ScaleFilter::new(32, 32).unwrap();
        let scaled = scaler.process(&frame).unwrap();
        let y = scaled.plane(0).unwrap();
        for i in 0..(32 * 32) {
            assert!(read_10bit_sample(y, i) <= 1023);
        }
    }

    #[test]
    fn preserves_pts_and_duration() {
        let mut frame = make_solid_frame(64, 64, 100, 50, 50);
        frame.set_pts(Some(12345));
        frame = frame.with_duration(67890);

        let scaler = ScaleFilter::new(32, 32).unwrap();
        let scaled = scaler.process(&frame).unwrap();

        assert_eq!(scaled.pts(), Some(12345));
        assert_eq!(scaled.duration(), Some(67890));
    }
}
