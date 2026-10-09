//! User-uploaded wallpaper photos that the route is drawn on top of.
//!
//! Uploads are converted once (any format macOS reads, EXIF orientation
//! applied, capped in size) to `backgrounds/<hash>.png` in the data dir, so
//! the renderer only ever reads PNG.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use recap_macos::ScreenInfo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Default darkening so the route and text stay readable on busy photos.
pub const DEFAULT_DIM: f64 = 0.3;

/// Recommended size when no screen info is available (13"/14" MacBook class).
const FALLBACK: (u32, u32) = (2880, 1800);

/// Formats accepted for upload (decoded by macOS ImageIO).
pub const IMAGE_EXTENSIONS: [&str; 11] = [
    "png", "jpg", "jpeg", "heic", "heif", "tif", "tiff", "webp", "gif", "bmp", "jxl",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackgroundImage {
    /// Converted PNG, relative to the data dir.
    pub file: PathBuf,
    /// Original file name, for display.
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// 0 = untouched, 0.7 = very dark.
    #[serde(default = "default_dim")]
    pub dim: f64,
}

fn default_dim() -> f64 {
    DEFAULT_DIM
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Smallest image that covers every connected display without upscaling:
/// the largest width and largest height across screens, in pixels.
pub fn recommended_size(screens: &[ScreenInfo]) -> (u32, u32) {
    let w = screens.iter().map(|s| s.width_px.max(s.height_px)).max();
    let h = screens.iter().map(|s| s.width_px.min(s.height_px)).max();
    match (w, h) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
        _ => FALLBACK,
    }
}

/// One-line guidance shown next to the upload button.
pub fn recommendation_text(rec: (u32, u32)) -> String {
    format!("Recommended: {} × {} px or larger, landscape", rec.0, rec.1)
}

/// Warning when the photo is smaller than the screen and will be upscaled.
pub fn low_res_warning(bg: &BackgroundImage, rec: (u32, u32)) -> Option<String> {
    // Aspect-fill scale needed to cover the recommended area.
    let scale = (rec.0 as f32 / bg.width.max(1) as f32).max(rec.1 as f32 / bg.height.max(1) as f32);
    (scale > 1.05).then(|| {
        format!(
            "{} × {} px is smaller than your display ({} × {}), so it may look soft.",
            bg.width, bg.height, rec.0, rec.1
        )
    })
}

/// Convert `src` into the data dir and describe it. Previous uploads are
/// removed. Runs on a worker thread.
pub fn import(data_dir: &Path, src: &Path, rec: (u32, u32), dim: f64) -> Result<BackgroundImage> {
    let bytes = fs::read(src).with_context(|| format!("reading {}", src.display()))?;
    let hash: String = Sha256::digest(&bytes)[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let dir = data_dir.join("backgrounds");
    fs::create_dir_all(&dir)?;
    let file = PathBuf::from("backgrounds").join(format!("{hash}.png"));
    let dst = data_dir.join(&file);

    // Keep enough pixels for the largest display (and 4K+ for future screens),
    // but cap memory: an 8K RGBA image is already ~130 MB while rendering.
    let max_side = rec.0.max(rec.1).clamp(3840, 8192);
    let (width, height) =
        recap_macos::convert_image_to_png(src, &dst, max_side).map_err(anyhow::Error::msg)?;

    for entry in fs::read_dir(&dir)?.flatten() {
        let p = entry.path();
        if p != dst && p.extension().is_some_and(|e| e == "png") {
            let _ = fs::remove_file(p);
        }
    }
    let name = src
        .file_name()
        .map_or_else(|| "photo".into(), |n| n.to_string_lossy().into_owned());
    Ok(BackgroundImage {
        file,
        name,
        width,
        height,
        dim,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(w: u32, h: u32) -> ScreenInfo {
        ScreenInfo {
            id: "1".into(),
            width_px: w,
            height_px: h,
            scale: 2.0,
            safe_px: [0.0; 4],
        }
    }

    #[test]
    fn recommendation_covers_all_screens() {
        assert_eq!(
            recommended_size(&[screen(2880, 1800), screen(2560, 1080)]),
            (2880, 1800)
        );
        assert_eq!(
            recommended_size(&[screen(3024, 1964), screen(5120, 2880)]),
            (5120, 2880)
        );
        assert_eq!(recommended_size(&[]), FALLBACK);
    }

    #[test]
    fn warns_only_when_upscaled() {
        let bg = |w, h| BackgroundImage {
            file: "x.png".into(),
            name: "x".into(),
            width: w,
            height: h,
            dim: 0.3,
        };
        assert!(low_res_warning(&bg(1920, 1080), (2880, 1800)).is_some());
        assert!(low_res_warning(&bg(4032, 3024), (2880, 1800)).is_none());
        // Wide panorama: tall enough? 6000×1500 must scale up to 1800 high.
        assert!(low_res_warning(&bg(6000, 1500), (2880, 1800)).is_some());
    }

    #[test]
    fn image_extensions() {
        assert!(is_image(Path::new("/a/IMG_0001.HEIC")));
        assert!(is_image(Path::new("beach.jpg")));
        assert!(!is_image(Path::new("run.gpx")));
    }
}
