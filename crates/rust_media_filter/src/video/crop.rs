//! Video crop filter
//!
//! Extracts a rectangular region from a YUV420P video frame. Equivalent to
//! FFmpeg's `crop=W:H:X:Y` filter.
//!
//! # Parameters
//!
//! - `width`, `height`: dimensions of the output crop region
//! - `x`, `y`: top-left offset in the source frame (defaults to centered)
//!
//! All four values (and the source frame dimensions) must be even, since
//! YUV420P uses 4:2:0 chroma subsampling and we crop chroma planes at
//! half resolution.
//!
//! # Example
//!
//! ```rust,ignore
//! // Crop the center 320x240 region of a 640x480 frame
//! let cropper = CropFilter::new(320, 240, None, None);
//!
//! // Crop a specific region (top-left 320x240)
//! let cropper = CropFilter::new(320, 240, Some(0), Some(0));
//! ```

use rust_media_core::{Frame, PixelFormat};

/// Bilinear video cropper
///
/// Extracts a rectangular region from a YUV420P frame. The region must lie
/// entirely within the source frame, and all dimensions/offsets must be even
/// for YUV420P chroma alignment.
pub struct CropFilter {
    width: usize,
    height: usize,
    /// Top-left X offset; `None` means center horizontally
    x: Option<usize>,
    /// Top-left Y offset; `None` means center vertically
    y: Option<usize>,
}

