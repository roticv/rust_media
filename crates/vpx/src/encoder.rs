use crate::error::{self, Result};
use crate::image::Codec;
use std::mem::MaybeUninit;
use std::ptr;

/// Encoding deadline / quality tradeoff.
#[derive(Debug, Clone, Copy, Default)]
pub enum Deadline {
    /// Best possible quality (slowest).
    BestQuality,
    /// Good quality (balanced, default).
    #[default]
    GoodQuality,
    /// Realtime (fastest).
    Realtime,
}

impl Deadline {
    fn as_raw(self) -> std::os::raw::c_ulong {
        match self {
            Deadline::BestQuality => vpx_sys::VPX_DL_BEST_QUALITY as _,
            Deadline::GoodQuality => vpx_sys::VPX_DL_GOOD_QUALITY as _,
            Deadline::Realtime => vpx_sys::VPX_DL_REALTIME as _,
        }
    }
}

/// Per-frame encoder flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameFlags {
    /// Force this frame to be a keyframe.
    pub force_keyframe: bool,
}

impl FrameFlags {
    fn as_raw(self) -> std::os::raw::c_long {
        let mut flags = 0;
        if self.force_keyframe {
            flags |= vpx_sys::VPX_EFLAG_FORCE_KF as std::os::raw::c_long;
        }
        flags
    }
}

/// Rate control mode.
#[derive(Debug, Clone, Copy)]
pub enum RateControl {
    /// Variable bitrate (kbps).
    VBR(u32),
    /// Constant bitrate (kbps).
    CBR(u32),
    /// Constrained quality: bitrate cap (kbps) with quality target (cq_level).
    CQ { kbps: u32, cq_level: u32 },
    /// Pure quantizer mode (no bitrate target). Quality controlled via
    /// `rc_min_quantizer`/`rc_max_quantizer` in `EncoderConfig`.
    Q,
}

/// Encoder configuration.
pub struct EncoderConfig {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub timebase_num: u32,
    pub timebase_den: u32,
    pub rate_control: RateControl,
    /// Bit depth (8 or 10). Default: 8. When 10, the encoder expects I42016
    /// input (packed u16 LE samples) and sets VP9 Profile 2.
    pub bit_depth: u8,
    /// Maximum keyframe interval in frames. `None` uses the libvpx default.
    pub kf_max_dist: Option<u32>,
    /// Minimum keyframe interval in frames. `None` uses the libvpx default.
    pub kf_min_dist: Option<u32>,
    /// Encoding thread count. `None` uses the libvpx default (auto).
    pub threads: Option<u32>,
    /// Speed / cpu-used control. Applied after encoder init via
    /// `VP8E_SET_CPUUSED`. Range depends on codec and deadline:
    /// VP8: -16..16, VP9 good: 0..9, VP9 realtime: 0..15.
    /// `None` uses the libvpx default.
    pub cpu_used: Option<i32>,
    /// Horizontal tile columns (log2). VP9 only; ignored for VP8.
    pub tile_columns: Option<i32>,
    /// Vertical tile rows (log2). VP9 only; ignored for VP8.
    pub tile_rows: Option<i32>,
    /// Minimum quantizer (0..63). `None` uses the libvpx default.
    pub rc_min_quantizer: Option<u32>,
    /// Maximum quantizer (0..63). `None` uses the libvpx default.
    pub rc_max_quantizer: Option<u32>,
}

/// A compressed output packet from the encoder.
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub is_keyframe: bool,
}

/// VP8/VP9 encoder.
pub struct Encoder {
    ctx: vpx_sys::vpx_codec_ctx_t,
    width: u32,
    height: u32,
    bit_depth: u8,
}

