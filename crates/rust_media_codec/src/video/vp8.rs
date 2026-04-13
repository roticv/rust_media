//! VP8 video codec implementation using libvpx

use rust_media_core::{
    Decoder, Encoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
};
use rust_media_core::frame::FrameParams;
use vpx::{
    Codec, Deadline, DecoderConfig, EncoderConfig, FrameFlags, RateControl,
    Decoder as VpxDecoder, Encoder as VpxEncoder,
};

/// VP8 video decoder
pub struct Vp8Decoder {
    stream_info: StreamInfo,
    decoder: VpxDecoder,
    buffered_frames: Vec<Frame>,
    flushed: bool,
}

impl Vp8Decoder {
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "vp8" {
            return Err(Error::Unsupported(format!(
                "Expected vp8 codec, got {}", stream_info.codec
            )));
        }

        let (width, height) = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => {
                (params.width as u32, params.height as u32)
            }
            _ => (640, 480),
        };

        let config = DecoderConfig { codec: Codec::VP8, width, height };
        let decoder = VpxDecoder::new(&config)
            .map_err(|e| Error::Decode(format!("Failed to create VP8 decoder: {}", e)))?;

        Ok(Self { stream_info, decoder, buffered_frames: Vec::new(), flushed: false })
    }
}

impl Decoder for Vp8Decoder {
    fn codec(&self) -> &str { "vp8" }
    fn stream_info(&self) -> &StreamInfo { &self.stream_info }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if packet.media_type() != MediaType::Video {
            return Err(Error::InvalidData(format!(
                "Expected video packet, got {:?}", packet.media_type()
            )));
        }

        let pts = packet.pts();
        let decoded_frames = self.decoder.decode(packet.data())
            .map_err(|e| Error::Decode(format!("VP8 decode failed: {}", e)))?;

        for df in decoded_frames {
            let mut frame = Frame::new_video(df.width, df.height, PixelFormat::YUV420P);
            frame.set_pts(pts);

            let y_plane = frame.plane_mut(0)
                .ok_or_else(|| Error::InvalidData("Failed to get Y plane".into()))?;
            y_plane.copy_from_slice(&df.y);

            let u_plane = frame.plane_mut(1)
                .ok_or_else(|| Error::InvalidData("Failed to get U plane".into()))?;
            u_plane.copy_from_slice(&df.u);

            let v_plane = frame.plane_mut(2)
                .ok_or_else(|| Error::InvalidData("Failed to get V plane".into()))?;
            v_plane.copy_from_slice(&df.v);

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

    fn is_flushed(&self) -> bool { self.flushed }
}

/// VP8 rate control mode.
#[derive(Debug, Clone, Copy)]
pub enum Vp8RateControl {
    /// Variable bitrate (bits per second).
    VBR(u32),
    /// Constant bitrate (bits per second).
    CBR(u32),
    /// Constrained quality: bitrate cap (bps) with quality target (0..63,
    /// lower = better).
    CQ { bitrate: u32, cq_level: u32 },
    /// Pure quantizer mode (no bitrate target). Quality controlled via
    /// `min_quantizer`/`max_quantizer`.
    Q,
}

/// VP8 encoder configuration with builder pattern.
///
/// # Example
///
/// ```rust,ignore
/// let config = Vp8EncoderConfig::new()
///     .speed(4)
///     .rate_control(Vp8RateControl::VBR(2_000_000))
///     .gop_size(120);
/// let encoder = Vp8Encoder::with_config(stream_info, config)?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct Vp8EncoderConfig {
    /// Speed / CPU usage. VP8 range: 0 (best quality) to 16 (fastest).
    /// Default: libvpx default (0).
    pub speed: Option<i32>,
    /// Rate control mode. Default: VBR at 1 Mbps (or `stream_info.bitrate`).
    pub rate_control: Option<Vp8RateControl>,
    /// Maximum keyframe interval in frames. Default: libvpx default.
    pub kf_max_dist: Option<u32>,
    /// Minimum keyframe interval in frames. Default: libvpx default.
    pub kf_min_dist: Option<u32>,
    /// Minimum quantizer (0..63). Default: libvpx default.
    pub min_quantizer: Option<u32>,
    /// Maximum quantizer (0..63). Default: libvpx default.
    pub max_quantizer: Option<u32>,
    /// Encoding thread count. 0 = auto. Default: libvpx default.
    pub threads: Option<u32>,
}

impl Vp8EncoderConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn speed(mut self, v: i32) -> Self {
        self.speed = Some(v);
        self
    }

    pub fn rate_control(mut self, rc: Vp8RateControl) -> Self {
        self.rate_control = Some(rc);
        self
    }

    pub fn gop_size(mut self, max: u32) -> Self {
        self.kf_max_dist = Some(max);
        self
    }

    pub fn keyint_min(mut self, min: u32) -> Self {
        self.kf_min_dist = Some(min);
        self
    }

    pub fn min_quantizer(mut self, q: u32) -> Self {
        self.min_quantizer = Some(q);
        self
    }

    pub fn max_quantizer(mut self, q: u32) -> Self {
        self.max_quantizer = Some(q);
        self
    }

    pub fn threads(mut self, n: u32) -> Self {
        self.threads = Some(n);
        self
    }
}

/// VP8 video encoder
pub struct Vp8Encoder {
    stream_info: StreamInfo,
    encoder: VpxEncoder,
    frame_count: i64,
    buffered_packets: Vec<Packet>,
    flushed: bool,
}

