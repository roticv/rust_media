//! AV1 video decoder implementation using dav1d
//!
//! Provides AV1 decoding via the dav1d crate (MIT bindings to libdav1d).
//! libdav1d is the reference AV1 decoder from VideoLAN, BSD-2-Clause licensed.
//!
//! # System Requirements
//!
//! Requires libdav1d to be installed on the system:
//! - macOS: `brew install dav1d`
//! - Debian/Ubuntu: `apt-get install libdav1d-dev`
//!
//! # Profile Support
//!
//! - **Main Profile** (8-bit) — fully supported
//! - **Main 10** (10-bit, HDR) — currently rejected; the decoder layer is
//!   8-bit only end-to-end
//!
//! # Bitstream Format
//!
//! AV1 packets are expected to contain raw OBU (Open Bitstream Unit) data,
//! which is the standard format used by both MP4 (`av01` sample entry) and
//! WebM/MKV (`V_AV1` codec ID) containers.

use dav1d::{Decoder as Dav1dDecoder, Error as Dav1dError, PixelLayout, PlanarImageComponent};
use rav1e::prelude::{
    ChromaSampling, Config as Rav1eConfig, Context as Rav1eContext, EncoderConfig as Rav1eEncoderConfig,
    EncoderStatus, FrameType as Rav1eFrameType, Rational,
};
use rust_media_core::frame::FrameParams;
use rust_media_core::{
    Decoder, Encoder, Error, Frame, MediaType, Packet, PixelFormat, Result, StreamInfo,
    StreamParams,
};
use std::collections::VecDeque;
use std::sync::Arc;

/// AV1 video decoder using dav1d
pub struct Av1Decoder {
    stream_info: StreamInfo,
    decoder: Dav1dDecoder,
    buffered_frames: VecDeque<Frame>,
    flushed: bool,
    /// Monotonically increasing counter used as the dav1d timestamp so we can
    /// correlate output pictures with their source packets.
    next_timestamp: i64,
    /// Maps `next_timestamp` values back to the original packet PTS.
    pts_map: VecDeque<(i64, Option<i64>)>,
}

impl Av1Decoder {
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        if stream_info.codec != "av1" && stream_info.codec != "av01" {
            return Err(Error::Unsupported(format!(
                "Expected av1 codec, got {}",
                stream_info.codec
            )));
        }

        let decoder = Dav1dDecoder::new()
            .map_err(|e| Error::Decode(format!("Failed to create AV1 decoder: {:?}", e)))?;

