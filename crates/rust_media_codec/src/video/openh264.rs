//! H.264/AVC video decoder implementation using OpenH264
//!
//! Provides H.264 decoding via the OpenH264 library (Cisco's open source implementation).
//!
//! # Licensing
//!
//! OpenH264 is licensed under BSD-2-Clause, making it compatible with the project's
//! MIT/Apache-2.0 default license. No special feature flags are required.
//!
//! # Current Implementation Status
//!
//! ## Decoder
//! - ✅ Fully implemented with YUV420P (I420) output
//! - ✅ Supports NAL unit and bitstream input
//! - ✅ Proper flush handling for B-frames and buffered data
//!
//! # Example
//!
//! ```rust,ignore
//! use rust_media_core::{StreamInfo, MediaType, VideoStreamParams, PixelFormat};
//! use rust_media_codec::H264Decoder;
//!
//! let video_params = VideoStreamParams::new(1920, 1080, PixelFormat::YUV420P);
//! let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
//!     .with_time_base(1, 90000)
//!     .with_params(rust_media_core::StreamParams::Video(video_params));
//!
//! let mut decoder = H264Decoder::new(stream_info)?;
//! // ... decode packets using send_packet() and receive_frame()
//! ```

use openh264::decoder::{Decoder as OpenH264DecoderInternal, DecodedYUV};
use openh264::formats::YUVSource;
use rust_media_core::{
    Decoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
};

/// H.264/AVC video decoder using OpenH264
///
/// Decodes H.264-compressed video packets into raw YUV frames.
///
/// # Notes
///
/// - Input packets should contain NAL units (with or without start codes)
/// - Output is always YUV420P (I420) format
/// - The decoder handles B-frames internally; use `flush()` to retrieve buffered frames
pub struct H264Decoder {
    stream_info: StreamInfo,
    decoder: OpenH264DecoderInternal,
    buffered_frames: Vec<Frame>,
    flushed: bool,
}

impl H264Decoder {
    /// Creates a new H.264 decoder from stream information
    ///
    /// # Arguments
    ///
    /// * `stream_info` - Stream information (codec must be "h264" or "avc")
    ///
    /// # Returns
    ///
    /// A new H264Decoder or an error if initialization fails
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec - accept both "h264" and "avc" (common alternative names)
        if stream_info.codec != "h264" && stream_info.codec != "avc" {
            return Err(Error::Unsupported(format!(
                "Expected h264/avc codec, got {}",
                stream_info.codec
            )));
        }

        // Create decoder
        let decoder = OpenH264DecoderInternal::new()
            .map_err(|e| Error::Decode(format!("Failed to create H.264 decoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            decoder,
            buffered_frames: Vec::new(),
            flushed: false,
        })
    }

}

/// Converts a DecodedYUV to a Frame (standalone function to avoid borrow issues)
fn decoded_yuv_to_frame(yuv: &DecodedYUV, pts: Option<i64>) -> Result<Frame> {
    let (width, height) = yuv.dimensions();

    let mut frame = Frame::new_video(width, height, PixelFormat::YUV420P);
    frame.set_pts(pts);

    // Get strides for each plane
    let (y_stride, u_stride, v_stride) = yuv.strides();

    // Get Y, U, V data via YUVSource trait
    let y_data = yuv.y();
    let u_data = yuv.u();
    let v_data = yuv.v();

    // Copy Y plane (full resolution)
    let y_frame_plane = frame
        .plane_mut(0)
        .ok_or_else(|| Error::InvalidData("Failed to get Y plane".to_string()))?;
    for row in 0..height {
        let src_start = row * y_stride;
        let src_end = src_start + width;
        let dst_start = row * width;
        let dst_end = dst_start + width;
        y_frame_plane[dst_start..dst_end].copy_from_slice(&y_data[src_start..src_end]);
    }

    // Copy U plane (half resolution)
    let uv_height = height / 2;
    let uv_width = width / 2;

    let u_frame_plane = frame
        .plane_mut(1)
        .ok_or_else(|| Error::InvalidData("Failed to get U plane".to_string()))?;
    for row in 0..uv_height {
        let src_start = row * u_stride;
        let src_end = src_start + uv_width;
        let dst_start = row * uv_width;
        let dst_end = dst_start + uv_width;
        u_frame_plane[dst_start..dst_end].copy_from_slice(&u_data[src_start..src_end]);
    }

    // Copy V plane (half resolution)
    let v_frame_plane = frame
        .plane_mut(2)
        .ok_or_else(|| Error::InvalidData("Failed to get V plane".to_string()))?;
    for row in 0..uv_height {
        let src_start = row * v_stride;
        let src_end = src_start + uv_width;
        let dst_start = row * uv_width;
        let dst_end = dst_start + uv_width;
        v_frame_plane[dst_start..dst_end].copy_from_slice(&v_data[src_start..src_end]);
    }

    Ok(frame)
}

impl Decoder for H264Decoder {
    fn codec(&self) -> &str {
        "h264"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if packet.media_type() != MediaType::Video {
            return Err(Error::InvalidData(format!(
                "Expected video packet, got {:?}",
                packet.media_type()
            )));
        }

        let pts = packet.pts();
        let data = packet.data();

        // Decode the packet
        let decode_result = self.decoder.decode(data);
        match decode_result {
            Ok(Some(yuv)) => {
                let frame = decoded_yuv_to_frame(&yuv, pts)?;
                self.buffered_frames.push(frame);
            }
            Ok(None) => {
                // No frame available yet (need more data or buffered for reordering)
            }
            Err(e) => {
                // OpenH264 can sometimes fail on corrupted data but recover
                // Log the error but don't fail entirely unless we want strict mode
                return Err(Error::Decode(format!("H.264 decode error: {:?}", e)));
            }
        }

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if !self.buffered_frames.is_empty() {
            Ok(self.buffered_frames.remove(0))
        } else if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        // Flush remaining frames from the decoder
        let flush_result = self.decoder.flush_remaining();
        match flush_result {
            Ok(remaining_frames) => {
                for yuv in remaining_frames {
                    if let Ok(frame) = decoded_yuv_to_frame(&yuv, None) {
                        self.buffered_frames.push(frame);
                    }
                }
            }
            Err(e) => {
                // Log error but don't fail - some frames may still be recoverable
                eprintln!("Warning: H.264 flush error: {:?}", e);
            }
        }

        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        // Create a new decoder instance for reset
        self.decoder = OpenH264DecoderInternal::new()
            .map_err(|e| Error::Decode(format!("Failed to reset H.264 decoder: {:?}", e)))?;
        self.buffered_frames.clear();
        self.flushed = false;
        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_media_core::VideoStreamParams;

    #[test]
    fn test_h264_decoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_h264_decoder_accepts_avc() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "avc".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_h264_decoder_wrong_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info);
        assert!(decoder.is_err());
    }

    #[test]
    fn test_h264_decoder_codec_name() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 90000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = H264Decoder::new(stream_info).unwrap();
        assert_eq!(decoder.codec(), "h264");
    }
}
