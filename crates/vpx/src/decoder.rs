use crate::error::{self, Result};
use crate::image::Codec;
use std::mem::MaybeUninit;
use std::ptr;

/// Configuration for creating a decoder.
pub struct DecoderConfig {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
}

/// A decoded video frame in I420 format.
pub struct DecodedFrame {
    pub width: usize,
    pub height: usize,
    /// Y plane data (packed, width * height bytes)
    pub y: Vec<u8>,
    pub y_stride: usize,
    /// U plane data (packed)
    pub u: Vec<u8>,
    pub u_stride: usize,
    /// V plane data (packed)
    pub v: Vec<u8>,
    pub v_stride: usize,
}

/// VP8/VP9 decoder.
pub struct Decoder {
    ctx: vpx_sys::vpx_codec_ctx_t,
}

impl Decoder {
    /// Create a new decoder.
    pub fn new(config: &DecoderConfig) -> Result<Self> {
        let iface = config.codec.decoder_iface();

        let mut cfg = unsafe { MaybeUninit::<vpx_sys::vpx_codec_dec_cfg_t>::zeroed().assume_init() };
        cfg.w = config.width;
        cfg.h = config.height;
        cfg.threads = 1;

        let mut ctx = unsafe { MaybeUninit::<vpx_sys::vpx_codec_ctx_t>::zeroed().assume_init() };

        let status = unsafe {
            vpx_sys::vpx_codec_dec_init_ver(
                &mut ctx,
                iface as *mut _,
                &cfg,
                0,
                vpx_sys::VPX_DECODER_ABI_VERSION as i32,
            )
        };
        error::check(status, Some(&ctx))?;

        Ok(Self { ctx })
    }

    /// Decode a compressed packet. Returns decoded frames (may be zero or more).
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<DecodedFrame>> {
        let status = unsafe {
            vpx_sys::vpx_codec_decode(
                &mut self.ctx,
                data.as_ptr(),
                data.len() as u32,
                ptr::null_mut(),
                0,
            )
        };
        error::check(status, Some(&self.ctx))?;

        self.collect_frames()
    }

    /// Collect all available decoded frames from the codec.
    fn collect_frames(&mut self) -> Result<Vec<DecodedFrame>> {
        let mut frames = Vec::new();
        let mut iter = ptr::null();

        loop {
            let img = unsafe { vpx_sys::vpx_codec_get_frame(&mut self.ctx, &mut iter) };
            if img.is_null() {
                break;
            }

            let img = unsafe { &*img };
            let w = img.d_w as usize;
            let h = img.d_h as usize;

            let y_stride = img.stride[0] as usize;
            let u_stride = img.stride[1] as usize;
            let v_stride = img.stride[2] as usize;

            let y_ptr = img.planes[0];
            let u_ptr = img.planes[1];
            let v_ptr = img.planes[2];

            let chroma_h = (h + 1) / 2;

            let y = copy_plane(y_ptr, y_stride, w, h);
            let u = copy_plane(u_ptr, u_stride, (w + 1) / 2, chroma_h);
            let v = copy_plane(v_ptr, v_stride, (w + 1) / 2, chroma_h);

            frames.push(DecodedFrame {
                width: w,
                height: h,
                y,
                y_stride,
                u,
                u_stride,
                v,
                v_stride,
            });
        }

        Ok(frames)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            vpx_sys::vpx_codec_destroy(&mut self.ctx);
        }
    }
}

/// Copy pixel data from a strided buffer into a packed Vec.
fn copy_plane(ptr: *mut u8, stride: usize, width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height);
    for row in 0..height {
        let src = unsafe { std::slice::from_raw_parts(ptr.add(row * stride), width) };
        out.extend_from_slice(src);
    }
    out
}