        Ok(Self {
            stream_info,
            decoder,
            buffered_frames: VecDeque::new(),
            flushed: false,
            next_timestamp: 0,
            pts_map: VecDeque::new(),
        })
    }

    /// Drain all available pictures from the dav1d decoder into the output queue.
    fn drain_pictures(&mut self) -> Result<()> {
        loop {
            match self.decoder.get_picture() {
                Ok(picture) => {
                    let frame = self.picture_to_frame(picture)?;
                    self.buffered_frames.push_back(frame);
                }
                Err(Dav1dError::Again) => break,
                Err(e) => {
                    return Err(Error::Decode(format!("dav1d get_picture failed: {:?}", e)));
                }
            }
        }
        Ok(())
    }

    /// Convert a dav1d Picture into a rust_media_core Frame, accounting for
    /// per-row stride.
    fn picture_to_frame(&mut self, picture: dav1d::Picture) -> Result<Frame> {
        // dav1d's `bit_depth()` returns the actual bit depth (8, 10, or 12),
        // not the storage size. 8-bit content uses 1 byte per sample; 10/12-bit
        // content uses 2 bytes per sample (little-endian u16, lower bits significant).
        let bit_depth = picture.bit_depth();
        let pixel_format = match bit_depth {
            8 => PixelFormat::YUV420P,
            10 => PixelFormat::YUV420P10LE,
            12 => {
                return Err(Error::Unsupported(
                    "12-bit AV1 (Profile 2) is not yet supported. \
                    The codec layer and pixel format support are 8-bit/10-bit only."
                        .to_string(),
                ));
            }
            other => {
                return Err(Error::Unsupported(format!(
                    "AV1 decoder: unexpected bit depth {}",
                    other
                )));
            }
        };

        // We only support YUV 4:2:0 (I420) for now.
        if picture.pixel_layout() != PixelLayout::I420 {
            return Err(Error::Unsupported(format!(
                "AV1 decoder only supports YUV 4:2:0 (I420), got {:?}",
                picture.pixel_layout()
            )));
        }

        let width = picture.width() as usize;
        let height = picture.height() as usize;

        let mut frame = Frame::new_video(width, height, pixel_format);

        // Map dav1d's monotonic timestamp back to the source packet PTS.
        let ts = picture.timestamp();
        let pts = match ts {
            Some(t) => {
                // Find and consume the corresponding entry from pts_map.
                let mut found = None;
                while let Some(&(map_ts, map_pts)) = self.pts_map.front() {
                    if map_ts == t {
                        found = Some(map_pts);
                        self.pts_map.pop_front();
                        break;
                    } else if map_ts < t {
                        // Stale entry (lost packet), drop it
                        self.pts_map.pop_front();
                    } else {
                        break;
                    }
                }
                found.unwrap_or(None)
            }
            None => None,
        };
        frame.set_pts(pts);

        // Bytes per pixel: 1 for 8-bit, 2 for 10/12-bit storage
        let bps = if bit_depth == 8 { 1 } else { 2 };

        // Copy each plane, honoring stride (which may be > width*bps for SIMD alignment).
        // dav1d stride is in bytes; we copy `plane_width * bps` bytes per row.
        let copy_plane = |frame: &mut Frame,
                          plane_idx: usize,
                          src: &[u8],
                          src_stride: usize,
                          plane_width: usize,
                          plane_height: usize|
         -> Result<()> {
            let row_bytes = plane_width * bps;
            let dst = frame
                .plane_mut(plane_idx)
                .ok_or_else(|| Error::InvalidData(format!("missing plane {}", plane_idx)))?;
            for row in 0..plane_height {
                let src_off = row * src_stride;
                let dst_off = row * row_bytes;
                dst[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&src[src_off..src_off + row_bytes]);
            }
            Ok(())
        };

        // Y plane (full resolution)
        let y_plane = picture.plane(PlanarImageComponent::Y);
        let y_stride = picture.stride(PlanarImageComponent::Y) as usize;
        copy_plane(&mut frame, 0, y_plane.as_ref(), y_stride, width, height)?;

        // U plane (half resolution for I420)
        let u_plane = picture.plane(PlanarImageComponent::U);
        let u_stride = picture.stride(PlanarImageComponent::U) as usize;
        copy_plane(
            &mut frame,
            1,
            u_plane.as_ref(),
            u_stride,
            width / 2,
            height / 2,
        )?;

        // V plane (half resolution for I420)
        let v_plane = picture.plane(PlanarImageComponent::V);
        let v_stride = picture.stride(PlanarImageComponent::V) as usize;
        copy_plane(
            &mut frame,
            2,
            v_plane.as_ref(),
            v_stride,
            width / 2,
            height / 2,
        )?;

        Ok(frame)
    }
}

impl Decoder for Av1Decoder {
    fn codec(&self) -> &str {
        "av1"
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

        // dav1d takes ownership of the buffer (T: 'static), so clone it.
        let data: Vec<u8> = packet.data().to_vec();

        // Use a monotonic counter as dav1d's internal timestamp so we can
        // correlate output pictures with their source PTS values.
        let internal_ts = self.next_timestamp;
        self.next_timestamp += 1;
        self.pts_map.push_back((internal_ts, packet.pts()));

        // Standard dav1d push/pull pattern: if send_data returns Again, we
        // need to drain pictures first, then retry send_pending_data.
        match self.decoder.send_data(data, None, Some(internal_ts), None) {
            Ok(()) => {}
            Err(Dav1dError::Again) => {
                // Drain pending pictures, then send the buffered data
                self.drain_pictures()?;
                self.decoder
                    .send_pending_data()
                    .map_err(|e| Error::Decode(format!("dav1d send_pending_data: {:?}", e)))?;
            }
            Err(e) => {
                return Err(Error::Decode(format!("dav1d send_data: {:?}", e)));
            }
        }

        // Drain any pictures that became available after sending
        self.drain_pictures()?;

        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if let Some(frame) = self.buffered_frames.pop_front() {
            return Ok(frame);
        }

        // Try to drain more pictures from the decoder
        self.drain_pictures()?;
        if let Some(frame) = self.buffered_frames.pop_front() {
            return Ok(frame);
        }

        if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        // Drain any remaining pictures
        self.drain_pictures()?;
        // dav1d's flush() resets the decoder state — instead of calling that,
        // we just mark ourselves as flushed so receive_frame can return EOS
        // once buffered_frames is empty.
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.decoder.flush();
        self.buffered_frames.clear();
        self.pts_map.clear();
        self.next_timestamp = 0;
        self.flushed = false;
        Ok(())
    }

