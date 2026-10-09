//! The main RunReCap window: a live wallpaper preview that doubles as a drop
//! zone, run navigation, layout/theme pickers, import buttons and the
//! "Set as Wallpaper" action. Stock AppKit controls only (AGENTS.md §6.2 tier 3);
//! the preview frame uses Design.md tokens (8px radius, no border at rest).

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBitmapImageFileType, NSButton, NSColor,
    NSControl, NSDragOperation, NSDraggingInfo, NSFont, NSFontWeightSemibold, NSImage,
    NSImageScaling, NSImageView, NSLineBreakMode, NSPasteboardTypeFileURL, NSPopUpButton, NSSlider,
    NSTextAlignment, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};

/// Content width of the window, in points.
const W: f64 = 880.0;
const PAD: f64 = 24.0;
const ROW: f64 = 32.0;

define_class!(
    // SAFETY: NSView has no subclassing requirements for this use; no Drop impl.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "RunReCapDropView"]
    struct DropView;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for DropView {}

    impl DropView {
        // SAFETY: matches `- (NSDragOperation)draggingEntered:(id<NSDraggingInfo>)sender`.
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if dragged_paths(info).is_empty() {
                NSDragOperation::None
            } else {
                self.set_highlight(true);
                NSDragOperation::Copy
            }
        }

        // SAFETY: matches `- (void)draggingExited:(id<NSDraggingInfo>)sender`.
        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _info: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            self.set_highlight(false);
        }

        // SAFETY: matches `- (BOOL)performDragOperation:(id<NSDraggingInfo>)sender`.
        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, info: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.set_highlight(false);
            let paths = dragged_paths(info);
            let accepted = !paths.is_empty();
            if accepted {
                crate::app::import_dropped(paths);
            }
            accepted
        }
    }
);

impl DropView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSView's designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(8.0);
            layer.setMasksToBounds(true);
        }
        // SAFETY: reading an immutable AppKit constant.
        let types = NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]);
        view.registerForDraggedTypes(&types);
        view.set_highlight(false);
        view
    }

    fn set_highlight(&self, on: bool) {
        if let Some(layer) = self.layer() {
            // Accent (system) only while a drag is over the drop zone.
            let color = if on {
                NSColor::controlAccentColor()
            } else {
                NSColor::quaternaryLabelColor()
            };
            layer.setBorderWidth(if on { 2.0 } else { 0.0 });
            layer.setBorderColor(Some(&color.CGColor()));
            layer.setBackgroundColor(Some(&NSColor::quaternarySystemFillColor().CGColor()));
        }
    }
}

fn dragged_paths(info: &ProtocolObject<dyn NSDraggingInfo>) -> Vec<PathBuf> {
    let pb = info.draggingPasteboard();
    let Some(items) = pb.pasteboardItems() else {
        return Vec::new();
    };
    items
        .iter()
        // SAFETY: reading an immutable AppKit constant.
        .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
        .filter_map(|s| NSURL::URLWithString(&s))
        .filter_map(|u| u.to_file_path())
        .collect()
}

/// Everything the window shows; built by the app from engine + UI state.
pub struct ViewModel<'a> {
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub has_runs: bool,
    pub busy: bool,
    pub layout_index: usize,
    pub layouts: Vec<&'static str>,
    /// Index 0 is "Automatic".
    pub themes: Vec<String>,
    pub theme_index: usize,
    /// True when RunReCap currently owns the desktop picture.
    pub active: bool,
    pub preview_png: Option<&'a [u8]>,
    /// e.g. "Version 0.1.0".
    pub version: String,
    /// e.g. "Update to 0.2.0"; None hides the button.
    pub update_title: Option<String>,
    pub update_enabled: bool,
    /// Background row: photo name/size or the recommended resolution.
    pub background_hint: String,
    pub has_background: bool,
    /// Photo dimming, 0.0–0.7.
    pub dim: f64,
}

