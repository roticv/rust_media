/// Codec selection for VP8 or VP9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    VP8,
    VP9,
}

impl Codec {
    pub(crate) fn encoder_iface(&self) -> *const vpx_sys::vpx_codec_iface {
        unsafe {
            match self {
                Codec::VP8 => vpx_sys::vpx_codec_vp8_cx(),
                Codec::VP9 => vpx_sys::vpx_codec_vp9_cx(),
            }
        }
    }

    pub(crate) fn decoder_iface(&self) -> *const vpx_sys::vpx_codec_iface {
        unsafe {
            match self {
                Codec::VP8 => vpx_sys::vpx_codec_vp8_dx(),
                Codec::VP9 => vpx_sys::vpx_codec_vp9_dx(),
            }
        }
    }
}
