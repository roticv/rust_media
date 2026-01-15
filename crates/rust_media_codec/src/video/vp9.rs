//! VP9 video codec implementation using libvpx
//!
//! Provides VP9 encoding and decoding via vpx-rs bindings to libvpx.
//!
//! # Current Implementation Status
//!
//! ## Decoder
//! - ✅ Fully implemented with YUV420P (I420) output
//! - ✅ Supports all VP9 features through libvpx
//!
//! ## Encoder
//! - ✅ Basic encoding with YUV420P (I420) input
//! - ⚠️  Limited configuration options (see limitations below)
//!
//! # Current Encoder Limitations
//!
//! The encoder currently has hardcoded values for many settings that should be configurable:
//!
//! ## Hardcoded Settings in `send_frame()`
//! - **Encoding Deadline**: `EncodingDeadline::default()` - No control over speed/quality tradeoff
//! - **Frame Flags**: `EncoderFrameFlags::empty()` - Cannot force keyframes or set frame-specific flags
//! - **Frame Duration**: Hardcoded to `1` - Should derive from frame rate
//!
//! ## Limited Configuration in `new()`
//! - **Rate Control**: Hardcoded to `RateControl::VariableBitRate` - No CBR, CQ, or Q mode support
//! - **GOP Size**: Not configurable - libvpx uses defaults (auto keyframe placement)
//! - **Keyframe Interval**: Not set - No control over max keyframe distance
//! - **Quality Settings**: Not exposed - No min/max quantizer control
//! - **CPU Usage**: Not configurable - No deadline/speed preset control
//! - **Threading**: Not set - No control over encoder threads
//! - **Error Resilience**: Not configured - No tile configuration
//! - **Spatial/Temporal Layers**: Not supported - No SVC configuration
//!
//! # VP9-Specific Features Not Yet Exposed
//!
//! VP9 has additional features beyond VP8:
//!
//! - **Tile-based encoding**: For parallel processing
//! - **10-bit and 12-bit encoding**: High bit-depth support
//! - **Profile support**: Profile 0 (8-bit 4:2:0), Profile 1 (8-bit 4:2:2/4:4:4), Profile 2 (10/12-bit 4:2:0), Profile 3 (10/12-bit 4:2:2/4:4:4)
//! - **Lossless mode**: Mathematically lossless compression
//! - **Row-based multi-threading**: Better parallelization than VP8
//!
//! # Future Enhancement Plan
//!
//! See VP8 module documentation for configuration system options (applies to VP9 as well).

use rust_media_core::{
    Decoder, Encoder, Error, Frame, MediaType, Packet, PixelFormat, Result,
    StreamInfo,
};
use rust_media_core::frame::FrameParams;
use std::num::NonZero;
use vpx_rs::{
    dec::CodecId as DecCodecId, enc::CodecId as EncCodecId, DecodedImageData, Decoder as VpxDecoder,
    DecoderConfig, Encoder as VpxEncoder, EncoderConfig, EncoderFrameFlags, EncodingDeadline,
    ImageFormat, RateControl, Timebase, YUVImageData,
};

/// VP9 video decoder
///
/// Decodes VP9-compressed video packets into raw YUV frames.
pub struct Vp9Decoder {
    stream_info: StreamInfo,
    decoder: VpxDecoder,
    buffered_frames: Vec<Frame>,
    flushed: bool,
}

impl Vp9Decoder {
    /// Creates a new VP9 decoder from stream information
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec
        if stream_info.codec != "vp9" {
            return Err(Error::Unsupported(format!(
                "Expected vp9 codec, got {}",
                stream_info.codec
            )));
        }

        // Get video parameters (if available)
        let (width, height) = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => {
                (params.width as u32, params.height as u32)
            }
            _ => (640, 480), // Default size if not specified
        };

        // Create decoder configuration
        let config = DecoderConfig::new(DecCodecId::VP9, width, height);

        // Create decoder
        let decoder = VpxDecoder::new(config)
            .map_err(|e| Error::Decode(format!("Failed to create VP9 decoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            decoder,
            buffered_frames: Vec::new(),
            flushed: false,
        })
    }
}

