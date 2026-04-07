//! Video filters

pub mod ssim;

pub use ssim::{calculate_frame_ssim, ssim_to_db};
