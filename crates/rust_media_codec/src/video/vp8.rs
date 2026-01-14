//! VP8 video codec implementation using libvpx
//!
//! Provides VP8 encoding and decoding via vpx-rs bindings to libvpx.

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

/// VP8 video decoder
///
/// Decodes VP8-compressed video packets into raw YUV frames.
pub struct Vp8Decoder {
    stream_info: StreamInfo,
    decoder: VpxDecoder,
    buffered_frames: Vec<Frame>,
    flushed: bool,
}

impl Vp8Decoder {
    /// Creates a new VP8 decoder from stream information
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec
        if stream_info.codec != "vp8" {
            return Err(Error::Unsupported(format!(
                "Expected vp8 codec, got {}",
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
        let config = DecoderConfig::new(DecCodecId::VP8, width, height);

        // Create decoder
        let decoder = VpxDecoder::new(config)
            .map_err(|e| Error::Decode(format!("Failed to create VP8 decoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            decoder,
            buffered_frames: Vec::new(),
            flushed: false,
        })
    }
}

impl Decoder for Vp8Decoder {
    fn codec(&self) -> &str {
        "vp8"
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
            .map_err(|e| Error::Decode(format!("VP8 decode failed: {:?}", e)))?;

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
                        return Err(Error::Unsupported("Expected separate UV planes for VP8".to_string()));
                    }

                    frame
                }
                DecodedImageData::Data16b(_) => {
                    return Err(Error::Unsupported("VP8 16-bit output not supported".to_string()));
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

/// VP8 video encoder
///
/// Encodes raw YUV frames into VP8-compressed video packets.
pub struct Vp8Encoder {
    stream_info: StreamInfo,
    encoder: VpxEncoder<u8>,
    frame_count: i64,
    buffered_packets: Vec<Packet>,
    flushed: bool,
}

impl Vp8Encoder {
    /// Creates a new VP8 encoder from stream information
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        // Validate codec
        if stream_info.codec != "vp8" {
            return Err(Error::Unsupported(format!(
                "Expected vp8 codec, got {}",
                stream_info.codec
            )));
        }

        // Get video parameters
        let video_params = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => params,
            _ => {
                return Err(Error::InvalidData(
                    "VP8 encoder requires video stream parameters".to_string(),
                ))
            }
        };

        // Configure rate control (bitrate is in kbit/s for vpx-rs)
        let bitrate_kbps = (stream_info.bitrate.unwrap_or(1_000_000) / 1000) as u32;
        let rate_control = RateControl::VariableBitRate(bitrate_kbps);

        // Set timebase from stream_info
        let timebase = Timebase {
            num: NonZero::new(stream_info.time_base.0).unwrap(),
            den: NonZero::new(stream_info.time_base.1).unwrap(),
        };

        // Create encoder configuration
        let config = EncoderConfig::<u8>::new(
            EncCodecId::VP8,
            video_params.width as u32,
            video_params.height as u32,
            timebase,
            rate_control,
        )
        .map_err(|e| Error::Encode(format!("Failed to create encoder config: {:?}", e)))?;

        // Create encoder
        let encoder = VpxEncoder::new(config)
            .map_err(|e| Error::Encode(format!("Failed to create VP8 encoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            encoder,
            frame_count: 0,
            buffered_packets: Vec::new(),
            flushed: false,
        })
    }

    /// Creates a VP8 encoder with custom bitrate
    pub fn with_bitrate(mut stream_info: StreamInfo, bitrate: u64) -> Result<Self> {
        stream_info.bitrate = Some(bitrate);
        Self::new(stream_info)
    }
}

impl Encoder for Vp8Encoder {
    fn codec(&self) -> &str {
        "vp8"
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
                    "VP8 encoder requires video frame parameters".to_string(),
                ))
            }
        };

        // VP8 requires YUV420P format
        if frame_params.format != PixelFormat::YUV420P {
            return Err(Error::Unsupported(format!(
                "VP8 encoder only supports YUV420P, got {:?}",
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
        let packets = self
            .encoder
            .encode(
                timestamp,
                1, // duration
                yuv_image,
                EncodingDeadline::default(),
                EncoderFrameFlags::empty(),
            )
            .map_err(|e| Error::Encode(format!("VP8 encode failed: {:?}", e)))?;

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
    fn test_vp8_decoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp8".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = Vp8Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_vp8_decoder_wrong_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let decoder = Vp8Decoder::new(stream_info);
        assert!(decoder.is_err());
    }

    #[test]
    fn test_vp8_encoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp8".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = Vp8Encoder::new(stream_info);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_vp8_encoder_with_bitrate() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp8".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));

        let encoder = Vp8Encoder::with_bitrate(stream_info, 2_000_000);
        assert!(encoder.is_ok());
    }
}