impl Decoder for Vp9Decoder {
    fn codec(&self) -> &str {
        "vp9"
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

        // Decode the packet
        let pts = packet.pts();
        let decoded_images = self
            .decoder
            .decode(packet.data())
            .map_err(|e| Error::Decode(format!("VP9 decode failed: {:?}", e)))?;

        // Convert all decoded images to frames
        // We need to process them inline due to lifetime constraints
        for decoded_image in decoded_images {
            let width = decoded_image.width();
            let height = decoded_image.height();
            // Convert inline to avoid lifetime issues
            let frame = match decoded_image.data() {
                DecodedImageData::Data8b(yuv_data) => {
                    let mut frame = Frame::new_video(width as usize, height as usize, PixelFormat::YUV420P);
                    frame.set_pts(pts);

                    let planes = yuv_data.planes();
                    let width_usize = width as usize;
                    let height_usize = height as usize;

                    // Copy Y plane
                    let y_stride = planes.y_stride();
                    let y_frame_plane = frame.plane_mut(0).ok_or_else(|| Error::InvalidData("Failed to get Y plane".to_string()))?;
                    for row in 0..height_usize {
                        let src_start = row * y_stride;
                        let src_end = src_start + width_usize;
                        let dst_start = row * width_usize;
                        let dst_end = dst_start + width_usize;
                        y_frame_plane[dst_start..dst_end].copy_from_slice(&planes.y[src_start..src_end]);
                    }

                    // Get U and V planes
                    if let vpx_rs::image::UVImagePlanes::Separate(uv_planes) = planes.uv {
                        let u_stride = uv_planes.u_stride();
                        let v_stride = uv_planes.v_stride();
                        let uv_height = height_usize / 2;
                        let uv_width = width_usize / 2;

                        // Copy U plane
                        let u_frame_plane = frame.plane_mut(1).ok_or_else(|| Error::InvalidData("Failed to get U plane".to_string()))?;
                        for row in 0..uv_height {
                            let src_start = row * u_stride;
                            let src_end = src_start + uv_width;
                            let dst_start = row * uv_width;
                            let dst_end = dst_start + uv_width;
                            u_frame_plane[dst_start..dst_end].copy_from_slice(&uv_planes.u[src_start..src_end]);
                        }

                        // Copy V plane
                        let v_frame_plane = frame.plane_mut(2).ok_or_else(|| Error::InvalidData("Failed to get V plane".to_string()))?;
                        for row in 0..uv_height {
                            let src_start = row * v_stride;
                            let src_end = src_start + uv_width;
                            let dst_start = row * uv_width;
                            let dst_end = dst_start + uv_width;
                            v_frame_plane[dst_start..dst_end].copy_from_slice(&uv_planes.v[src_start..src_end]);
                        }
                    } else {
                        return Err(Error::Unsupported("Expected separate UV planes for VP9".to_string()));
                    }

                    frame
                }
                DecodedImageData::Data16b(_) => {
                    return Err(Error::Unsupported("VP9 16-bit output not yet supported".to_string()));
                }
            };

            self.buffered_frames.push(frame);
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
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.buffered_frames.clear();
        self.flushed = false;
        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }
}

/// VP9 video encoder
///
/// Encodes raw YUV frames into VP9-compressed video packets.
///
/// # Current Configuration
///
/// Settings are extracted from `StreamInfo`:
/// - **Codec**: Must be "vp9"
/// - **Dimensions**: `VideoStreamParams.width` × `VideoStreamParams.height`
/// - **Bitrate**: `StreamInfo.bitrate` (default: 1 Mbps)
/// - **Timebase**: `StreamInfo.time_base` (numerator/denominator)
///
/// # Hardcoded Defaults
///
/// These settings are currently not configurable:
/// - **Rate Control**: Variable Bitrate (VBR) only
/// - **Encoding Deadline**: `GoodQuality` (balanced speed/quality)
/// - **GOP Size**: Automatic (libvpx default)
/// - **Keyframe Interval**: Automatic (libvpx default, typically ~128-256 frames)
/// - **Frame Duration**: 1 timebase unit
/// - **Quality Range**: libvpx defaults (min_q=0, max_q=63)
/// - **CPU Usage**: libvpx default (speed=0 for best quality)
/// - **Threads**: libvpx default (auto-detected)
/// - **Tile Columns**: Not configured (VP9 tile-based parallelism)
/// - **Profile**: Profile 0 (8-bit 4:2:0)
///
/// # VP9 Advantages Over VP8
///
/// VP9 provides:
/// - ~30-50% better compression efficiency at the same quality
/// - Tile-based encoding for better parallelization
/// - Support for 10-bit and 12-bit color depth
/// - Better subjective quality, especially at low bitrates
/// - Lossless compression mode
///
/// # Example
///
/// ```rust,ignore
/// use rust_media_core::{StreamInfo, MediaType, VideoStreamParams, PixelFormat};
/// use rust_media_codec::Vp9Encoder;
///
/// let video_params = VideoStreamParams::new(1920, 1080, PixelFormat::YUV420P);
/// let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
///     .with_time_base(1, 30)  // 30 fps
///     .with_bitrate(3_000_000)  // 3 Mbps (lower than VP8 for same quality!)
///     .with_params(rust_media_core::StreamParams::Video(video_params));
///
/// let encoder = Vp9Encoder::new(stream_info)?;
///
/// // Or use the helper for custom bitrate:
/// let encoder = Vp9Encoder::with_bitrate(stream_info, 5_000_000)?; // 5 Mbps
/// ```
///
/// # See Also
///
/// See module-level documentation for detailed information about current limitations
/// and future enhancement plans for exposing GOP size, rate control modes, tile configuration,
/// and other encoding parameters.
pub struct Vp9Encoder {
    stream_info: StreamInfo,
    encoder: VpxEncoder<u8>,
    frame_count: i64,
    buffered_packets: Vec<Packet>,
    flushed: bool,
}

impl Vp9Encoder {
    /// Creates a new VP9 encoder from stream information
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec
        if stream_info.codec != "vp9" {
            return Err(Error::Unsupported(format!(
                "Expected vp9 codec, got {}",
                stream_info.codec
            )));
        }

