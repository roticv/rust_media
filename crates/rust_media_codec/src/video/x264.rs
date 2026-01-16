//! H.264/AVC video encoder implementation using x264
//!
//! Provides H.264 encoding via the x264 library bindings.
//!
//! # License Warning
//!
//! **x264 is licensed under GPL v2+**. Using this encoder means your compiled
//! binary becomes GPL-licensed. This module is only available when the
//! `gpl-x264` feature is enabled.
//!
//! # Usage
//!
//! ```rust,ignore
//! use rust_media_core::{StreamInfo, MediaType, VideoStreamParams, PixelFormat};
//! use rust_media_codec::X264Encoder;
//!
//! let video_params = VideoStreamParams::new(1920, 1080, PixelFormat::YUV420P);
//! let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
//!     .with_time_base(1, 30)  // 30 fps
//!     .with_bitrate(5_000_000);  // 5 Mbps
//!
//! let encoder = X264Encoder::new(stream_info)?;
//! ```
//!
//! # Encoder Configuration
//!
//! Settings extracted from `StreamInfo`:
//! - **Codec**: Must be "h264" or "avc"
//! - **Dimensions**: From `VideoStreamParams`
//! - **Bitrate**: From `StreamInfo.bitrate` (converted to kbps)
//! - **Frame rate**: Derived from `StreamInfo.time_base`
//!
//! Default settings:
//! - **Preset**: Medium (balanced speed/quality)
//! - **Profile**: High (maximum features/quality)
//! - **Tune**: None
//! - **Max keyframe interval**: 250 frames

use rust_media_core::{
    Encoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
};
use rust_media_core::frame::FrameParams;
use x264::{Colorspace, Encoder as X264EncoderInner, Image, Plane, Preset, Setup, Tune};

/// H.264/AVC video encoder using x264
///
/// This encoder is only available when the `gpl-x264` feature is enabled.
/// Using this encoder makes your binary GPL-licensed.
///
/// # Example
///
/// ```rust,ignore
/// use rust_media_core::{StreamInfo, MediaType, VideoStreamParams, PixelFormat, Encoder, Frame};
/// use rust_media_codec::X264Encoder;
///
/// // Create stream info for H.264 encoding
/// let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
/// let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
///     .with_time_base(1, 30)
///     .with_bitrate(2_000_000)
///     .with_params(rust_media_core::StreamParams::Video(video_params));
///
/// // Create encoder
/// let mut encoder = X264Encoder::new(stream_info)?;
///
/// // Encode frames
/// let frame = Frame::new_video(640, 480, PixelFormat::YUV420P);
/// encoder.send_frame(&frame)?;
///
/// // Receive encoded packets
/// while let Ok(packet) = encoder.receive_packet() {
///     // Process encoded H.264 packet
/// }
/// ```
pub struct X264Encoder {
    stream_info: StreamInfo,
    // Wrapped in Option because flush() consumes the encoder
    encoder: Option<X264EncoderInner>,
    width: i32,
    height: i32,
    frame_count: i64,
    buffered_packets: Vec<Packet>,
    flushed: bool,
    headers_emitted: bool,
}

impl X264Encoder {
    /// Creates a new H.264 encoder from stream information
    ///
    /// # Arguments
    ///
    /// * `stream_info` - Stream configuration including codec, dimensions, bitrate
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Codec is not "h264" or "avc"
    /// - Stream params are not video
    /// - x264 encoder creation fails
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec
        if stream_info.codec != "h264" && stream_info.codec != "avc" {
            return Err(Error::Unsupported(format!(
                "Expected h264 or avc codec, got {}",
                stream_info.codec
            )));
        }

