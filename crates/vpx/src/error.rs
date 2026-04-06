use std::fmt;

/// Result type for vpx operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from libvpx operations.
#[derive(Debug)]
pub enum Error {
    /// A libvpx function returned an error code.
    Codec(String),
    /// Invalid parameter passed to the API.
    InvalidParam(String),
    /// Image data has wrong size.
    BadImageData { expected: usize, got: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Codec(msg) => write!(f, "vpx codec error: {}", msg),
            Error::InvalidParam(msg) => write!(f, "invalid parameter: {}", msg),
            Error::BadImageData { expected, got } => {
                write!(f, "image data length {}, expected {}", got, expected)
            }
        }
    }
}

impl std::error::Error for Error {}

/// Check a vpx status code and return an error if it indicates failure.
pub(crate) fn check(
    status: vpx_sys::vpx_codec_err_t,
    ctx: Option<&vpx_sys::vpx_codec_ctx_t>,
) -> Result<()> {
    if status == vpx_sys::vpx_codec_err_t_VPX_CODEC_OK {
        return Ok(());
    }
    let detail = ctx
        .map(|c| unsafe {
            let p = vpx_sys::vpx_codec_error_detail(c as *const _ as *mut _);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        })
        .unwrap_or_default();

    let name = unsafe {
        let p = vpx_sys::vpx_codec_err_to_string(status);
        if p.is_null() {
            format!("error code {}", status)
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };
    let msg = if detail.is_empty() {
        name
    } else {
        format!("{}: {}", name, detail)
    };
    Err(Error::Codec(msg))
}