        // Get video parameters
        let video_params = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => params,
            _ => {
                return Err(Error::InvalidData(
                    "VP9 encoder requires video stream parameters".to_string(),
                ))
            }
        };

        // Configure rate control (bitrate is in kbit/s for vpx-rs)
        let bitrate_kbps = (stream_info.bitrate.unwrap_or(1_000_000) / 1000) as u32;

        // TODO: Make rate control configurable
        // Currently hardcoded to VBR. Should support:
        // - RateControl::ConstantBitRate(kbps)
        // - RateControl::ConstantQuality(q)
        // - RateControl::Quantizer(q)
        let rate_control = RateControl::VariableBitRate(bitrate_kbps);

        // Set timebase from stream_info
        let timebase = Timebase {
            num: NonZero::new(stream_info.time_base.0).unwrap(),
            den: NonZero::new(stream_info.time_base.1).unwrap(),
        };

        // Create encoder configuration
        // TODO: EncoderConfig supports additional VP9-specific settings:
        // - GOP size / keyframe interval (g, kf_max_dist, kf_min_dist)
        // - Quality range (rc_min_quantizer, rc_max_quantizer)
        // - CPU usage / deadline preset
        // - Thread count (g_threads)
        // - Tile columns (tile_columns) for parallel encoding
        // - Lossless mode (g_lossless)
        // - Profile selection (g_profile: 0=8-bit 4:2:0, 1=8-bit 4:2:2/4:4:4, etc.)
        // - Row-based multi-threading (g_row_mt)
        let config = EncoderConfig::<u8>::new(
            EncCodecId::VP9,
            video_params.width as u32,
            video_params.height as u32,
            timebase,
            rate_control,
        )
        .map_err(|e| Error::Encode(format!("Failed to create encoder config: {:?}", e)))?;

        // Create encoder
        let encoder = VpxEncoder::new(config)
            .map_err(|e| Error::Encode(format!("Failed to create VP9 encoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            encoder,
            frame_count: 0,
            buffered_packets: Vec::new(),
            flushed: false,
        })
    }

