//! SSIM (Structural Similarity Index) computation
//!
//! Computes SSIM between two video frames for quality assessment.
//! Higher SSIM (closer to 1.0) means higher similarity.

use rust_media_core::Frame;

/// Constants for SSIM calculation
const SSIM_K1: f64 = 0.01;
const SSIM_K2: f64 = 0.03;
const SSIM_L: f64 = 255.0; // Dynamic range for 8-bit images

/// Calculate SSIM between two image planes (Y, U, or V)
///
/// SSIM formula:
/// SSIM(x, y) = (2*μx*μy + C1)(2*σxy + C2) / ((μx² + μy² + C1)(σx² + σy² + C2))
fn calculate_ssim_plane(plane1: &[u8], plane2: &[u8], width: usize, height: usize) -> f64 {
    if plane1.len() != plane2.len() || plane1.len() != width * height {
        return 0.0;
    }

    let n = (width * height) as f64;
    if n == 0.0 {
        return 1.0;
    }

    // Calculate means
    let sum1: f64 = plane1.iter().map(|&x| x as f64).sum();
    let sum2: f64 = plane2.iter().map(|&x| x as f64).sum();
    let mean1 = sum1 / n;
    let mean2 = sum2 / n;

    // Calculate variances and covariance
    let mut var1 = 0.0;
    let mut var2 = 0.0;
    let mut covar = 0.0;

    for i in 0..plane1.len() {
        let diff1 = plane1[i] as f64 - mean1;
        let diff2 = plane2[i] as f64 - mean2;
        var1 += diff1 * diff1;
        var2 += diff2 * diff2;
        covar += diff1 * diff2;
    }

    var1 /= n;
    var2 /= n;
    covar /= n;

    // SSIM constants
    let c1 = (SSIM_K1 * SSIM_L).powi(2);
    let c2 = (SSIM_K2 * SSIM_L).powi(2);

    // SSIM formula
    let numerator = (2.0 * mean1 * mean2 + c1) * (2.0 * covar + c2);
    let denominator = (mean1.powi(2) + mean2.powi(2) + c1) * (var1 + var2 + c2);

    if denominator == 0.0 {
        return 1.0; // Identical images
    }

    numerator / denominator
}

/// Calculate SSIM for a YUV420P frame pair.
///
/// Returns `(ssim_y, ssim_u, ssim_v, ssim_avg)` where the average uses
/// perceptual weighting: `(6*Y + U + V) / 8`.
pub fn calculate_frame_ssim(
    frame1: &Frame,
    frame2: &Frame,
) -> Result<(f64, f64, f64, f64), String> {
    let params1 = frame1
        .video_params()
        .ok_or("Frame 1 is not a video frame")?;
    let params2 = frame2
        .video_params()
        .ok_or("Frame 2 is not a video frame")?;
    let (width1, height1) = (params1.width, params1.height);
    let (width2, height2) = (params2.width, params2.height);

    if width1 != width2 || height1 != height2 {
        return Err(format!(
            "Frame dimensions mismatch: {}x{} vs {}x{}",
            width1, height1, width2, height2
        ));
    }

    let y1 = frame1.plane(0).ok_or("Frame 1 missing Y plane")?;
    let y2 = frame2.plane(0).ok_or("Frame 2 missing Y plane")?;
    let u1 = frame1.plane(1).ok_or("Frame 1 missing U plane")?;
    let u2 = frame2.plane(1).ok_or("Frame 2 missing U plane")?;
    let v1 = frame1.plane(2).ok_or("Frame 1 missing V plane")?;
    let v2 = frame2.plane(2).ok_or("Frame 2 missing V plane")?;

    let uv_width = width1 / 2;
    let uv_height = height1 / 2;

    let ssim_y = calculate_ssim_plane(y1, y2, width1, height1);
    let ssim_u = calculate_ssim_plane(u1, u2, uv_width, uv_height);
    let ssim_v = calculate_ssim_plane(v1, v2, uv_width, uv_height);

    // Weighted average (Y is more perceptually important)
    let ssim_avg = (6.0 * ssim_y + ssim_u + ssim_v) / 8.0;

    Ok((ssim_y, ssim_u, ssim_v, ssim_avg))
}

/// Convert SSIM to dB scale
/// dB = -10 * log10(1 - SSIM)
pub fn ssim_to_db(ssim: f64) -> f64 {
    if ssim >= 1.0 {
        return f64::INFINITY;
    }
    if ssim <= 0.0 {
        return 0.0;
    }
    -10.0 * (1.0 - ssim).log10()
}
