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

/// x264 speed preset (maps to x264 presets ultrafast..placebo).
///
/// The numeric value 0..9 maps to:
/// 0 = ultrafast, 1 = superfast, 2 = veryfast, 3 = faster, 4 = fast,
/// 5 = medium, 6 = slow, 7 = slower, 8 = veryslow, 9 = placebo.
fn speed_to_preset(speed: u8) -> Preset {
    match speed {
        0 => Preset::Ultrafast,
        1 => Preset::Superfast,
        2 => Preset::Veryfast,
        3 => Preset::Faster,
        4 => Preset::Fast,
        5 => Preset::Medium,
        6 => Preset::Slow,
        7 => Preset::Slower,
        8 => Preset::Veryslow,
        _ => Preset::Placebo,
    }
}

/// H.264 encoder configuration with builder pattern.
///
/// # Example
///
/// ```rust,ignore
/// let config = X264EncoderConfig::new()
///     .speed(3)     // "faster" preset
///     .crf(23.0)
///     .gop_size(250);
/// let encoder = X264Encoder::with_config(stream_info, config)?;
/// ```
#[derive(Debug, Clone)]
pub struct X264EncoderConfig {
    /// Speed preset 0..9 (0 = ultrafast, 5 = medium, 9 = placebo).
    /// Default: 5 (medium).
    pub speed: u8,
    /// CRF (Constant Rate Factor) quality value. Lower = better quality.
    /// Typical range 18..28. When set, bitrate is used as a VBV cap.
    /// Default: None (uses bitrate-based ABR instead).
    pub crf: Option<f32>,
    /// Maximum keyframe interval in frames. Default: 250.
    pub gop_size: u32,
    /// Minimum keyframe interval in frames. Default: x264 auto.
    pub keyint_min: Option<u32>,
}

impl Default for X264EncoderConfig {
    fn default() -> Self {
        Self {
            speed: 5,
            crf: None,
            gop_size: 250,
            keyint_min: None,
        }
    }
}

impl X264EncoderConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn speed(mut self, v: u8) -> Self {
        self.speed = v.min(9);
        self
    }

    pub fn crf(mut self, v: f32) -> Self {
        self.crf = Some(v);
        self
    }

    pub fn gop_size(mut self, v: u32) -> Self {
        self.gop_size = v;
        self
    }

    pub fn keyint_min(mut self, v: u32) -> Self {
        self.keyint_min = Some(v);
        self
    }
}

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
    /// Cached avcC (AVCDecoderConfigurationRecord) built from SPS/PPS at
    /// construction time. Used by `codec_config()` so muxers can write the
    /// `avcC` box before any frames are encoded.
    avcc_record: Vec<u8>,
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
    /// Create with default config. Honors `stream_info.bitrate` if set.
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        Self::with_config(stream_info, X264EncoderConfig::default())
    }

    /// Creates an H.264 encoder with custom bitrate
    pub fn with_bitrate(mut stream_info: StreamInfo, bitrate: u64) -> Result<Self> {
        stream_info.bitrate = Some(bitrate);
        Self::new(stream_info)
    }

    /// Create with explicit configuration.
    pub fn with_config(stream_info: StreamInfo, config: X264EncoderConfig) -> Result<Self> {
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
        let (tb_num, tb_den) = stream_info.time_base;
        let fps_num = tb_den;
        let fps_den = tb_num;

        let preset = speed_to_preset(config.speed);

        // Create encoder using builder pattern
        let mut setup = Setup::preset(preset, Tune::None, false, false)
            .fps(fps_num, fps_den)
            .timebase(tb_num, tb_den)
            .bitrate(bitrate_kbps)
            .max_keyframe_interval(config.gop_size as i32)
            .annexb(true)
            .high();

        if let Some(min) = config.keyint_min {
            setup = setup.min_keyframe_interval(min as i32);
        }

        let mut encoder = setup
            .build(Colorspace::I420, width, height)
            .map_err(|e| Error::Encode(format!("Failed to create x264 encoder: {:?}", e)))?;

        // Build avcC record from SPS/PPS headers. The headers are fully
        // determined by the encoder config and available immediately.
        let avcc_record = Self::build_avcc_from_encoder(&mut encoder)?;

        Ok(Self {
            stream_info,
            encoder: Some(encoder),
            width,
            height,
            frame_count: 0,
            buffered_packets: Vec::new(),
            flushed: false,
            headers_emitted: false,
            avcc_record,
        })
    }

    /// Returns the avcC (AVCDecoderConfigurationRecord) for container muxing.
    ///
    /// The record is built from the encoder's SPS/PPS headers at construction
    /// time, so it can be called before any frames are sent. The bytes are
    /// suitable for the MP4 `avcC` box or MKV CodecPrivate.
    pub fn codec_config(&self) -> &[u8] {
        &self.avcc_record
    }

    /// Extract SPS/PPS NAL units from the x264 encoder headers and build an
    /// AVCDecoderConfigurationRecord (avcC).
    fn build_avcc_from_encoder(encoder: &mut X264EncoderInner) -> Result<Vec<u8>> {
        let headers = encoder
            .headers()
            .map_err(|e| Error::Encode(format!("Failed to get x264 headers: {:?}", e)))?;

        let mut sps_list: Vec<Vec<u8>> = Vec::new();
        let mut pps_list: Vec<Vec<u8>> = Vec::new();

        for i in 0..headers.len() {
            let unit = headers.unit(i);
            let payload: &[u8] = unit.as_ref();

            // Strip Annex B start code (00 00 00 01 or 00 00 01)
            let nal = if payload.starts_with(&[0, 0, 0, 1]) {
                &payload[4..]
            } else if payload.starts_with(&[0, 0, 1]) {
                &payload[3..]
            } else {
                payload
            };

            if nal.is_empty() {
                continue;
            }

            match nal[0] & 0x1F {
                7 => sps_list.push(nal.to_vec()), // SPS
                8 => pps_list.push(nal.to_vec()), // PPS
                _ => {}
            }
        }

        if sps_list.is_empty() {
            return Err(Error::Encode(
                "x264 encoder produced no SPS in headers".to_string(),
            ));
        }

        let sps = &sps_list[0];
        if sps.len() < 4 {
            return Err(Error::Encode("SPS too short for avcC".to_string()));
        }

        // Build AVCDecoderConfigurationRecord
        let mut avcc = vec![
            1,      // configurationVersion
            sps[1], // AVCProfileIndication
            sps[2], // profile_compatibility
            sps[3], // AVCLevelIndication
            0xFF,   // reserved(6 bits) + lengthSizeMinusOne(2 bits) = 3 → 4-byte lengths
            0xE0 | (sps_list.len() as u8), // reserved(3 bits) + numOfSequenceParameterSets
        ];
        for s in &sps_list {
            let len = s.len() as u16;
            avcc.extend_from_slice(&len.to_be_bytes());
            avcc.extend_from_slice(s);
        }
        avcc.push(pps_list.len() as u8); // numOfPictureParameterSets
        for p in &pps_list {
            let len = p.len() as u16;
            avcc.extend_from_slice(&len.to_be_bytes());
            avcc.extend_from_slice(p);
        }

        Ok(avcc)
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