pub struct MainWindow {
    pub window: Retained<NSWindow>,
    preview: Retained<NSImageView>,
    empty: Retained<NSTextField>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    status: Retained<NSTextField>,
    layout: Retained<NSPopUpButton>,
    theme: Retained<NSPopUpButton>,
    nav: Vec<Retained<NSButton>>,
    imports: Vec<Retained<NSButton>>,
    apply: Retained<NSButton>,
    version: Retained<NSTextField>,
    update_button: Retained<NSButton>,
    background_buttons: Vec<Retained<NSButton>>,
    remove_background: Retained<NSButton>,
    dim: Retained<NSSlider>,
    background_hint: Retained<NSTextField>,
    /// Left edge of the status label (right of the import buttons).
    status_x: f64,
    /// Preview size in points (the render is 2× this).
    pub preview_size: NSSize,
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn label(mtm: MainThreadMarker, text: &str) -> Retained<NSTextField> {
    NSTextField::labelWithString(&NSString::from_str(text), mtm)
}

fn button(
    mtm: MainThreadMarker,
    title: &str,
    target: &AnyObject,
    action: Sel,
) -> Retained<NSButton> {
    // SAFETY: `target` implements `action` (methods on the app delegate).
    unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            mtm,
        )
    }
}

/// Size `view` to fit its content and place its bottom-left at (x, y).
/// Returns the fitted width.
fn place(view: &NSView, x: f64, y: f64) -> f64 {
    let size = view.fittingSize();
    view.setFrame(rect(x, y, size.width, size.height.max(ROW - 4.0)));
    size.width
}

fn popup(mtm: MainThreadMarker, target: &AnyObject, action: Sel) -> Retained<NSPopUpButton> {
    let p = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm),
        rect(0.0, 0.0, 190.0, 28.0),
        false,
    );
    // SAFETY: `target` implements `action`.
    unsafe {
        p.setTarget(Some(target));
        p.setAction(Some(action));
    }
    p
}

impl MainWindow {
    /// `aspect` = main screen height / width, so the preview matches the desktop.
    pub fn new(mtm: MainThreadMarker, target: &AnyObject, aspect: f64) -> Self {
        let aspect = aspect.clamp(0.3, 0.8);
        let pw = W - 2.0 * PAD;
        let ph = (pw * aspect).round();

        // Rows from the bottom (AppKit's origin is bottom-left).
        let y_actions = PAD;
        let y_background = y_actions + ROW + 12.0;
        let y_controls = y_background + ROW + 12.0;
        let y_subtitle = y_controls + ROW + 20.0;
        let y_title = y_subtitle + 20.0;
        let y_preview = y_title + 28.0 + 16.0;
        let h = y_preview + ph + PAD;

        // SAFETY: standard window creation; released-when-closed is disabled
        // below because we keep (and reopen) the window ourselves.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(0.0, 0.0, W, h),
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: see above.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str("RunReCap"));
        let content = window.contentView().expect("window has a content view");