impl Vp8Encoder {
    /// Create with default config. Honors `stream_info.bitrate` if set.
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        Self::with_config(stream_info, Vp8EncoderConfig::default())
    }

    pub fn with_bitrate(mut stream_info: StreamInfo, bitrate: u64) -> Result<Self> {
        stream_info.bitrate = Some(bitrate);
        Self::new(stream_info)
    }

    /// Create with explicit configuration. Config values override
    /// `stream_info.bitrate`.
    pub fn with_config(stream_info: StreamInfo, config: Vp8EncoderConfig) -> Result<Self> {
        if stream_info.codec != "vp8" {
            return Err(Error::Unsupported(format!(
                "Expected vp8 codec, got {}", stream_info.codec
            )));
        }

        let video_params = match &stream_info.params {
            rust_media_core::StreamParams::Video(params) => params,
            _ => return Err(Error::InvalidData(
                "VP8 encoder requires video stream parameters".into(),
            )),
        };

        let default_bitrate_kbps = (stream_info.bitrate.unwrap_or(1_000_000) / 1000) as u32;

        let rate_control = match config.rate_control {
            Some(Vp8RateControl::VBR(bps)) => RateControl::VBR(bps / 1000),
            Some(Vp8RateControl::CBR(bps)) => RateControl::CBR(bps / 1000),
            Some(Vp8RateControl::CQ { bitrate, cq_level }) => {
                RateControl::CQ { kbps: bitrate / 1000, cq_level }
            }
            Some(Vp8RateControl::Q) => RateControl::Q,
            None => RateControl::VBR(default_bitrate_kbps),
        };

        let vpx_config = EncoderConfig {
            codec: Codec::VP8,
            width: video_params.width as u32,
            height: video_params.height as u32,
            timebase_num: stream_info.time_base.0,
            timebase_den: stream_info.time_base.1,
            rate_control,
            kf_max_dist: config.kf_max_dist,
            kf_min_dist: config.kf_min_dist,
            threads: config.threads,
            cpu_used: config.speed,
            tile_columns: None, // VP8 doesn't support tiles
            tile_rows: None,
            rc_min_quantizer: config.min_quantizer,
            rc_max_quantizer: config.max_quantizer,
            bit_depth: 8, // VP8 is always 8-bit
        };

        let encoder = VpxEncoder::new(&vpx_config)
            .map_err(|e| Error::Encode(format!("Failed to create VP8 encoder: {}", e)))?;

        Ok(Self {
            stream_info, encoder, frame_count: 0,
            buffered_packets: Vec::new(), flushed: false,
        })
    }

    /// Returns the codec configuration record for container muxing.
    ///
    /// VP8 has no standard codec configuration record (no equivalent of
    /// avcC, vpcC, or av1C), so this returns an empty vector.
    pub fn codec_config(&self) -> Vec<u8> {
        Vec::new()
    }
}

impl Encoder for Vp8Encoder {
    fn codec(&self) -> &str { "vp8" }
    fn stream_info(&self) -> &StreamInfo { &self.stream_info }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if frame.media_type() != MediaType::Video {
            return Err(Error::InvalidData(format!(
                "Expected video frame, got {:?}", frame.media_type()
            )));
        }

        let frame_params = match frame.params() {
            FrameParams::Video(params) => params,
            _ => return Err(Error::InvalidData(
                "VP8 encoder requires video frame parameters".into(),
            )),
        };

        if frame_params.format != PixelFormat::YUV420P {
            return Err(Error::Unsupported(format!(
                "VP8 encoder only supports YUV420P, got {:?}", frame_params.format
            )));
        }

        let y_plane = frame.plane(0).ok_or_else(|| Error::InvalidData("Failed to get Y plane".into()))?;
        let u_plane = frame.plane(1).ok_or_else(|| Error::InvalidData("Failed to get U plane".into()))?;
        let v_plane = frame.plane(2).ok_or_else(|| Error::InvalidData("Failed to get V plane".into()))?;

        let mut combined = Vec::with_capacity(y_plane.len() + u_plane.len() + v_plane.len());
        combined.extend_from_slice(y_plane);
        combined.extend_from_slice(u_plane);
        combined.extend_from_slice(v_plane);

        let timestamp = frame.pts().unwrap_or(self.frame_count);

        let packets = self.encoder.encode(
            timestamp, 1, &combined, Deadline::default(), FrameFlags::default(),
        ).map_err(|e| Error::Encode(format!("VP8 encode failed: {}", e)))?;

        for ep in packets {
            let mut pkt = Packet::new(ep.data, 0, MediaType::Video);
            pkt.set_pts(Some(ep.pts));
            if ep.is_keyframe {
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
        let packets = self.encoder.flush()
            .map_err(|e| Error::Encode(format!("VP8 flush failed: {}", e)))?;

        for ep in packets {
            let mut pkt = Packet::new(ep.data, 0, MediaType::Video);
            pkt.set_pts(Some(ep.pts));
            if ep.is_keyframe {
                pkt.set_keyframe(true);
            }
            self.buffered_packets.push(pkt);
        }

        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.frame_count = 0;
        self.buffered_packets.clear();
        self.flushed = false;
        Ok(())
    }

    fn is_flushed(&self) -> bool { self.flushed }
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
        assert!(Vp8Decoder::new(stream_info).is_ok());
    }

    #[test]
    fn test_vp8_decoder_wrong_codec() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));
        assert!(Vp8Decoder::new(stream_info).is_err());
    }

    #[test]
    fn test_vp8_encoder_creation() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp8".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));
        assert!(Vp8Encoder::new(stream_info).is_ok());
    }

    #[test]
    fn test_vp8_encoder_with_bitrate() {
        let video_params = VideoStreamParams::new(640, 480, PixelFormat::YUV420P);
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp8".to_string())
            .with_time_base(1, 1_000_000)
            .with_params(rust_media_core::StreamParams::Video(video_params));
        assert!(Vp8Encoder::with_bitrate(stream_info, 2_000_000).is_ok());
    }
}