    fn is_flushed(&self) -> bool {
        self.flushed
    }
}

// ============================================================================
// AV1 Encoder (rav1e)
// ============================================================================

/// rav1e encoder context — held as one of two type-specialized variants since
/// `rav1e::Context<T>` is generic over pixel storage (`u8` for 8-bit,
/// `u16` for 10-bit). The variant is chosen at construction time based on the
/// stream's pixel format and never changes afterwards.
enum Rav1eVariant {
    Eight(Rav1eContext<u8>),
    Ten(Rav1eContext<u16>),
}

/// Rate control mode for the AV1 encoder. Quantizer and bitrate are mutually
/// exclusive — modeling them as an enum makes that impossible to misuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Av1RateControl {
    /// Constant quantizer mode. Range `0..=255` where 0 is lossless and 255
    /// is worst quality. rav1e's default is 100. There is no constant-quality
    /// (CQ) mode in rav1e — quantizer is the closest equivalent.
    Quantizer(u8),
    /// Average bitrate target in bits per second. Internally clamped to
    /// `i32::MAX` because that's what rav1e's `EncoderConfig.bitrate` accepts.
    Bitrate(u32),
}

/// Configurable parameters for [`Av1Encoder`].
///
/// Build one of these with [`Av1EncoderConfig::new`] (or `default()`) and
/// chain `.speed_preset(..)`, `.bitrate(..)`, etc. Then pass it to
/// [`Av1Encoder::with_config`].
///
/// # Defaults
///
/// All fields default to rav1e's own defaults so that
/// `Av1Encoder::new(stream_info)` and
/// `Av1Encoder::with_config(stream_info, Av1EncoderConfig::default())` are
/// equivalent (modulo `stream_info.bitrate`, which `new` honors for
/// backwards compatibility — see [`Av1Encoder::new`]).
///
/// | Field | Default | Notes |
/// |-------|---------|-------|
/// | `speed_preset` | 6 | rav1e's balanced default. 0 = slowest/best, 10 = fastest/worst |
/// | `rate_control` | `Quantizer(100)` | rav1e default qp |
/// | `min_key_frame_interval` | 12 | minimum frames between keyframes |
/// | `max_key_frame_interval` | 240 | 0 means infinite (single keyframe at start) |
/// | `tile_cols` / `tile_rows` | 0 / 0 | 0 = let rav1e choose; values must be powers of two |
/// | `tiles` | 0 | total tile count, overrides `tile_cols`/`tile_rows` if non-zero |
/// | `low_latency` | false | true disables frame reordering for streaming |
/// | `error_resilient` | false | true makes every frame independently decodable |
///
/// # Example
///
/// ```rust,ignore
/// use rust_media_codec::video::av1::{Av1Encoder, Av1EncoderConfig, Av1RateControl};
///
/// let config = Av1EncoderConfig::new()
///     .speed_preset(8)                       // faster encode
///     .rate_control(Av1RateControl::Bitrate(2_000_000))
///     .key_frame_interval(60, 240)           // ~2s GOP @ 30fps
///     .tiles(4)                              // 4 tiles for parallelism
///     .low_latency(true);
/// let encoder = Av1Encoder::with_config(stream_info, config)?;
/// ```
#[derive(Debug, Clone)]
pub struct Av1EncoderConfig {
    speed_preset: u8,
    rate_control: Av1RateControl,
    min_key_frame_interval: u64,
    max_key_frame_interval: u64,
    tile_cols: usize,
    tile_rows: usize,
    tiles: usize,
    low_latency: bool,
    error_resilient: bool,
}

impl Default for Av1EncoderConfig {
    fn default() -> Self {
        Self {
            speed_preset: 6,
            rate_control: Av1RateControl::Quantizer(100),
            min_key_frame_interval: 12,
            max_key_frame_interval: 240,
            tile_cols: 0,
            tile_rows: 0,
            tiles: 0,
            low_latency: false,
            error_resilient: false,
        }
    }
}

impl Av1EncoderConfig {
    /// Create a new config with rav1e defaults. Same as
    /// [`Av1EncoderConfig::default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the speed preset in the range `0..=10`. Lower values are slower
    /// and produce higher quality. Values above 10 are clamped to 10.
    pub fn speed_preset(mut self, preset: u8) -> Self {
        self.speed_preset = preset.min(10);
        self
    }

