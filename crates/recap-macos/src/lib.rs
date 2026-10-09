//! Thin, safe wrappers over the macOS APIs RunReCap needs. All Objective-C
//! interop and `unsafe` lives in this crate (AGENTS.md §7).

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::{
    NSArray, NSBundle, NSDictionary, NSNumber, NSString, NSTimeZone, NSUserDefaults, NSURL,
};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// One connected display, in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenInfo {
    /// Stable CoreGraphics display number, as a string (used as a map key).
    pub id: String,
    pub width_px: u32,
    pub height_px: u32,
    pub scale: f64,
    /// Areas to keep clear (menu bar, notch, Dock), in pixels: top, right, bottom, left.
    pub safe_px: [f32; 4],
}

fn screen_id(screen: &NSScreen) -> String {
    let desc = screen.deviceDescription();
    desc.objectForKey(&NSString::from_str("NSScreenNumber"))
        .and_then(|o| o.downcast::<NSNumber>().ok())
        .map(|n| n.unsignedIntValue().to_string())
        .unwrap_or_else(|| "main".to_owned())
}

fn all_screens(mtm: MainThreadMarker) -> Retained<NSArray<NSScreen>> {
    NSScreen::screens(mtm)
}

fn find_screen(mtm: MainThreadMarker, id: &str) -> Option<Retained<NSScreen>> {
    all_screens(mtm).iter().find(|s| screen_id(s) == id)
}

pub fn screens(mtm: MainThreadMarker) -> Vec<ScreenInfo> {
    all_screens(mtm)
        .iter()
        .map(|s| {
            let scale = s.backingScaleFactor();
            let frame = s.frame();
            let visible = s.visibleFrame();
            let insets = s.safeAreaInsets();
            // visibleFrame excludes the menu bar and Dock; safeAreaInsets covers the notch.
            let top = (frame.origin.y + frame.size.height
                - (visible.origin.y + visible.size.height))
                .max(insets.top);
            let bottom = (visible.origin.y - frame.origin.y).max(insets.bottom);
            let left = (visible.origin.x - frame.origin.x).max(insets.left);
            let right = (frame.origin.x + frame.size.width
                - (visible.origin.x + visible.size.width))
                .max(insets.right);
            let px = |v: f64| (v * scale) as f32;
            ScreenInfo {
                id: screen_id(&s),
                width_px: (frame.size.width * scale).round() as u32,
                height_px: (frame.size.height * scale).round() as u32,
                scale,
                safe_px: [px(top), px(right), px(bottom), px(left)],
            }
        })
        .collect()
}

/// Set the desktop picture for one screen (current Space).
pub fn set_wallpaper(mtm: MainThreadMarker, screen: &str, image: &Path) -> Result<(), String> {
    let screen = find_screen(mtm, screen).ok_or_else(|| format!("screen {screen} not found"))?;
    let url =
        NSURL::from_file_path(image).ok_or_else(|| format!("bad path {}", image.display()))?;
    let options = NSDictionary::new();
    // SAFETY: an empty options dictionary is always valid for this API.
    unsafe {
        NSWorkspace::sharedWorkspace()
            .setDesktopImageURL_forScreen_options_error(&url, &screen, &options)
    }
    .map_err(|e| e.localizedDescription().to_string())
}

/// The screen's current desktop picture, if it's a file.
pub fn wallpaper(mtm: MainThreadMarker, screen: &str) -> Option<PathBuf> {
    let screen = find_screen(mtm, screen)?;
    NSWorkspace::sharedWorkspace()
        .desktopImageURLForScreen(&screen)?
        .to_file_path()
}

/// Path of the running `.app` bundle, or None when run outside a bundle
/// (e.g. `cargo run`).
pub fn app_bundle_path() -> Option<PathBuf> {
    let path = PathBuf::from(NSBundle::mainBundle().bundlePath().to_string());
    path.extension().is_some_and(|e| e == "app").then_some(path)
}

/// True when the system appearance is Dark.
pub fn system_is_dark() -> bool {
    NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str("AppleInterfaceStyle"))
        .is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark"))
}

pub fn utc_offset_s() -> i32 {
    NSTimeZone::localTimeZone().secondsFromGMT() as i32
}

/// Open a file or folder with its default app.
pub fn open(path: &Path) -> bool {
    NSURL::from_file_path(path).is_some_and(|u| NSWorkspace::sharedWorkspace().openURL(&u))
}

/// Launch at login works only when running from a signed `.app` bundle.
pub fn login_item_enabled() -> bool {
    // SAFETY: plain class/instance getters with no preconditions.
    unsafe { SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled }
}

pub fn set_login_item(enabled: bool) -> Result<(), String> {
    // SAFETY: register/unregister have no preconditions; errors are returned as NSError.
    let service = unsafe { SMAppService::mainAppService() };
    let result = if enabled {
        unsafe { service.registerAndReturnError() }
    } else {
        unsafe { service.unregisterAndReturnError() }
    };
    result.map_err(|e| e.localizedDescription().to_string())
}

/// Decode any image macOS can read (JPEG, HEIC, PNG, TIFF, WebP…), apply its
/// EXIF orientation, shrink it so the long side is at most `max_side` (never
/// enlarges), and write it as PNG for the renderer. Returns the output size.
pub fn convert_image_to_png(src: &Path, dst: &Path, max_side: u32) -> Result<(u32, u32), String> {
    use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFString, CFType, CFURL};
    use objc2_core_graphics::CGImage;
    use objc2_image_io::{
        kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
        kCGImageSourceThumbnailMaxPixelSize, CGImageDestination, CGImageSource,
    };

    let src_url =
        CFURL::from_file_path(src).ok_or_else(|| format!("bad path {}", src.display()))?;
    let dst_url =
        CFURL::from_file_path(dst).ok_or_else(|| format!("bad path {}", dst.display()))?;
    // SAFETY: valid file URL, no options.
    let source = unsafe { CGImageSource::with_url(&src_url, None) }
        .ok_or_else(|| "macOS can't read this file as an image".to_owned())?;

    let yes: &CFType = CFBoolean::new(true).as_ref();
    let max = CFNumber::new_i64(max_side as i64);
    // SAFETY: reading immutable ImageIO constants.
    let keys: [&CFString; 3] = unsafe {
        [
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform,
            kCGImageSourceThumbnailMaxPixelSize,
        ]
    };
    let values: [&CFType; 3] = [yes, yes, max.as_ref()];
    let options = CFDictionary::<CFString, CFType>::from_slices(&keys, &values);

    // SAFETY: index 0 exists for any decodable image; options have the
    // documented key/value types (bool, bool, number).
    let image = unsafe { source.thumbnail_at_index(0, Some(options.as_ref())) }
        .ok_or_else(|| "couldn't decode the image".to_owned())?;
    let size = (
        CGImage::width(Some(&image)) as u32,
        CGImage::height(Some(&image)) as u32,
    );

    let png = CFString::from_static_str("public.png");
    // SAFETY: valid URL and UTI; one image, no properties.
    unsafe {
        let dest = CGImageDestination::with_url(&dst_url, &png, 1, None)
            .ok_or_else(|| format!("can't write {}", dst.display()))?;
        dest.add_image(&image, None);
        if !dest.finalize() {
            return Err(format!("writing {} failed", dst.display()));
        }
    }
    Ok(size)
}