        // Get video parameters
        let video_params = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => params,
            _ => {
                return Err(Error::InvalidData(
                    "H.264 encoder requires video stream parameters".to_string(),
                ))
            }
        };

        let width = video_params.width as i32;
        let height = video_params.height as i32;

        // Calculate bitrate in kbps (x264 uses kbps)
        let bitrate_kbps = (stream_info.bitrate.unwrap_or(2_000_000) / 1000) as i32;

        // Calculate frame rate from time_base
        // time_base is (num, den) where pts * num / den = seconds
        // For fps, we want den / num (inverted)
        let (tb_num, tb_den) = stream_info.time_base;
        let fps_num = tb_den;
        let fps_den = tb_num;

        // Create encoder using builder pattern
        let encoder = Setup::preset(Preset::Medium, Tune::None, false, false)
            .fps(fps_num, fps_den)
            .timebase(tb_num, tb_den)
            .bitrate(bitrate_kbps)
            .max_keyframe_interval(250) // ~10 seconds at 25fps
            .annexb(true) // Use Annex B format (start codes)
            .high() // Use high profile for best quality
            .build(Colorspace::I420, width, height)
            .map_err(|e| Error::Encode(format!("Failed to create x264 encoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            encoder: Some(encoder),
            width,
            height,
            frame_count: 0,
            buffered_packets: Vec::new(),
            flushed: false,
            headers_emitted: false,
        })
    }

    /// Creates an H.264 encoder with custom bitrate
    ///
    /// # Arguments
    ///
    /// * `stream_info` - Stream configuration
    /// * `bitrate` - Target bitrate in bits per second
    pub fn with_bitrate(mut stream_info: StreamInfo, bitrate: u64) -> Result<Self> {
        stream_info.bitrate = Some(bitrate);
        Self::new(stream_info)
    }

    /// Emits SPS/PPS headers as the first packet if not already done
    fn emit_headers_if_needed(&mut self) -> Result<()> {
        if self.headers_emitted {
            return Ok(());
        }

        let encoder = self.encoder.as_mut().ok_or_else(|| {
            Error::Encode("Encoder has been flushed".to_string())
        })?;

        // Get SPS/PPS headers from encoder
        let headers = encoder
            .headers()
            .map_err(|e| Error::Encode(format!("Failed to get x264 headers: {:?}", e)))?;

        // Collect all header NAL units into a single buffer
        let mut header_data = Vec::new();
        for i in 0..headers.len() {
            let unit = headers.unit(i);
            // Use AsRef<[u8]> to get the NAL unit data
            header_data.extend_from_slice(unit.as_ref());
        }

        if !header_data.is_empty() {
            let mut pkt = Packet::new(header_data, 0, MediaType::Video);
            pkt.set_pts(Some(0));
            pkt.set_keyframe(true); // Headers are always associated with keyframes
            self.buffered_packets.push(pkt);
        }

        self.headers_emitted = true;
        Ok(())
    }
}