    /// Set the rate control mode (quantizer or bitrate). Use this directly
    /// when you need to express the choice explicitly; otherwise the
    /// shorthand `quantizer(..)` and `bitrate(..)` methods are usually clearer.
    pub fn rate_control(mut self, rc: Av1RateControl) -> Self {
        self.rate_control = rc;
        self
    }

    /// Shorthand for `rate_control(Av1RateControl::Quantizer(q))`.
    pub fn quantizer(self, q: u8) -> Self {
        self.rate_control(Av1RateControl::Quantizer(q))
    }

    /// Shorthand for `rate_control(Av1RateControl::Bitrate(bps))`.
    pub fn bitrate(self, bps: u32) -> Self {
        self.rate_control(Av1RateControl::Bitrate(bps))
    }

    /// Set the minimum and maximum keyframe interval, in frames. A `max` of
    /// 0 maps to "infinite" (only the first frame is a keyframe), matching
    /// rav1e's `set_key_frame_interval` semantics.
    pub fn key_frame_interval(mut self, min: u64, max: u64) -> Self {
        self.min_key_frame_interval = min;
        self.max_key_frame_interval = max;
        self
    }

    /// Set the number of horizontal tiles. Must be a power of two; 0 lets
    /// rav1e choose. Overridden by [`Self::tiles`] if that's also set.
    pub fn tile_cols(mut self, n: usize) -> Self {
        self.tile_cols = n;
        self
    }

    /// Set the number of vertical tiles. Must be a power of two; 0 lets
    /// rav1e choose. Overridden by [`Self::tiles`] if that's also set.
    pub fn tile_rows(mut self, n: usize) -> Self {
        self.tile_rows = n;
        self
    }

    /// Set the total tile count. rav1e will split horizontally and
    /// vertically as needed to reach this number. When non-zero this
    /// overrides `tile_cols`/`tile_rows`.
    pub fn tiles(mut self, total: usize) -> Self {
        self.tiles = total;
        self
    }

    /// Enable or disable low-latency mode. When enabled, rav1e disables
    /// frame reordering (no B-frames in display order) to minimize end-to-end
    /// latency. Useful for streaming and live encoding.
    pub fn low_latency(mut self, v: bool) -> Self {
        self.low_latency = v;
        self
    }

    /// Enable or disable error-resilient mode. When enabled, every frame
    /// is independently decodable, which makes recovery from packet loss
    /// possible at the cost of compression efficiency.
    pub fn error_resilient(mut self, v: bool) -> Self {
        self.error_resilient = v;
        self
    }
}

/// AV1 video encoder using rav1e (pure-Rust, BSD-2-Clause).
///
/// Supports `YUV420P` (8-bit) and `YUV420P10LE` (10-bit) input. The bit depth
/// is locked at construction time from `StreamInfo`'s pixel format and cannot
/// change mid-stream.
///
/// # Configuration
///
/// `Av1Encoder::new(stream_info)` uses [`Av1EncoderConfig::default`] (rav1e
/// defaults), and additionally honors `stream_info.bitrate` for backwards
/// compatibility — if it's set, the encoder switches into bitrate mode at
/// that target.
///
/// For more control, use [`Av1Encoder::with_config`] and build an
/// [`Av1EncoderConfig`] yourself:
///
/// ```rust,ignore
/// let config = Av1EncoderConfig::new()
///     .speed_preset(8)
///     .bitrate(2_000_000)
///     .key_frame_interval(60, 240);
/// let encoder = Av1Encoder::with_config(stream_info, config)?;
/// ```
///
/// # Codec Config Record (av1C)
///
/// [`Av1Encoder::codec_config`] exposes the sequence header in the format
/// expected by both ISOBMFF (MP4 av1C box) and Matroska (WebM/MKV
/// CodecPrivate). Unlike most encoder config records, it can be called
/// immediately after construction — the bytes are derived from the encoder
/// config, not from any sent frames.
pub struct Av1Encoder {
    stream_info: StreamInfo,
    ctx: Rav1eVariant,
    /// Bit depth in use (8 or 10). Locked at construction.
    bit_depth: usize,
    /// Sequential frame number we've sent so far. Also used as fallback PTS.
    next_input_frameno: u64,
    /// PTS values keyed by `input_frameno`, kept in send order. rav1e returns
    /// packets in display order with sequential `input_frameno`s, so we just
    /// pop the front of this queue when a packet arrives.
    pts_queue: VecDeque<Option<i64>>,
    /// Packets that have been pulled from rav1e but not yet returned via
    /// `receive_packet`.
    buffered_packets: VecDeque<Packet>,
    flushed: bool,
}

