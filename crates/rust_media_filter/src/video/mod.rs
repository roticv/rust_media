//! Video filters

pub mod scale;
pub mod ssim;

pub use scale::ScaleFilter;
pub use ssim::{calculate_frame_ssim, ssim_to_db};