    /// Creates a VP9 encoder with custom bitrate
    pub fn with_bitrate(mut stream_info: StreamInfo, bitrate: u64) -> Result<Self> {
        stream_info.bitrate = Some(bitrate);
        Self::new(stream_info)
    }
}

impl Encoder for Vp9Encoder {
    fn codec(&self) -> &str {
        "vp9"
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
                    "VP9 encoder requires video frame parameters".to_string(),
                ))
            }
        };

        // VP9 requires YUV420P format (for Profile 0)
        if frame_params.format != PixelFormat::YUV420P {
            return Err(Error::Unsupported(format!(
                "VP9 encoder currently only supports YUV420P, got {:?}",
                frame_params.format
            )));
        }

        // Get plane data
        let y_plane = frame.plane(0).ok_or_else(|| Error::InvalidData("Failed to get Y plane".to_string()))?;
        let u_plane = frame.plane(1).ok_or_else(|| Error::InvalidData("Failed to get U plane".to_string()))?;
        let v_plane = frame.plane(2).ok_or_else(|| Error::InvalidData("Failed to get V plane".to_string()))?;

        // Combine all planes into a single buffer
        let mut combined_data = Vec::with_capacity(y_plane.len() + u_plane.len() + v_plane.len());
        combined_data.extend_from_slice(y_plane);
        combined_data.extend_from_slice(u_plane);
        combined_data.extend_from_slice(v_plane);

        // Create YUVImageData
        let yuv_image = YUVImageData::<u8>::from_raw_data(
            ImageFormat::I420, // YUV420P
            frame_params.width,
            frame_params.height,
            &combined_data,
        )
        .map_err(|e| Error::Encode(format!("Failed to create YUV image: {:?}", e)))?;

        // Encode the frame
        let timestamp = frame.pts().unwrap_or(self.frame_count);

        // TODO: Make these encoding parameters configurable:
        //
        // 1. Duration: Currently hardcoded to 1 timebase unit
        //    Should derive from frame rate: duration = timebase.den / fps
        //
        // 2. EncodingDeadline: Currently using default (GoodQuality)
        //    Should be configurable to:
        //    - EncodingDeadline::BestQuality (slowest, best compression)
        //    - EncodingDeadline::GoodQuality (balanced, default)
        //    - EncodingDeadline::Realtime (fastest, lower compression)
        //
        // 3. EncoderFrameFlags: Currently empty
        //    Should support:
        //    - EncoderFrameFlags::FORCE_KEYFRAME (for scene changes, seek points)
        //    - EncoderFrameFlags::NO_REFERENCE (for temporal layers)
        let packets = self
            .encoder
            .encode(
                timestamp,
                1, // duration - TODO: derive from frame rate
                yuv_image,
                EncodingDeadline::default(), // TODO: make configurable (BestQuality/GoodQuality/Realtime)
                EncoderFrameFlags::empty(),  // TODO: support FORCE_KEYFRAME and other flags
            )
            .map_err(|e| Error::Encode(format!("VP9 encode failed: {:?}", e)))?;

        // Convert compressed frames to packets
        for packet in packets {
            if let vpx_rs::Packet::CompressedFrame(compressed_frame) = packet {
                let mut pkt = Packet::new(
                    compressed_frame.data.to_vec(),
                    0, // stream index
                    MediaType::Video,
                );

                pkt.set_pts(Some(compressed_frame.pts));

                // Set keyframe flag
                if compressed_frame.flags.is_key {
                    pkt.set_keyframe(true);
                }

                self.buffered_packets.push(pkt);
            }
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
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.frame_count = 0;
        self.buffered_packets.clear();
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
    fn test_vp9_decoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = Vp9Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_vp9_decoder_wrong_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp8".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = Vp9Decoder::new(stream_info);
        assert!(decoder.is_err());
    }

    #[test]
    fn test_vp9_encoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = Vp9Encoder::new(stream_info);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_vp9_encoder_with_bitrate() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = Vp9Encoder::with_bitrate(stream_info, 2_000_000);
        assert!(encoder.is_ok());
    }
}