impl Av1Encoder {
    /// Create an encoder with rav1e defaults. For backwards compatibility,
    /// if `stream_info.bitrate` is set this is treated as
    /// `rate_control = Bitrate(stream_info.bitrate)`; otherwise the encoder
    /// runs in quantizer mode at rav1e's default qp = 100.
    ///
    /// For finer control over speed preset, key-frame interval, tiles, etc.,
    /// use [`Av1Encoder::with_config`] and build an [`Av1EncoderConfig`].
    pub fn new(stream_info: StreamInfo) -> Result<Self> {
        let mut config = Av1EncoderConfig::default();
        if let Some(bitrate_bps) = stream_info.bitrate {
            // Promote stream_info.bitrate to a Bitrate rate_control. We cap
            // to u32::MAX here; the inner i32::MAX clamp happens in
            // `with_config` when applying to rav1e.
            let bps = bitrate_bps.min(u32::MAX as u64) as u32;
            config = config.bitrate(bps);
        }
        Self::with_config(stream_info, config)
    }

    /// Backwards-compatible shortcut for setting `stream_info.bitrate` and
    /// then calling `new`. New code should prefer
    /// `Av1Encoder::with_config(stream_info, Av1EncoderConfig::new().bitrate(..))`.
    pub fn with_bitrate(mut stream_info: StreamInfo, bitrate: u64) -> Result<Self> {
        stream_info.bitrate = Some(bitrate);
        Self::new(stream_info)
    }

    /// Create an encoder with an explicit configuration. This is the
    /// preferred constructor when you need to tune anything beyond bitrate.
    ///
    /// Note: `config` overrides `stream_info.bitrate`. If you set
    /// `Av1RateControl::Quantizer` in the config but `stream_info.bitrate` is
    /// also set, the config wins and quantizer mode is used.
    pub fn with_config(stream_info: StreamInfo, config: Av1EncoderConfig) -> Result<Self> {
        if stream_info.codec != "av1" && stream_info.codec != "av01" {
            return Err(Error::Unsupported(format!(
                "Expected av1 codec, got {}",
                stream_info.codec
            )));
        }

        let video_params = match &stream_info.params {
            StreamParams::Video(params) => params,
            _ => {
                return Err(Error::InvalidData(
                    "AV1 encoder requires video stream parameters".into(),
                ))
            }
        };

        // Lock bit depth based on the input pixel format. Mixed bit-depth
        // streams are not supported (rav1e contexts are generic over T).
        let bit_depth = match video_params.pixel_format {
            PixelFormat::YUV420P => 8usize,
            PixelFormat::YUV420P10LE => 10usize,
            other => {
                return Err(Error::Unsupported(format!(
                    "AV1 encoder only supports YUV420P (8-bit) or YUV420P10LE (10-bit), got {:?}",
                    other
                )))
            }
        };

        let width = video_params.width;
        let height = video_params.height;
        if width == 0 || height == 0 {
            return Err(Error::InvalidData(
                "AV1 encoder: width/height must be non-zero".into(),
            ));
        }

        // Start from a speed-preset baseline, then layer the rest of the
        // config on top. with_speed_preset() initializes SpeedSettings, so
        // calling it first means we don't have to touch SpeedSettings
        // ourselves later.
        let mut enc = Rav1eEncoderConfig::with_speed_preset(config.speed_preset);
        enc.width = width;
        enc.height = height;
        enc.bit_depth = bit_depth;
        enc.chroma_sampling = ChromaSampling::Cs420;
        enc.time_base = Rational {
            num: stream_info.time_base.0 as u64,
            den: stream_info.time_base.1 as u64,
        };

        // Apply rate control. In rav1e, `bitrate == 0` means "use quantizer
        // mode at the value in `quantizer`"; non-zero means bitrate mode and
        // `quantizer` is ignored.
        match config.rate_control {
            Av1RateControl::Quantizer(q) => {
                enc.bitrate = 0;
                enc.quantizer = q as usize;
            }
            Av1RateControl::Bitrate(bps) => {
                enc.bitrate = bps.min(i32::MAX as u32) as i32;
            }
        }

        // Keyframe interval. set_key_frame_interval handles the special case
        // of max == 0 → infinite, so we forward our value as-is.
        enc.set_key_frame_interval(
            config.min_key_frame_interval,
            config.max_key_frame_interval,
        );

        enc.tile_cols = config.tile_cols;
        enc.tile_rows = config.tile_rows;
        enc.tiles = config.tiles;
        enc.low_latency = config.low_latency;
        enc.error_resilient = config.error_resilient;

        let cfg = Rav1eConfig::new().with_encoder_config(enc);

        let ctx = if bit_depth == 8 {
            Rav1eVariant::Eight(
                cfg.new_context::<u8>()
                    .map_err(|e| Error::Encode(format!("rav1e new_context (8-bit): {:?}", e)))?,
            )
        } else {
            Rav1eVariant::Ten(
                cfg.new_context::<u16>()
                    .map_err(|e| Error::Encode(format!("rav1e new_context (10-bit): {:?}", e)))?,
            )
        };

        Ok(Self {
            stream_info,
            ctx,
            bit_depth,
            next_input_frameno: 0,
            pts_queue: VecDeque::new(),
            buffered_packets: VecDeque::new(),
            flushed: false,
        })
    }