        // Preview / drop zone.
        let drop = DropView::new(mtm, rect(PAD, y_preview, pw, ph));
        let preview = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect(0.0, 0.0, pw, ph));
        preview.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        preview.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        drop.addSubview(&preview);
        let empty = NSTextField::wrappingLabelWithString(
            &NSString::from_str(
                "Drop .gpx files or your Strava export (.zip) here\n\
                 Drop a photo to use it as the background\n\n\
                 Get your Strava export at strava.com → Settings → My Account →\n\
                 Download or Delete Your Account → Request your archive.",
            ),
            mtm,
        );
        empty.setAlignment(NSTextAlignment::Center);
        empty.setTextColor(Some(&NSColor::secondaryLabelColor()));
        empty.setFont(Some(&NSFont::systemFontOfSize(15.0)));
        empty.setFrame(rect(PAD, ph / 2.0 - 50.0, pw - 2.0 * PAD, 100.0));
        drop.addSubview(&empty);
        content.addSubview(&drop);

        // Run title + stats.
        let title = label(mtm, "");
        // SAFETY: reading an immutable AppKit constant.
        title.setFont(Some(&NSFont::systemFontOfSize_weight(20.0, unsafe {
            NSFontWeightSemibold
        })));
        title.setFrame(rect(PAD, y_title, pw, 28.0));
        title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        content.addSubview(&title);
        let subtitle = label(mtm, "");
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        subtitle.setFrame(rect(PAD, y_subtitle, pw - 160.0, 18.0));
        subtitle.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        content.addSubview(&subtitle);
        let version = label(mtm, "");
        version.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        version.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        version.setAlignment(NSTextAlignment::Right);
        version.setFrame(rect(W - PAD - 150.0, y_subtitle + 1.0, 150.0, 16.0));
        content.addSubview(&version);

        // Controls row: navigation left, pickers right.
        let mut x = PAD;
        let mut nav = Vec::new();
        for (t, a) in [
            ("◀ Previous", sel!(previous:)),
            ("Next ▶", sel!(next:)),
            ("Shuffle", sel!(shuffle:)),
        ] {
            let b = button(mtm, t, target, a);
            x += place(&b, x, y_controls) + 8.0;
            content.addSubview(&b);
            nav.push(b);
        }
        let theme = popup(mtm, target, sel!(themeChanged:));
        let layout = popup(mtm, target, sel!(layoutChanged:));
        let mut right = W - PAD;
        for (name, p) in [("Theme", &theme), ("Layout", &layout)] {
            right -= 190.0;
            p.setFrame(rect(right, y_controls + 2.0, 190.0, 28.0));
            content.addSubview(p);
            let l = label(mtm, name);
            let lw = l.fittingSize().width;
            right -= lw + 6.0;
            l.setFrame(rect(right, y_controls + 7.0, lw, 18.0));
            content.addSubview(&l);
            right -= 20.0;
        }

        // Background row: upload / remove a photo, dim it, and the size guidance.
        let mut x = PAD;
        let bg_label = label(mtm, "Background");
        x += place(&bg_label, x, y_background + 7.0) + 10.0;
        bg_label.setFrame(rect(
            PAD,
            y_background + 7.0,
            bg_label.fittingSize().width,
            18.0,
        ));
        content.addSubview(&bg_label);
        let choose = button(mtm, "Choose Photo…", target, sel!(chooseBackground:));
        x += place(&choose, x, y_background) + 8.0;
        content.addSubview(&choose);
        let remove_background = button(mtm, "Remove", target, sel!(removeBackground:));
        x += place(&remove_background, x, y_background) + 16.0;
        content.addSubview(&remove_background);
        let dim_label = label(mtm, "Dim");
        dim_label.setFrame(rect(
            x,
            y_background + 7.0,
            dim_label.fittingSize().width,
            18.0,
        ));
        x += dim_label.fittingSize().width + 6.0;
        content.addSubview(&dim_label);
        // SAFETY: `target` implements `dimChanged:`.
        let dim = unsafe {
            NSSlider::sliderWithValue_minValue_maxValue_target_action(
                0.3,
                0.0,
                0.7,
                Some(target),
                Some(sel!(dimChanged:)),
                mtm,
            )
        };
        // Re-render once on release, not on every pixel of the drag.
        dim.setContinuous(false);
        dim.setFrame(rect(x, y_background + 4.0, 110.0, 22.0));
        x += 110.0 + 14.0;
        content.addSubview(&dim);
        let background_hint = label(mtm, "");
        background_hint.setTextColor(Some(&NSColor::secondaryLabelColor()));
        background_hint.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        background_hint.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        background_hint.setFrame(rect(x, y_background + 7.0, W - PAD - x, 16.0));
        content.addSubview(&background_hint);

        // Actions row: imports left, status middle, apply right.
        let mut x = PAD;
        let mut imports = Vec::new();
        for (t, a) in [
            ("Import GPX Files…", sel!(importFiles:)),
            ("Import Strava Export…", sel!(importStrava:)),
        ] {
            let b = button(mtm, t, target, a);
            x += place(&b, x, y_actions) + 8.0;
            content.addSubview(&b);
            imports.push(b);
        }
        let apply = button(mtm, "Restore My Wallpaper", target, sel!(togglePause:));
        let aw = apply.fittingSize().width;
        apply.setFrame(rect(W - PAD - aw, y_actions, aw, ROW - 4.0));
        content.addSubview(&apply);
        let update_button = button(mtm, "Update", target, sel!(installUpdate:));
        update_button.setHidden(true);
        content.addSubview(&update_button);
        let status = label(mtm, "");
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        status.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        status.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        status.setFrame(rect(
            x + 8.0,
            y_actions + 6.0,
            (W - PAD - aw - 16.0) - (x + 8.0),
            16.0,
        ));
        content.addSubview(&status);

        window.center();
        Self {
            window,
            preview,
            empty,
            title,
            subtitle,
            status,
            layout,
            theme,
            nav,
            imports,
            apply,
            version,
            update_button,
            background_buttons: vec![choose],
            remove_background,
            dim,
            background_hint,
            status_x: x + 8.0,
            preview_size: NSSize::new(pw, ph),
        }
    }

    pub fn update(&self, vm: &ViewModel) {
        self.title.setStringValue(&NSString::from_str(&vm.title));
        self.subtitle
            .setStringValue(&NSString::from_str(&vm.subtitle));
        self.status.setStringValue(&NSString::from_str(&vm.status));

        let image = vm
            .preview_png
            .and_then(|png| NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)));
        if let Some(img) = &image {
            img.setSize(self.preview_size);
        }
        self.preview.setImage(image.as_deref());
        self.empty.setHidden(vm.has_runs);

        fill_popup(
            &self.layout,
            vm.layouts.iter().map(|s| s.to_string()),
            vm.layout_index,
        );
        fill_popup(&self.theme, vm.themes.iter().cloned(), vm.theme_index);

        for b in &self.nav {
            set_enabled(b, vm.has_runs && !vm.busy);
        }
        set_enabled(&self.layout, vm.has_runs);
        set_enabled(&self.theme, vm.has_runs);
        for b in &self.imports {
            set_enabled(b, !vm.busy);
        }

        // Design.md: the accent goes on the one primary action only.
        let (title, key) = if vm.active {
            ("Restore My Wallpaper", "")
        } else {
            ("Set as Wallpaper", "\r")
        };
        self.apply.setTitle(&NSString::from_str(title));
        self.apply.setKeyEquivalent(&NSString::from_str(key));
        set_enabled(&self.apply, vm.has_runs && !vm.busy);
        self.background_hint
            .setStringValue(&NSString::from_str(&vm.background_hint));
        self.background_hint
            .setToolTip(Some(&NSString::from_str(&vm.background_hint)));
        for b in &self.background_buttons {
            set_enabled(b, !vm.busy);
        }
        set_enabled(&self.remove_background, vm.has_background && !vm.busy);
        set_enabled(&self.dim, vm.has_background && !vm.busy);
        self.dim.setDoubleValue(vm.dim);
        self.version
            .setStringValue(&NSString::from_str(&vm.version));

        // Right side of the actions row: [Update to x.y.z] [Set as Wallpaper].
        let frame = self.apply.frame();
        let aw = self.apply.fittingSize().width;
        let mut left = W - PAD - aw;
        self.apply
            .setFrame(rect(left, frame.origin.y, aw, frame.size.height));
        match &vm.update_title {
            Some(t) => {
                self.update_button.setTitle(&NSString::from_str(t));
                let uw = self.update_button.fittingSize().width;
                left -= uw + 8.0;
                self.update_button
                    .setFrame(rect(left, frame.origin.y, uw, frame.size.height));
                self.update_button.setHidden(false);
                set_enabled(&self.update_button, vm.update_enabled);
            }
            None => self.update_button.setHidden(true),
        }
        let sf = self.status.frame();
        self.status.setFrame(rect(
            self.status_x,
            sf.origin.y,
            (left - 8.0 - self.status_x).max(0.0),
            sf.size.height,
        ));
    }
}

impl MainWindow {
    /// PNG of the window's content (dev aid for UI checks; see AGENTS.md §9).
    pub fn snapshot_png(&self) -> Option<Vec<u8>> {
        let view = self.window.contentView()?;
        let bounds = view.bounds();
        let rep = view.bitmapImageRepForCachingDisplayInRect(bounds)?;
        view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
        // SAFETY: an empty properties dictionary is valid for PNG.
        let data = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        Some(data.to_vec())
    }
}

fn set_enabled(control: &NSControl, on: bool) {
    control.setEnabled(on);
}

fn fill_popup(p: &NSPopUpButton, titles: impl Iterator<Item = String>, selected: usize) {
    p.removeAllItems();
    for (i, t) in titles.enumerate() {
        p.addItemWithTitle(&NSString::from_str(&t));
        if let Some(item) = p.lastItem() {
            item.setTag(i as isize);
        }
    }
    p.selectItemAtIndex(selected as isize);
}