impl CropFilter {
    /// Create a new crop filter.
    ///
    /// `x` and `y` are optional — if `None`, the crop is centered along that axis.
    ///
    /// # Errors
    ///
    /// Returns an error if `width` or `height` is zero, or if either is odd
    /// (YUV420P chroma subsampling requires even dimensions).
    pub fn new(
        width: usize,
        height: usize,
        x: Option<usize>,
        y: Option<usize>,
    ) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err(format!(
                "crop dimensions must be positive, got {}x{}",
                width, height
            ));
        }
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(format!(
                "crop dimensions must be even for YUV420P, got {}x{}",
                width, height
            ));
        }
        if let Some(x) = x {
            if !x.is_multiple_of(2) {
                return Err(format!("crop x offset must be even for YUV420P, got {}", x));
            }
        }
        if let Some(y) = y {
            if !y.is_multiple_of(2) {
                return Err(format!("crop y offset must be even for YUV420P, got {}", y));
            }
        }

        Ok(Self {
            width,
            height,
            x,
            y,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Crop an input frame to the configured rectangle.
    ///
    /// Returns an error if the input is not a YUV420P video frame, or if the
    /// crop region extends beyond the source frame dimensions.
    pub fn process(&self, frame: &Frame) -> Result<Frame, Box<dyn std::error::Error>> {
        let params = frame.video_params().ok_or("Not a video frame")?;

        if params.format != PixelFormat::YUV420P {
            return Err(format!(
                "crop filter only supports YUV420P, got {:?}",
                params.format
            )
            .into());
        }

        let src_w = params.width;
        let src_h = params.height;

        if self.width > src_w || self.height > src_h {
            return Err(format!(
                "crop {}x{} exceeds source dimensions {}x{}",
                self.width, self.height, src_w, src_h
            )
            .into());
        }

        // Compute offsets (centered if not specified). Center offsets must be
        // even for YUV420P, so we round down to the nearest even value.
        let x_offset = match self.x {
            Some(x) => x,
            None => ((src_w - self.width) / 2) & !1,
        };
        let y_offset = match self.y {
            Some(y) => y,
            None => ((src_h - self.height) / 2) & !1,
        };

        if x_offset + self.width > src_w {
            return Err(format!(
                "crop x={}+w={} exceeds source width {}",
                x_offset, self.width, src_w
            )
            .into());
        }
        if y_offset + self.height > src_h {
            return Err(format!(
                "crop y={}+h={} exceeds source height {}",
                y_offset, self.height, src_h
            )
            .into());
        }

        // Identity fast path
        if src_w == self.width && src_h == self.height && x_offset == 0 && y_offset == 0 {
            return Ok(frame.clone());
        }

        let mut out = Frame::new_video(self.width, self.height, PixelFormat::YUV420P);

        // Copy Y plane (full resolution)
        {
            let src_y = frame.plane(0).ok_or("missing Y plane")?;
            let src_y_owned: Vec<u8> = src_y.to_vec();
            let dst_y = out.plane_mut(0).ok_or("missing output Y plane")?;
            copy_plane_region(
                &src_y_owned,
                src_w,
                x_offset,
                y_offset,
                dst_y,
                self.width,
                self.height,
            );
        }

        // Copy U plane (half resolution)
        {
            let src_u = frame.plane(1).ok_or("missing U plane")?;
            let src_u_owned: Vec<u8> = src_u.to_vec();
            let dst_u = out.plane_mut(1).ok_or("missing output U plane")?;
            copy_plane_region(
                &src_u_owned,
                src_w / 2,
                x_offset / 2,
                y_offset / 2,
                dst_u,
                self.width / 2,
                self.height / 2,
            );
        }

        // Copy V plane (half resolution)
        {
            let src_v = frame.plane(2).ok_or("missing V plane")?;
            let src_v_owned: Vec<u8> = src_v.to_vec();
            let dst_v = out.plane_mut(2).ok_or("missing output V plane")?;
            copy_plane_region(
                &src_v_owned,
                src_w / 2,
                x_offset / 2,
                y_offset / 2,
                dst_v,
                self.width / 2,
                self.height / 2,
            );
        }

        // Preserve PTS and duration
        out.set_pts(frame.pts());
        if let Some(dur) = frame.duration() {
            out = out.with_duration(dur);
        }

        Ok(out)
    }
}

/// Copy a rectangular region from a tightly-packed source plane into a
/// tightly-packed destination plane.
fn copy_plane_region(
    src: &[u8],
    src_stride: usize,
    x_offset: usize,
    y_offset: usize,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) {
    for row in 0..dst_h {
        let src_off = (y_offset + row) * src_stride + x_offset;
        let dst_off = row * dst_w;
        dst[dst_off..dst_off + dst_w].copy_from_slice(&src[src_off..src_off + dst_w]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_solid_frame(width: usize, height: usize, y: u8) -> Frame {
        let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);
        frame.plane_mut(0).unwrap().fill(y);
        frame.plane_mut(1).unwrap().fill(128);
        frame.plane_mut(2).unwrap().fill(128);
        frame
    }

    /// Make a frame where each pixel's Y value encodes its (x, y) position
    /// modulo 256, useful for verifying that crops extract the correct region.
    fn make_position_frame(width: usize, height: usize) -> Frame {
        let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);
        let y_plane = frame.plane_mut(0).unwrap();
        for y in 0..height {
            for x in 0..width {
                y_plane[y * width + x] = ((x + y * 16) % 256) as u8;
            }
        }
        frame.plane_mut(1).unwrap().fill(128);
        frame.plane_mut(2).unwrap().fill(128);
        frame
    }

    fn mean_y(frame: &Frame) -> f64 {
        let y = frame.plane(0).unwrap();
        let sum: u64 = y.iter().map(|&p| p as u64).sum();
        sum as f64 / y.len() as f64
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert!(CropFilter::new(0, 240, None, None).is_err());
        assert!(CropFilter::new(320, 0, None, None).is_err());
    }

    #[test]
    fn rejects_odd_dimensions() {
        assert!(CropFilter::new(321, 240, None, None).is_err());
        assert!(CropFilter::new(320, 241, None, None).is_err());
    }

    #[test]
    fn rejects_odd_offsets() {
        assert!(CropFilter::new(320, 240, Some(1), Some(0)).is_err());
        assert!(CropFilter::new(320, 240, Some(0), Some(1)).is_err());
    }

    #[test]
    fn rejects_crop_larger_than_source() {
        let frame = make_solid_frame(320, 240, 100);
        let cropper = CropFilter::new(640, 480, None, None).unwrap();
        let result = cropper.process(&frame);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_crop_outside_source() {
        let frame = make_solid_frame(320, 240, 100);
        // 200+200 > 320, should fail
        let cropper = CropFilter::new(200, 200, Some(200), None).unwrap();
        let result = cropper.process(&frame);
        assert!(result.is_err());
    }

    #[test]
    fn identity_crop_returns_clone() {
        let frame = make_solid_frame(320, 240, 100);
        let cropper = CropFilter::new(320, 240, Some(0), Some(0)).unwrap();
        let cropped = cropper.process(&frame).unwrap();
        let params = cropped.video_params().unwrap();
        assert_eq!(params.width, 320);
        assert_eq!(params.height, 240);
    }

    #[test]
    fn solid_color_crop_preserves_color() {
        let frame = make_solid_frame(320, 240, 200);
        let cropper = CropFilter::new(160, 120, None, None).unwrap();
        let cropped = cropper.process(&frame).unwrap();
        let params = cropped.video_params().unwrap();
        assert_eq!(params.width, 160);
        assert_eq!(params.height, 120);

        // Every pixel should still be 200
        for &pixel in cropped.plane(0).unwrap() {
            assert_eq!(pixel, 200);
        }
    }

    #[test]
    fn centered_crop_extracts_middle() {
        // 320x240 source, crop center 160x120 → offset (80, 60)
        let frame = make_position_frame(320, 240);
        let cropper = CropFilter::new(160, 120, None, None).unwrap();
        let cropped = cropper.process(&frame).unwrap();

        // First pixel of crop should be source[60][80] = (80 + 60*16) % 256 = 1040 % 256 = 16
        let dst_y = cropped.plane(0).unwrap();
        assert_eq!(dst_y[0], ((80 + 60 * 16) % 256) as u8);

        // Last pixel of first row of crop: source[60][80+159] = (239 + 60*16) % 256 = 1199 % 256 = 175
        assert_eq!(dst_y[159], ((239 + 60 * 16) % 256) as u8);
    }

    #[test]
    fn explicit_offset_crop_extracts_correct_region() {
        let frame = make_position_frame(64, 64);
        // Crop top-left 32x32
        let cropper = CropFilter::new(32, 32, Some(0), Some(0)).unwrap();
        let cropped = cropper.process(&frame).unwrap();
        let dst_y = cropped.plane(0).unwrap();

        // Verify each pixel matches the source's top-left region
        for y in 0..32 {
            for x in 0..32 {
                let expected = ((x + y * 16) % 256) as u8;
                assert_eq!(
                    dst_y[y * 32 + x],
                    expected,
                    "mismatch at ({}, {})",
                    x,
                    y
                );
            }
        }
    }

    #[test]
    fn bottom_right_corner_crop() {
        let frame = make_position_frame(64, 64);
        // Crop bottom-right 32x32 (offset 32, 32)
        let cropper = CropFilter::new(32, 32, Some(32), Some(32)).unwrap();
        let cropped = cropper.process(&frame).unwrap();
        let dst_y = cropped.plane(0).unwrap();

        for y in 0..32 {
            for x in 0..32 {
                let expected = (((x + 32) + (y + 32) * 16) % 256) as u8;
                assert_eq!(
                    dst_y[y * 32 + x],
                    expected,
                    "mismatch at ({}, {})",
                    x,
                    y
                );
            }
        }
    }

    #[test]
    fn rejects_non_yuv420p_input() {
        let frame = Frame::new_video(64, 64, PixelFormat::NV12);
        let cropper = CropFilter::new(32, 32, None, None).unwrap();
        assert!(cropper.process(&frame).is_err());
    }

    #[test]
    fn preserves_pts_and_duration() {
        let mut frame = make_solid_frame(64, 64, 100);
        frame.set_pts(Some(98765));
        frame = frame.with_duration(43210);

        let cropper = CropFilter::new(32, 32, None, None).unwrap();
        let cropped = cropper.process(&frame).unwrap();

        assert_eq!(cropped.pts(), Some(98765));
        assert_eq!(cropped.duration(), Some(43210));
    }

    #[test]
    fn chroma_planes_are_correctly_cropped() {
        // Create a frame with distinct U and V to verify chroma cropping
        let mut frame = Frame::new_video(64, 64, PixelFormat::YUV420P);
        let y_plane = frame.plane_mut(0).unwrap();
        // Y plane: gradient
        for y in 0..64 {
            for x in 0..64 {
                y_plane[y * 64 + x] = (x + y) as u8;
            }
        }
        // U plane: half-resolution gradient (32x32)
        let u_plane = frame.plane_mut(1).unwrap();
        for y in 0..32 {
            for x in 0..32 {
                u_plane[y * 32 + x] = (x * 4) as u8;
            }
        }
        // V plane: similar
        let v_plane = frame.plane_mut(2).unwrap();
        for y in 0..32 {
            for x in 0..32 {
                v_plane[y * 32 + x] = (y * 4) as u8;
            }
        }

        // Crop bottom-right 32x32 (offset 32, 32 in Y; offset 16, 16 in chroma)
        let cropper = CropFilter::new(32, 32, Some(32), Some(32)).unwrap();
        let cropped = cropper.process(&frame).unwrap();

        // U plane in output is 16x16, source region was U[16..32][16..32]
        let dst_u = cropped.plane(1).unwrap();
        assert_eq!(dst_u.len(), 16 * 16);
        // U[0][0] of cropped = source U[16][16] = 16*4 = 64
        assert_eq!(dst_u[0], 64);
        // U[0][15] of cropped = source U[16][31] = 31*4 = 124
        assert_eq!(dst_u[15], 124);

        // V plane similar
        let dst_v = cropped.plane(2).unwrap();
        assert_eq!(dst_v.len(), 16 * 16);
        // V[0][0] of cropped = source V[16][16] = 16*4 = 64
        assert_eq!(dst_v[0], 64);
        // V[15][0] of cropped = source V[31][16] = 31*4 = 124
        assert_eq!(dst_v[15 * 16], 124);
    }

    #[test]
    fn cropping_solid_color_does_not_affect_mean() {
        let frame = make_solid_frame(64, 64, 150);
        let cropper = CropFilter::new(32, 32, Some(16), Some(16)).unwrap();
        let cropped = cropper.process(&frame).unwrap();
        assert_eq!(mean_y(&cropped), 150.0);
    }
}