    /// Returns the AV1 sequence header in the format expected by ISOBMFF
    /// (`av1C` box) and Matroska (`CodecPrivate`). Useful for muxers that
    /// need to write the codec config record.
    pub fn codec_config(&self) -> Vec<u8> {
        match &self.ctx {
            Rav1eVariant::Eight(c) => c.container_sequence_header(),
            Rav1eVariant::Ten(c) => c.container_sequence_header(),
        }
    }

    /// Drain all packets currently available from rav1e into `buffered_packets`.
    fn drain_packets(&mut self) -> Result<()> {
        loop {
            let result = match &mut self.ctx {
                Rav1eVariant::Eight(c) => match c.receive_packet() {
                    Ok(pkt) => Ok((pkt.input_frameno, pkt.frame_type, pkt.data)),
                    Err(e) => Err(e),
                },
                Rav1eVariant::Ten(c) => match c.receive_packet() {
                    Ok(pkt) => Ok((pkt.input_frameno, pkt.frame_type, pkt.data)),
                    Err(e) => Err(e),
                },
            };
            match result {
                Ok((input_frameno, frame_type, data)) => {
                    let pts = self.pts_queue.pop_front().flatten();
                    let mut pkt = Packet::new(data, 0, MediaType::Video);
                    // Fall back to the input frame number if no PTS was provided.
                    pkt.set_pts(Some(pts.unwrap_or(input_frameno as i64)));
                    if matches!(frame_type, Rav1eFrameType::KEY) {
                        pkt.set_keyframe(true);
                    }
                    self.buffered_packets.push_back(pkt);
                }
                Err(EncoderStatus::NeedMoreData) => break,
                Err(EncoderStatus::Encoded) => continue,
                Err(EncoderStatus::LimitReached) => {
                    // Flush completed
                    break;
                }
                Err(e) => {
                    return Err(Error::Encode(format!("rav1e receive_packet: {:?}", e)));
                }
            }
        }
        Ok(())
    }
}

impl Encoder for Av1Encoder {
    fn codec(&self) -> &str {
        "av1"
    }

    fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        if frame.media_type() != MediaType::Video {
            return Err(Error::InvalidData(format!(
                "Expected video frame, got {:?}",
                frame.media_type()
            )));
        }

        let frame_params = match frame.params() {
            FrameParams::Video(p) => p,
            _ => {
                return Err(Error::InvalidData(
                    "AV1 encoder requires video frame parameters".into(),
                ))
            }
        };

        // Verify the frame's bit depth matches what we were configured for —
        // mixing 8-bit and 10-bit frames in one stream isn't supported because
        // the rav1e Context is generic over the pixel storage type.
        let frame_is_10bit = frame_params.format == PixelFormat::YUV420P10LE;
        let frame_is_8bit = frame_params.format == PixelFormat::YUV420P;
        if !frame_is_8bit && !frame_is_10bit {
            return Err(Error::Unsupported(format!(
                "AV1 encoder: unsupported pixel format {:?}",
                frame_params.format
            )));
        }
        match (self.bit_depth, frame_is_10bit) {
            (8, false) | (10, true) => {}
            _ => {
                return Err(Error::InvalidData(format!(
                    "AV1 encoder bit depth mismatch: encoder is {}-bit, frame is {}",
                    self.bit_depth,
                    if frame_is_10bit { "10-bit" } else { "8-bit" }
                )));
            }
        }

        let width = frame_params.width;
        let height = frame_params.height;
        let bytes_per_sample = if self.bit_depth == 8 { 1 } else { 2 };