impl Encoder {
    /// Create a new encoder.
    pub fn new(config: &EncoderConfig) -> Result<Self> {
        let iface = config.codec.encoder_iface();

        let mut cfg =
            unsafe { MaybeUninit::<vpx_sys::vpx_codec_enc_cfg_t>::zeroed().assume_init() };
        let status =
            unsafe { vpx_sys::vpx_codec_enc_config_default(iface as *mut _, &mut cfg, 0) };
        error::check(status, None)?;

        cfg.g_w = config.width;
        cfg.g_h = config.height;
        cfg.g_timebase.num = config.timebase_num as i32;
        cfg.g_timebase.den = config.timebase_den as i32;

        match config.rate_control {
            RateControl::VBR(kbps) => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode_VPX_VBR;
                cfg.rc_target_bitrate = kbps;
            }
            RateControl::CBR(kbps) => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode_VPX_CBR;
                cfg.rc_target_bitrate = kbps;
            }
            RateControl::CQ { kbps, .. } => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode_VPX_CQ;
                cfg.rc_target_bitrate = kbps;
            }
            RateControl::Q => {
                cfg.rc_end_usage = vpx_sys::vpx_rc_mode_VPX_Q;
            }
        }

        if let Some(max) = config.kf_max_dist {
            cfg.kf_max_dist = max;
        }
        if let Some(min) = config.kf_min_dist {
            cfg.kf_min_dist = min;
        }
        if let Some(threads) = config.threads {
            cfg.g_threads = threads;
        }
        if let Some(min_q) = config.rc_min_quantizer {
            cfg.rc_min_quantizer = min_q;
        }
        if let Some(max_q) = config.rc_max_quantizer {
            cfg.rc_max_quantizer = max_q;
        }

        // 10-bit / high-bit-depth configuration
        let hbd = config.bit_depth > 8;
        if hbd {
            cfg.g_profile = 2; // VP9 Profile 2 (10/12-bit, 4:2:0)
            cfg.g_bit_depth = vpx_sys::vpx_bit_depth_VPX_BITS_10;
            cfg.g_input_bit_depth = config.bit_depth as u32;
        }

        let init_flags: vpx_sys::vpx_codec_flags_t = if hbd {
            vpx_sys::VPX_CODEC_USE_HIGHBITDEPTH as vpx_sys::vpx_codec_flags_t
        } else {
            0
        };

        let mut ctx =
            unsafe { MaybeUninit::<vpx_sys::vpx_codec_ctx_t>::zeroed().assume_init() };
        let status = unsafe {
            vpx_sys::vpx_codec_enc_init_ver(
                &mut ctx,
                iface as *mut _,
                &cfg,
                init_flags,
                vpx_sys::VPX_ENCODER_ABI_VERSION as i32,
            )
        };
        error::check(status, Some(&ctx))?;

        // Apply post-init controls
        if let Some(cpu_used) = config.cpu_used {
            let status = unsafe {
                vpx_sys::vpx_codec_control_(
                    &mut ctx,
                    vpx_sys::vp8e_enc_control_id_VP8E_SET_CPUUSED as i32,
                    cpu_used,
                )
            };
            error::check(status, Some(&ctx))?;
        }

        if let RateControl::CQ { cq_level, .. } = config.rate_control {
            let status = unsafe {
                vpx_sys::vpx_codec_control_(
                    &mut ctx,
                    vpx_sys::vp8e_enc_control_id_VP8E_SET_CQ_LEVEL as i32,
                    cq_level as i32,
                )
            };
            error::check(status, Some(&ctx))?;
        }

        // VP9-only tile controls
        if matches!(config.codec, Codec::VP9) {
            if let Some(cols) = config.tile_columns {
                let status = unsafe {
                    vpx_sys::vpx_codec_control_(
                        &mut ctx,
                        vpx_sys::vp8e_enc_control_id_VP9E_SET_TILE_COLUMNS as i32,
                        cols,
                    )
                };
                error::check(status, Some(&ctx))?;
            }
            if let Some(rows) = config.tile_rows {
                let status = unsafe {
                    vpx_sys::vpx_codec_control_(
                        &mut ctx,
                        vpx_sys::vp8e_enc_control_id_VP9E_SET_TILE_ROWS as i32,
                        rows,
                    )
                };
                error::check(status, Some(&ctx))?;
            }
        }

        Ok(Self {
            ctx,
            width: config.width,
            height: config.height,
            bit_depth: config.bit_depth,
        })
    }

    /// Encode a single I420 (8-bit) or I42016 (10-bit) frame.
    ///
    /// For 8-bit: `yuv_data` is packed I420 — Y (w*h), U (w/2*h/2), V (w/2*h/2).
    /// For 10-bit: `yuv_data` is packed I42016 — same layout but 2 bytes/sample (LE u16).
    pub fn encode(
        &mut self,
        pts: i64,
        duration: u64,
        yuv_data: &[u8],
        deadline: Deadline,
        flags: FrameFlags,
    ) -> Result<Vec<EncodedPacket>> {
        let w = self.width as usize;
        let h = self.height as usize;
        let bps: usize = if self.bit_depth > 8 { 2 } else { 1 };
        let expected = (w * h + 2 * (w / 2) * (h / 2)) * bps;
        if yuv_data.len() != expected {
            return Err(crate::Error::BadImageData {
                expected,
                got: yuv_data.len(),
            });
        }

        let img_fmt = if self.bit_depth > 8 {
            vpx_sys::vpx_img_fmt_VPX_IMG_FMT_I42016
        } else {
            vpx_sys::vpx_img_fmt_VPX_IMG_FMT_I420
        };

        let mut img =
            unsafe { MaybeUninit::<vpx_sys::vpx_image_t>::zeroed().assume_init() };
        let result = unsafe {
            vpx_sys::vpx_img_wrap(
                &mut img,
                img_fmt,
                self.width,
                self.height,
                1,
                yuv_data.as_ptr() as *mut u8,
            )
        };
        if result.is_null() {
            return Err(crate::Error::InvalidParam("vpx_img_wrap failed".into()));
        }

        let status = unsafe {
            vpx_sys::vpx_codec_encode(
                &mut self.ctx,
                &img,
                pts,
                duration as std::os::raw::c_ulong,
                flags.as_raw(),
                deadline.as_raw(),
            )
        };
        error::check(status, Some(&self.ctx))?;

        self.collect_packets()
    }

    /// Signal end of stream and flush all remaining packets.
    ///
    /// Repeatedly calls `vpx_codec_encode(NULL)` until libvpx has no more
    /// buffered data, collecting all output packets.
    pub fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
        let mut all_packets = Vec::new();

        loop {
            let status = unsafe {
                vpx_sys::vpx_codec_encode(
                    &mut self.ctx,
                    ptr::null(),
                    0,
                    0,
                    0,
                    Deadline::default().as_raw(),
                )
            };
            error::check(status, Some(&self.ctx))?;

            let packets = self.collect_packets()?;
            if packets.is_empty() {
                break;
            }
            all_packets.extend(packets);
        }

        Ok(all_packets)
    }

    /// Collect all pending compressed packets from the codec.
    fn collect_packets(&mut self) -> Result<Vec<EncodedPacket>> {
        let mut packets = Vec::new();
        let mut iter = ptr::null();

        loop {
            let pkt = unsafe { vpx_sys::vpx_codec_get_cx_data(&mut self.ctx, &mut iter) };
            if pkt.is_null() {
                break;
            }
            let pkt = unsafe { &*pkt };

            if pkt.kind == vpx_sys::vpx_codec_cx_pkt_kind_VPX_CODEC_CX_FRAME_PKT {
                let frame = unsafe { &pkt.data.frame };
                let data = unsafe {
                    std::slice::from_raw_parts(frame.buf as *const u8, frame.sz)
                };
                let is_keyframe = (frame.flags & vpx_sys::VPX_FRAME_IS_KEY) != 0;

                packets.push(EncodedPacket {
                    data: data.to_vec(),
                    pts: frame.pts,
                    is_keyframe,
                });
            }
        }

        Ok(packets)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            vpx_sys::vpx_codec_destroy(&mut self.ctx);
        }
    }
}
