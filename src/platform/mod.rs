#[cfg(target_os = "windows")]
mod icons;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use icons::{clear_image_cache, invalidate_image_cache};
#[cfg(target_os = "windows")]
pub use windows::*;