        let y_plane = frame.plane(0).ok_or_else(|| {
            Error::InvalidData("AV1 encoder: missing Y plane".into())
        })?;
        let u_plane = frame.plane(1).ok_or_else(|| {
            Error::InvalidData("AV1 encoder: missing U plane".into())
        })?;
        let v_plane = frame.plane(2).ok_or_else(|| {
            Error::InvalidData("AV1 encoder: missing V plane".into())
        })?;

        let y_stride = width * bytes_per_sample;
        let uv_stride = (width / 2) * bytes_per_sample;

        // Build the rav1e frame and copy planes in.
        let send_result = match &mut self.ctx {
            Rav1eVariant::Eight(c) => {
                let mut rf = c.new_frame();
                rf.planes[0].copy_from_raw_u8(y_plane, y_stride, bytes_per_sample);
                rf.planes[1].copy_from_raw_u8(u_plane, uv_stride, bytes_per_sample);
                rf.planes[2].copy_from_raw_u8(v_plane, uv_stride, bytes_per_sample);
                c.send_frame(Arc::new(rf))
            }
            Rav1eVariant::Ten(c) => {
                let mut rf = c.new_frame();
                rf.planes[0].copy_from_raw_u8(y_plane, y_stride, bytes_per_sample);
                rf.planes[1].copy_from_raw_u8(u_plane, uv_stride, bytes_per_sample);
                rf.planes[2].copy_from_raw_u8(v_plane, uv_stride, bytes_per_sample);
                c.send_frame(Arc::new(rf))
            }
        };
        send_result.map_err(|e| Error::Encode(format!("rav1e send_frame: {:?}", e)))?;

        // Track the PTS for the frame we just sent so we can attach it to the
        // matching output packet later.
        self.pts_queue
            .push_back(Some(frame.pts().unwrap_or(self.next_input_frameno as i64)));
        self.next_input_frameno += 1;

        // Try to drain any packets that became ready.
        self.drain_packets()?;