impl Encoder for X264Encoder {
    fn codec(&self) -> &str {
        "h264"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        // Ensure frame is video
        if frame.media_type() != MediaType::Video {
            return Err(Error::InvalidData(format!(
                "Expected video frame, got {:?}",
                frame.media_type()
            )));
        }

        // Get frame parameters
        let frame_params = match frame.params() {
            FrameParams::Video(params) => params,
            _ => {
                return Err(Error::InvalidData(
                    "H.264 encoder requires video frame parameters".to_string(),
                ))
            }
        };

        // H.264 with x264 requires YUV420P format (I420)
        if frame_params.format != PixelFormat::YUV420P {
            return Err(Error::Unsupported(format!(
                "H.264 encoder only supports YUV420P, got {:?}",
                frame_params.format
            )));
        }

        // Validate dimensions match
        if frame_params.width as i32 != self.width || frame_params.height as i32 != self.height {
            return Err(Error::InvalidData(format!(
                "Frame dimensions {}x{} don't match encoder {}x{}",
                frame_params.width, frame_params.height, self.width, self.height
            )));
        }

        // Emit headers on first frame
        self.emit_headers_if_needed()?;

        // Get plane data
        let y_plane = frame
            .plane(0)
            .ok_or_else(|| Error::InvalidData("Failed to get Y plane".to_string()))?;
        let u_plane = frame
            .plane(1)
            .ok_or_else(|| Error::InvalidData("Failed to get U plane".to_string()))?;
        let v_plane = frame
            .plane(2)
            .ok_or_else(|| Error::InvalidData("Failed to get V plane".to_string()))?;

        // Calculate strides (for planar YUV420P, stride = width for Y, width/2 for U/V)
        let y_stride = self.width;
        let uv_stride = self.width / 2;

        // Create x264 planes
        let planes = [
            Plane {
                stride: y_stride,
                data: y_plane,
            },
            Plane {
                stride: uv_stride,
                data: u_plane,
            },
            Plane {
                stride: uv_stride,
                data: v_plane,
            },
        ];

        // Create x264 image
        let image = Image::new(Colorspace::I420, self.width, self.height, &planes);

        // Get PTS from frame or use frame count
        let pts = frame.pts().unwrap_or(self.frame_count);

        // Encode the frame
        let encoder = self.encoder.as_mut().ok_or_else(|| {
            Error::Encode("Encoder has been flushed".to_string())
        })?;

        let (data, picture) = encoder
            .encode(pts, image)
            .map_err(|e| Error::Encode(format!("x264 encode failed: {:?}", e)))?;

        // Convert to packet
        let encoded_data: Vec<u8> = data.entirety().to_vec();

        if !encoded_data.is_empty() {
            let mut pkt = Packet::new(encoded_data, 0, MediaType::Video);
            pkt.set_pts(Some(picture.pts()));
            pkt.set_dts(Some(picture.dts()));

            // Check if this is a keyframe (IDR frame)
            if picture.keyframe() {
                pkt.set_keyframe(true);
            }

            self.buffered_packets.push(pkt);
        }

        self.frame_count += 1;

        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        if !self.buffered_packets.is_empty() {
            Ok(self.buffered_packets.remove(0))
        } else if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        // Take ownership of encoder for flushing (flush() consumes it)
        let encoder = self.encoder.take().ok_or_else(|| {
            Error::Encode("Encoder already flushed".to_string())
        })?;

        // Flush remaining frames from encoder
        let mut flush = encoder.flush();
        while let Some(result) = flush.next() {
            let (data, picture) = result
                .map_err(|e| Error::Encode(format!("x264 flush failed: {:?}", e)))?;

            let encoded_data: Vec<u8> = data.entirety().to_vec();

            if !encoded_data.is_empty() {
                let mut pkt = Packet::new(encoded_data, 0, MediaType::Video);
                pkt.set_pts(Some(picture.pts()));
                pkt.set_dts(Some(picture.dts()));

                if picture.keyframe() {
                    pkt.set_keyframe(true);
                }

                self.buffered_packets.push(pkt);
            }
        }

        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.frame_count = 0;
        self.buffered_packets.clear();
        self.flushed = false;
        self.headers_emitted = false;
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
    fn test_x264_encoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 30)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = X264Encoder::new(stream_info);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_x264_encoder_avc_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "avc".to_string())
            .with_time_base(1, 30)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = X264Encoder::new(stream_info);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_x264_encoder_wrong_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp8".to_string())
            .with_time_base(1, 30)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = X264Encoder::new(stream_info);
        assert!(encoder.is_err());
    }

    #[test]
    fn test_x264_encoder_with_bitrate() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 30)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = X264Encoder::with_bitrate(stream_info, 5_000_000);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_x264_encode_frame() {
        let video_params = VideoStreamParams::new(320, 240, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 30)
            .with_bitrate(1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let mut encoder = X264Encoder::new(stream_info).unwrap();

        // Create a test frame (solid gray)
        let mut frame = Frame::new_video(320, 240, PixelFormat::YUV420P);
        frame.set_pts(Some(0));

        // Fill with gray (Y=128, U=V=128)
        if let Some(y_plane) = frame.plane_mut(0) {
            y_plane.fill(128);
        }
        if let Some(u_plane) = frame.plane_mut(1) {
            u_plane.fill(128);
        }
        if let Some(v_plane) = frame.plane_mut(2) {
            v_plane.fill(128);
        }

        // Encode the frame
        let result = encoder.send_frame(&frame);
        assert!(result.is_ok());

        // Should have at least one packet (headers + first frame)
        let packet = encoder.receive_packet();
        assert!(packet.is_ok());
    }

    #[test]
    fn test_x264_flush() {
        let video_params = VideoStreamParams::new(320, 240, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "h264".to_string())
            .with_time_base(1, 30)
            .with_bitrate(1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let mut encoder = X264Encoder::new(stream_info).unwrap();

        // Create and encode a few frames
        for i in 0..5 {
            let mut frame = Frame::new_video(320, 240, PixelFormat::YUV420P);
            frame.set_pts(Some(i));

            if let Some(y_plane) = frame.plane_mut(0) {
                y_plane.fill(128);
            }
            if let Some(u_plane) = frame.plane_mut(1) {
                u_plane.fill(128);
            }
            if let Some(v_plane) = frame.plane_mut(2) {
                v_plane.fill(128);
            }

            encoder.send_frame(&frame).unwrap();
        }

        // Flush the encoder
        let flush_result = encoder.flush();
        assert!(flush_result.is_ok());
        assert!(encoder.is_flushed());

        // Drain all packets
        let mut packet_count = 0;
        while encoder.receive_packet().is_ok() {
            packet_count += 1;
        }

        // Should have multiple packets
        assert!(packet_count > 0);
    }
}
