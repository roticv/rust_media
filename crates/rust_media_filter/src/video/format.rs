//! Pixel format conversion filters
//!
//! Currently supports:
//! - YUV420P10LE → YUV420P (10-bit to 8-bit downconversion)
//!
//! 10-bit samples are stored as little-endian u16 values in the range [0, 1023].
//! 8-bit samples are u8 in [0, 255]. The conversion right-shifts by 2 bits
//! (equivalent to dividing by 4 and rounding down).

use rust_media_core::{Frame, PixelFormat};

/// Convert a YUV420P10LE frame to YUV420P (10-bit → 8-bit downconversion).
///
/// Each 10-bit sample (0..1023) is right-shifted by 2 to become an 8-bit
/// sample (0..255). This is the standard "drop the bottom 2 bits" approach.
///
/// Returns the input frame unchanged if it's already YUV420P.
pub fn yuv420p10le_to_yuv420p(frame: &Frame) -> Result<Frame, Box<dyn std::error::Error>> {
    let params = frame.video_params().ok_or("Not a video frame")?;

    // Already 8-bit — no work to do
    if params.format == PixelFormat::YUV420P {
        return Ok(frame.clone());
    }

    if params.format != PixelFormat::YUV420P10LE {
        return Err(format!(
            "yuv420p10le_to_yuv420p: input must be YUV420P10LE, got {:?}",
            params.format
        )
        .into());
    }

    let width = params.width;
    let height = params.height;
    let mut out = Frame::new_video(width, height, PixelFormat::YUV420P);

    // Convert each plane: pairs of bytes (little-endian u16) → single byte
    let convert = |src: &[u8], dst: &mut [u8]| {
        for (i, dst_byte) in dst.iter_mut().enumerate() {
            let sample =
                u16::from_le_bytes([src[i * 2], src[i * 2 + 1]]);
            // Right-shift by 2 to drop the bottom 2 bits.
            // Clamp to 8-bit range in case of out-of-spec input.
            *dst_byte = ((sample >> 2).min(255)) as u8;
        }
    };

    // Y plane (full resolution)
    {
        let src_y = frame.plane(0).ok_or("missing Y plane")?;
        let src_y_owned: Vec<u8> = src_y.to_vec();
        let dst_y = out.plane_mut(0).ok_or("missing output Y plane")?;
        convert(&src_y_owned, dst_y);
    }

    // U plane (half resolution)
    {
        let src_u = frame.plane(1).ok_or("missing U plane")?;
        let src_u_owned: Vec<u8> = src_u.to_vec();
        let dst_u = out.plane_mut(1).ok_or("missing output U plane")?;
        convert(&src_u_owned, dst_u);
    }

    // V plane (half resolution)
    {
        let src_v = frame.plane(2).ok_or("missing V plane")?;
        let src_v_owned: Vec<u8> = src_v.to_vec();
        let dst_v = out.plane_mut(2).ok_or("missing output V plane")?;
        convert(&src_v_owned, dst_v);
    }

    out.set_pts(frame.pts());
    if let Some(dur) = frame.duration() {
        out = out.with_duration(dur);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_solid_10bit_frame(width: usize, height: usize, y10: u16) -> Frame {
        let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P10LE);
        let bytes = y10.to_le_bytes();
        let y_plane = frame.plane_mut(0).unwrap();
        for chunk in y_plane.chunks_exact_mut(2) {
            chunk[0] = bytes[0];
            chunk[1] = bytes[1];
        }
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

    #[test]
    fn pass_through_8bit() {
        let frame = Frame::new_video(64, 64, PixelFormat::YUV420P);
        let out = yuv420p10le_to_yuv420p(&frame).unwrap();
        assert_eq!(out.video_params().unwrap().format, PixelFormat::YUV420P);
    }

    #[test]
    fn converts_solid_10bit_to_8bit() {
        // 10-bit value 800 → 800 >> 2 = 200
        let frame = make_solid_10bit_frame(64, 64, 800);
        let out = yuv420p10le_to_yuv420p(&frame).unwrap();
        let params = out.video_params().unwrap();
        assert_eq!(params.format, PixelFormat::YUV420P);
        assert_eq!(params.width, 64);
        assert_eq!(params.height, 64);

        let y = out.plane(0).unwrap();
        assert_eq!(y.len(), 64 * 64);
        for &pixel in y {
            assert_eq!(pixel, 200);
        }

        // U/V neutral chroma: 512 >> 2 = 128
        let u = out.plane(1).unwrap();
        for &pixel in u {
            assert_eq!(pixel, 128);
        }
    }

    #[test]
    fn converts_max_10bit_clamped() {
        // Max 10-bit = 1023 → 1023 >> 2 = 255
        let frame = make_solid_10bit_frame(64, 64, 1023);
        let out = yuv420p10le_to_yuv420p(&frame).unwrap();
        let y = out.plane(0).unwrap();
        for &pixel in y {
            assert_eq!(pixel, 255);
        }
    }

    #[test]
    fn converts_zero() {
        let frame = make_solid_10bit_frame(64, 64, 0);
        let out = yuv420p10le_to_yuv420p(&frame).unwrap();
        let y = out.plane(0).unwrap();
        for &pixel in y {
            assert_eq!(pixel, 0);
        }
    }

    #[test]
    fn rejects_non_yuv420p10le() {
        let frame = Frame::new_video(64, 64, PixelFormat::NV12);
        assert!(yuv420p10le_to_yuv420p(&frame).is_err());
    }

    #[test]
    fn preserves_pts_and_duration() {
        let mut frame = make_solid_10bit_frame(64, 64, 400);
        frame.set_pts(Some(12345));
        frame = frame.with_duration(67890);
        let out = yuv420p10le_to_yuv420p(&frame).unwrap();
        assert_eq!(out.pts(), Some(12345));
        assert_eq!(out.duration(), Some(67890));
    }
}