        let _ = (width, height); // currently unused after copy; kept for clarity
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        if let Some(p) = self.buffered_packets.pop_front() {
            return Ok(p);
        }
        // Try one more drain in case the encoder has packets ready since
        // the last send_frame call.
        self.drain_packets()?;
        if let Some(p) = self.buffered_packets.pop_front() {
            return Ok(p);
        }
        if self.flushed {
            Err(Error::EndOfStream)
        } else {
            Err(Error::NeedMoreData)
        }
    }

    fn flush(&mut self) -> Result<()> {
        // rav1e flush() is `send_frame(None)` which signals end-of-stream.
        match &mut self.ctx {
            Rav1eVariant::Eight(c) => c.flush(),
            Rav1eVariant::Ten(c) => c.flush(),
        }
        // Drain remaining packets after flush.
        self.drain_packets()?;
        self.flushed = true;
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        // rav1e doesn't support reset on an existing context — the user has to
        // build a new encoder. We can at least clear our buffered state.
        self.pts_queue.clear();
        self.buffered_packets.clear();
        self.next_input_frameno = 0;
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
    use rust_media_core::{StreamInfo, StreamParams, VideoStreamParams};

    fn make_stream_info() -> StreamInfo {
        StreamInfo::new(0, MediaType::Video, "av1".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                640,
                480,
                PixelFormat::YUV420P,
            )))
            .with_time_base(1, 30)
    }

    #[test]
    fn test_av1_decoder_creation() {
        let stream_info = make_stream_info();
        let decoder = Av1Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_av1_decoder_accepts_av01_codec_name() {
        let stream_info = StreamInfo::new(0, MediaType::Video, "av01".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                640,
                480,
                PixelFormat::YUV420P,
            )));
        let decoder = Av1Decoder::new(stream_info);
        assert!(decoder.is_ok());
    }

    #[test]
    fn test_av1_decoder_rejects_wrong_codec() {
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                640,
                480,
                PixelFormat::YUV420P,
            )));
        let decoder = Av1Decoder::new(stream_info);
        assert!(decoder.is_err());
    }

    #[test]
    fn test_av1_decoder_codec_name() {
        let decoder = Av1Decoder::new(make_stream_info()).unwrap();
        assert_eq!(decoder.codec(), "av1");
    }

    // ----- Av1EncoderConfig builder tests -----

    #[test]
    fn av1_encoder_config_default_matches_rav1e_defaults() {
        let cfg = Av1EncoderConfig::default();
        assert_eq!(cfg.speed_preset, 6);
        assert_eq!(cfg.rate_control, Av1RateControl::Quantizer(100));
        assert_eq!(cfg.min_key_frame_interval, 12);
        assert_eq!(cfg.max_key_frame_interval, 240);
        assert_eq!(cfg.tile_cols, 0);
        assert_eq!(cfg.tile_rows, 0);
        assert_eq!(cfg.tiles, 0);
        assert!(!cfg.low_latency);
        assert!(!cfg.error_resilient);
    }

    #[test]
    fn av1_encoder_config_chained_setters() {
        let cfg = Av1EncoderConfig::new()
            .speed_preset(8)
            .quantizer(80)
            .key_frame_interval(60, 240)
            .tile_cols(2)
            .tile_rows(2)
            .low_latency(true)
            .error_resilient(true);
        assert_eq!(cfg.speed_preset, 8);
        assert_eq!(cfg.rate_control, Av1RateControl::Quantizer(80));
        assert_eq!(cfg.min_key_frame_interval, 60);
        assert_eq!(cfg.max_key_frame_interval, 240);
        assert_eq!(cfg.tile_cols, 2);
        assert_eq!(cfg.tile_rows, 2);
        assert!(cfg.low_latency);
        assert!(cfg.error_resilient);
    }

    #[test]
    fn av1_encoder_config_speed_preset_clamps_to_10() {
        let cfg = Av1EncoderConfig::new().speed_preset(255);
        assert_eq!(cfg.speed_preset, 10);
    }

    #[test]
    fn av1_encoder_config_bitrate_overrides_quantizer() {
        // Last writer wins on rate_control — bitrate(..) should replace
        // an earlier quantizer(..) and vice versa.
        let cfg = Av1EncoderConfig::new().quantizer(50).bitrate(2_000_000);
        assert_eq!(cfg.rate_control, Av1RateControl::Bitrate(2_000_000));

        let cfg = Av1EncoderConfig::new().bitrate(2_000_000).quantizer(50);
        assert_eq!(cfg.rate_control, Av1RateControl::Quantizer(50));
    }

    #[test]
    fn av1_encoder_with_config_quantizer_mode() {
        // Constructing with an explicit quantizer config should succeed and
        // produce a working encoder regardless of stream_info.bitrate.
        let mut stream_info = make_stream_info();
        stream_info.bitrate = Some(1_500_000); // should be ignored — config wins
        let config = Av1EncoderConfig::new().speed_preset(10).quantizer(120);
        let encoder = Av1Encoder::with_config(stream_info, config)
            .expect("encoder construction should succeed");
        // Codec config should still be available immediately.
        assert!(!encoder.codec_config().is_empty());
    }

    #[test]
    fn av1_encoder_with_config_bitrate_mode() {
        let stream_info = make_stream_info();
        let config = Av1EncoderConfig::new()
            .bitrate(2_500_000)
            .key_frame_interval(60, 240)
            .tiles(4);
        let encoder = Av1Encoder::with_config(stream_info, config)
            .expect("encoder construction should succeed");
        assert_eq!(encoder.codec(), "av1");
        assert!(!encoder.codec_config().is_empty());
    }

    #[test]
    fn av1_encoder_new_promotes_stream_info_bitrate_to_rate_control() {
        // The legacy `new` path should still honor stream_info.bitrate by
        // converting it into a Bitrate rate_control internally.
        let mut stream_info = make_stream_info();
        stream_info.bitrate = Some(750_000);
        let encoder = Av1Encoder::new(stream_info).expect("should succeed");
        assert_eq!(encoder.codec(), "av1");
    }

    #[test]
    fn av1_encoder_with_config_rejects_wrong_codec() {
        let stream_info = StreamInfo::new(0, MediaType::Video, "vp9".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                320,
                240,
                PixelFormat::YUV420P,
            )))
            .with_time_base(1, 30);
        let result = Av1Encoder::with_config(stream_info, Av1EncoderConfig::new());
        assert!(result.is_err());
    }

    #[test]
    fn av1_encoder_with_config_10bit_pixel_format() {
        let stream_info = StreamInfo::new(0, MediaType::Video, "av1".to_string())
            .with_params(StreamParams::Video(VideoStreamParams::new(
                320,
                240,
                PixelFormat::YUV420P10LE,
            )))
            .with_time_base(1, 30);
        let config = Av1EncoderConfig::new().speed_preset(8);
        let encoder = Av1Encoder::with_config(stream_info, config)
            .expect("10-bit AV1 encoder should construct");
        assert_eq!(encoder.codec(), "av1");
    }
}
