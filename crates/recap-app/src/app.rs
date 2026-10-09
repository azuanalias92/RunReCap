//! The app: main window (preview + import), status item + menu, wallpaper
//! application, rotation, and reacting to screen / Space / appearance / wake
//! changes. The desktop is only changed after "Set as Wallpaper".
//!
//! Threading: AppKit is touched only on the main thread. Rendering and imports
//! run on worker threads and hand results back via the main dispatch queue.

use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{Context, Result};
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{
    define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationActivationPolicy,
    NSApplicationDidChangeScreenParametersNotification, NSBeep, NSControlStateValueOff,
    NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSPopUpButton, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSWindowDelegate, NSWorkspace,
    NSWorkspaceActiveSpaceDidChangeNotification, NSWorkspaceDidWakeNotification,
};
use objc2_foundation::{
    NSData, NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol, NSSize, NSString, NSTimer,
};
use recap_core::ImportReport;
use recap_macos::ScreenInfo;
use recap_render::{caption, Canvas, Fonts, Insets, Layout};

use crate::background;
use crate::config::{Order, AUTOMATIC};
use crate::engine::{Engine, RenderJob};
use crate::update::{self, Release};
use crate::window::{MainWindow, ViewModel};

/// `NSModalResponseOK`.
const MODAL_OK: isize = 1;
const ROTATION_CHOICES: [(u32, &str); 6] = [
    (0, "Never"),
    (15, "Every 15 Minutes"),
    (60, "Every Hour"),
    (240, "Every 4 Hours"),
    (1440, "Every Day"),
    (10080, "Every Week"),
];

thread_local! {
    static DELEGATE: OnceCell<Retained<Delegate>> = const { OnceCell::new() };
}

fn with_delegate(f: impl FnOnce(&Delegate)) {
    DELEGATE.with(|d| {
        if let Some(d) = d.get() {
            f(d)
        }
    });
}

#[derive(Default)]
struct Ui {
    rendering: bool,
    /// A refresh was requested while a render was running.
    dirty: bool,
    importing: bool,
    /// Last applied file per screen, re-applied when the Space changes.
    applied: Vec<(String, PathBuf)>,
    thumb: Option<Vec<u8>>,
    /// Main-screen-shaped preview shown in the window.
    preview: Option<Vec<u8>>,
    /// Last import summary, shown in the window.
    status: String,
    warnings: Vec<String>,
    config_mtime: Option<SystemTime>,
    last_dark: Option<bool>,
    /// Newer release found by the last update check.
    update: Option<Release>,
    /// Downloading / verifying an update.
    update_busy: bool,
    /// The launch-time update check has finished (or there's no feed).
    update_checked: bool,
    last_update_check: i64,
    /// Converting an uploaded photo.
    background_busy: bool,
}

struct Ivars {
    engine: Arc<Mutex<Engine>>,
    status_item: OnceCell<Retained<NSStatusItem>>,
    window: OnceCell<MainWindow>,
    ui: RefCell<Ui>,
}

struct RenderOutput {
    files: Vec<(String, PathBuf)>,
    thumb: Option<Vec<u8>>,
    preview: Option<Vec<u8>>,
    warnings: Vec<String>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; Delegate has no Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RunReCapDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    // SAFETY: NSObjectProtocol has no safety requirements.
    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        // SAFETY (all methods below): signatures match `- (void)name:(id)sender`.
        #[unsafe(method(next:))]
        fn next(&self, _sender: &AnyObject) {
            self.mutate(|e| e.step(1));
        }

        #[unsafe(method(previous:))]
        fn previous(&self, _sender: &AnyObject) {
            self.mutate(|e| e.step(-1));
        }

        #[unsafe(method(shuffle:))]
        fn shuffle(&self, _sender: &AnyObject) {
            self.mutate(|e| e.shuffle());
        }

        #[unsafe(method(togglePin:))]
        fn toggle_pin(&self, _sender: &AnyObject) {
            self.mutate(|e| e.state.pinned = !e.state.pinned);
        }

        #[unsafe(method(setLayout:))]
        fn set_layout(&self, sender: &NSMenuItem) {
            let layout = Layout::ALL[sender.tag() as usize % Layout::ALL.len()];
            self.mutate(|e| e.config.layout = layout);
        }

        #[unsafe(method(setTheme:))]
        fn set_theme(&self, sender: &NSMenuItem) {
            let tag = sender.tag() as usize;
            self.mutate(|e| {
                e.config.theme = match tag {
                    0 => AUTOMATIC.to_owned(),
                    i => e.config.all_themes().get(i - 1).map_or(AUTOMATIC.to_owned(), |t| t.name.clone()),
                };
            });
        }

        #[unsafe(method(layoutChanged:))]
        fn layout_changed(&self, sender: &NSPopUpButton) {
            let Some(tag) = sender.selectedItem().map(|i| i.tag() as usize) else { return };
            let layout = Layout::ALL[tag % Layout::ALL.len()];
            self.mutate(|e| e.config.layout = layout);
        }

        #[unsafe(method(themeChanged:))]
        fn theme_changed(&self, sender: &NSPopUpButton) {
            let Some(tag) = sender.selectedItem().map(|i| i.tag() as usize) else { return };
            self.mutate(|e| {
                e.config.theme = match tag {
                    0 => AUTOMATIC.to_owned(),
                    i => e.config.all_themes().get(i - 1).map_or(AUTOMATIC.to_owned(), |t| t.name.clone()),
                };
            });
        }

        #[unsafe(method(chooseBackground:))]
        fn choose_background(&self, _sender: &AnyObject) {
            self.pick_background();
        }

        #[unsafe(method(removeBackground:))]
        fn remove_background(&self, _sender: &AnyObject) {
            self.ivars().ui.borrow_mut().status = "Background photo removed".into();
            self.mutate(|e| e.config.background_image = None);
        }

        #[unsafe(method(dimChanged:))]
        fn dim_changed(&self, sender: &objc2_app_kit::NSSlider) {
            let dim = (sender.doubleValue().clamp(0.0, 0.7) * 100.0).round() / 100.0;
            self.mutate(|e| {
                if let Some(bg) = e.config.background_image.as_mut() {
                    bg.dim = dim;
                }
            });
        }

        #[unsafe(method(checkForUpdates:))]
        fn check_for_updates(&self, _sender: &AnyObject) {
            self.check_updates(true);
        }

        #[unsafe(method(installUpdate:))]
        fn install_update(&self, _sender: &AnyObject) {
            self.start_update();
        }

        #[unsafe(method(openWindow:))]
        fn open_window(&self, _sender: &AnyObject) {
            self.show_window();
        }

        #[unsafe(method(setRotation:))]
        fn set_rotation(&self, sender: &NSMenuItem) {
            let minutes = sender.tag() as u32;
            self.mutate(|e| e.config.rotation.every_minutes = minutes);
        }

        #[unsafe(method(setOrder:))]
        fn set_order(&self, sender: &NSMenuItem) {
            let order = if sender.tag() == 1 { Order::Shuffle } else { Order::Newest };
            self.mutate(|e| e.config.rotation.order = order);
        }

        #[unsafe(method(importFiles:))]
        fn import_files(&self, _sender: &AnyObject) {
            self.run_import(false);
        }

        #[unsafe(method(importStrava:))]
        fn import_strava(&self, _sender: &AnyObject) {
            self.run_import(true);
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &AnyObject) {
            let dir = self.dir();
            recap_macos::open(&crate::config::Config::path(&dir));
        }

        #[unsafe(method(showLibrary:))]
        fn show_library(&self, _sender: &AnyObject) {
            recap_macos::open(&self.dir());
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: &AnyObject) {
            let enable = !recap_macos::login_item_enabled();
            if let Err(e) = recap_macos::set_login_item(enable) {
                self.alert(
                    "Couldn't change Launch at Login",
                    &format!("{e}\n\nLaunch at Login works when RunReCap runs from RunReCap.app (see packaging/)."),
                );
            }
            self.rebuild_menu();
        }

        #[unsafe(method(togglePause:))]
        fn toggle_pause(&self, _sender: &AnyObject) {
            let Some(paused) = self.try_engine(|e| {
                e.state.paused = !e.state.paused;
                e.state.paused
            }) else {
                return;
            };
            if paused {
                self.restore_originals();
                self.sync_ui();
            } else {
                self.refresh();
            }
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: &AnyObject) {
            let paused = self.try_engine(|e| e.state.paused).unwrap_or(true);
            if !paused {
                self.restore_originals();
            }
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            self.on_tick();
        }

        #[unsafe(method(screensChanged:))]
        fn screens_changed(&self, _n: &NSNotification) {
            self.refresh();
        }

        #[unsafe(method(spaceChanged:))]
        fn space_changed(&self, _n: &NSNotification) {
            self.reapply();
        }

        #[unsafe(method(didWake:))]
        fn did_wake(&self, _n: &NSNotification) {
            self.reapply();
            self.on_tick();
        }

        #[unsafe(method(appearanceChanged:))]
        fn appearance_changed(&self, _n: &NSNotification) {
            let automatic = self.try_engine(|e| e.config.theme == AUTOMATIC).unwrap_or(false);
            if automatic {
                self.refresh();
            }
        }
    }

    // SAFETY: NSWindowDelegate has no safety requirements.
    unsafe impl NSWindowDelegate for Delegate {
        // SAFETY: matches `- (void)windowWillClose:(NSNotification *)n`.
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _n: &NSNotification) {
            // Back to a menu-bar-only app (no Dock icon) once the window is closed.
            NSApplication::sharedApplication(self.mtm())
                .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, engine: Engine) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            engine: Arc::new(Mutex::new(engine)),
            status_item: OnceCell::new(),
            window: OnceCell::new(),
            ui: RefCell::new(Ui::default()),
        });
        // SAFETY: NSObject's `init` has this signature.
        unsafe { msg_send![super(this), init] }
    }

    fn target(&self) -> &AnyObject {
        self.as_ref()
    }

    fn dir(&self) -> PathBuf {
        self.ivars()
            .engine
            .lock()
            .map(|e| e.dir.clone())
            .unwrap_or_default()
    }

    /// Run `f` on the engine without blocking the main thread. Beeps when an
    /// import is holding the engine.
    fn try_engine<R>(&self, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        match self.ivars().engine.try_lock() {
            Ok(mut e) => Some(f(&mut e)),
            Err(_) => {
                NSBeep();
                None
            }
        }
    }

    /// Change engine state from a menu action, persist it, and redraw.
    fn mutate(&self, f: impl FnOnce(&mut Engine)) {
        let saved = self.try_engine(|e| {
            f(e);
            e.save()
        });
        if let Some(Err(err)) = saved {
            tracing::warn!(%err, "saving settings failed");
        }
        self.remember_config_mtime();
        self.refresh();
    }

    fn setup(&self) {
        let mtm = self.mtm();
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        if let Some(button) = item.button(mtm) {
            let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str("figure.run"),
                Some(&NSString::from_str("RunReCap")),
            );
            match icon {
                Some(icon) => {
                    icon.setTemplate(true);
                    button.setImage(Some(&icon));
                }
                None => button.setTitle(&NSString::from_str("RunReCap")),
            }
        }
        let _ = self.ivars().status_item.set(item);

        // SAFETY: `self` outlives the observers (it lives for the whole process),
        // and each selector takes a single NSNotification argument.
        unsafe {
            let center = NSNotificationCenter::defaultCenter();
            center.addObserver_selector_name_object(
                self.target(),
                sel!(screensChanged:),
                Some(NSApplicationDidChangeScreenParametersNotification),
                None,
            );
            let ws = NSWorkspace::sharedWorkspace().notificationCenter();
            ws.addObserver_selector_name_object(
                self.target(),
                sel!(spaceChanged:),
                Some(NSWorkspaceActiveSpaceDidChangeNotification),
                None,
            );
            ws.addObserver_selector_name_object(
                self.target(),
                sel!(didWake:),
                Some(NSWorkspaceDidWakeNotification),
                None,
            );
            NSDistributedNotificationCenter::defaultCenter().addObserver_selector_name_object(
                self.target(),
                sel!(appearanceChanged:),
                Some(&NSString::from_str(
                    "AppleInterfaceThemeChangedNotification",
                )),
                None,
            );
            // Rotation, config-file changes and appearance fallback.
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                60.0,
                self.target(),
                sel!(tick:),
                None,
                true,
            );
        }

        self.remember_config_mtime();
        self.install_main_menu();
        self.show_window();
        // Dev aid (with RUNRECAP_SNAPSHOT): exercise the same path as a drop.
        if let Some(list) = std::env::var_os("RUNRECAP_DEV_IMPORT") {
            import_dropped(std::env::split_paths(&list).collect());
        }
        self.refresh();
        self.check_updates(false);
    }

    /// App menu used while the window is open (the app is a regular app then).
    fn install_main_menu(&self) {
        let mtm = self.mtm();
        let bar = NSMenu::new(mtm);
        let app_menu = NSMenu::new(mtm);
        app_menu.addItem(&self.item("Check for Updates…", Some(sel!(checkForUpdates:)), ""));
        app_menu.addItem(&NSMenuItem::separatorItem(mtm));
        app_menu.addItem(&self.item("Quit RunReCap", Some(sel!(quit:)), "q"));
        let file_menu = NSMenu::new(mtm);
        file_menu.setTitle(&NSString::from_str("File"));
        file_menu.addItem(&self.item("Import GPX Files…", Some(sel!(importFiles:)), "o"));
        file_menu.addItem(&self.item("Import Strava Export…", Some(sel!(importStrava:)), ""));
        file_menu.addItem(&NSMenuItem::separatorItem(mtm));
        // SAFETY: nil target sends `performClose:` to the key window.
        let close = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str("Close Window"),
                Some(sel!(performClose:)),
                &NSString::from_str("w"),
            )
        };
        file_menu.addItem(&close);
        for sub in [&app_menu, &file_menu] {
            let holder = NSMenuItem::new(mtm);
            holder.setSubmenu(Some(sub));
            bar.addItem(&holder);
        }
        NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
    }

    fn show_window(&self) {
        let mtm = self.mtm();
        let window = self.ivars().window.get_or_init(|| {
            let aspect = recap_macos::screens(mtm)
                .first()
                .map_or(0.625, |s| s.height_px as f64 / s.width_px.max(1) as f64);
            let w = MainWindow::new(mtm, self.target(), aspect);
            w.window.setDelegate(Some(ProtocolObject::from_ref(self)));
            w
        });
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        window.window.makeKeyAndOrderFront(None);
        self.activate();
        self.sync_ui();
    }

    fn config_mtime(&self) -> Option<SystemTime> {
        let path = crate::config::Config::path(&self.dir());
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    fn remember_config_mtime(&self) {
        let m = self.config_mtime();
        self.ivars().ui.borrow_mut().config_mtime = m;
    }

    fn on_tick(&self) {
        let mut changed = false;

        // Pick up hand edits to config.toml.
        let mtime = self.config_mtime();
        if mtime != self.ivars().ui.borrow().config_mtime {
            self.ivars().ui.borrow_mut().config_mtime = mtime;
            match self.try_engine(|e| e.reload_config()) {
                Some(Err(err)) => {
                    self.ivars().ui.borrow_mut().warnings = vec![format!("config.toml: {err:#}")]
                }
                Some(Ok(())) => changed = true,
                None => {}
            }
        }

        if self
            .try_engine(|e| !e.state.paused && e.rotate_if_due())
            .unwrap_or(false)
        {
            let _ = self.try_engine(|e| e.state.save(&e.dir));
            changed = true;
        }

        // Fallback in case the appearance notification was missed.
        let dark = recap_macos::system_is_dark();
        if crate::engine::now() - self.ivars().ui.borrow().last_update_check > 24 * 3600 {
            self.check_updates(false);
        }

        if self.ivars().ui.borrow().last_dark != Some(dark) {
            changed = true;
        }

        if changed {
            self.refresh();
        }
    }

    /// Re-render the current run for every screen (on a worker thread).
    fn refresh(&self) {
        {
            let mut ui = self.ivars().ui.borrow_mut();
            if ui.rendering || ui.importing {
                ui.dirty = true;
                return;
            }
        }
        let dark = recap_macos::system_is_dark();
        let job = self.try_engine(|e| {
            e.render_job(dark, recap_macos::utc_offset_s())
                .map(|j| (j, !e.state.paused))
        });
        let Some(Some((job, apply))) = job else {
            self.ivars().ui.borrow_mut().preview = None;
            self.sync_ui();
            self.write_snapshot_if_requested();
            return;
        };
        let screens = recap_macos::screens(self.mtm());
        let preview = self.preview_canvas(&screens);
        let renders = self.dir().join("renders");
        {
            let mut ui = self.ivars().ui.borrow_mut();
            ui.rendering = true;
            ui.last_dark = Some(dark);
        }
        std::thread::spawn(move || {
            let out = render_all(&job, &screens, apply, preview, &renders);
            DispatchQueue::main().exec_async(move || with_delegate(|d| d.finish_render(out)));
        });
    }

    fn finish_render(&self, out: RenderOutput) {
        let mtm = self.mtm();
        let mut warnings = out.warnings;
        let renders = self.dir().join("renders");

        // Remember each screen's own wallpaper before the first replacement.
        let _ = self.try_engine(|e| {
            for (screen, _) in &out.files {
                if !e.state.originals.contains_key(screen) {
                    if let Some(orig) =
                        recap_macos::wallpaper(mtm, screen).filter(|p| !p.starts_with(&renders))
                    {
                        e.state
                            .originals
                            .insert(screen.clone(), orig.to_string_lossy().into_owned());
                    }
                }
            }
            e.state.save(&e.dir)
        });

        for (screen, file) in &out.files {
            if let Err(e) = recap_macos::set_wallpaper(mtm, screen, file) {
                warnings.push(format!("Couldn't set wallpaper: {e}"));
            }
            cleanup_old_renders(&renders, screen, file);
        }

        let again = {
            let mut ui = self.ivars().ui.borrow_mut();
            ui.rendering = false;
            if !out.files.is_empty() {
                ui.applied = out.files;
            }
            ui.thumb = out.thumb;
            ui.preview = out.preview;
            ui.warnings = warnings;
            std::mem::take(&mut ui.dirty)
        };
        self.sync_ui();
        if again {
            self.refresh();
        } else {
            self.write_snapshot_if_requested();
        }
    }

    // ---- Background photo -------------------------------------------------

    fn recommended_size(&self) -> (u32, u32) {
        background::recommended_size(&recap_macos::screens(self.mtm()))
    }

    fn pick_background(&self) {
        let mtm = self.mtm();
        self.activate();
        let panel = objc2_app_kit::NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        panel.setPrompt(Some(&NSString::from_str("Use Photo")));
        let (w, h) = self.recommended_size();
        panel.setMessage(Some(&NSString::from_str(&format!(
            "Choose a landscape photo (JPEG, PNG, HEIC), ideally {w} × {h} px or larger. It's cropped to fill the screen and your runs are drawn on top."
        ))));
        if panel.runModal() != MODAL_OK {
            return;
        }
        if let Some(path) = panel.URLs().iter().find_map(|u| u.to_file_path()) {
            self.set_background(path);
        }
    }

    /// Convert and apply a photo (worker thread for the decode/resize).
    fn set_background(&self, path: PathBuf) {
        if !background::is_image(&path) {
            self.alert(
                "That file isn't a supported photo",
                "Use a JPEG, PNG, HEIC, TIFF or WebP image.",
            );
            return;
        }
        let rec = self.recommended_size();
        let dim = self
            .try_engine(|e| e.config.background_image.as_ref().map(|b| b.dim))
            .flatten()
            .unwrap_or(background::DEFAULT_DIM);
        let dir = self.dir();
        {
            let mut ui = self.ivars().ui.borrow_mut();
            ui.status = "Preparing photo…".into();
            ui.background_busy = true;
        }
        self.sync_ui();
        std::thread::spawn(move || {
            let result = background::import(&dir, &path, rec, dim).map_err(|e| format!("{e:#}"));
            DispatchQueue::main()
                .exec_async(move || with_delegate(|d| d.finish_background(result, rec)));
        });
    }

    fn finish_background(
        &self,
        result: Result<background::BackgroundImage, String>,
        rec: (u32, u32),
    ) {
        self.ivars().ui.borrow_mut().background_busy = false;
        match result {
            Ok(bg) => {
                let status = match background::low_res_warning(&bg, rec) {
                    Some(w) => format!("⚠︎ {w}"),
                    None => format!("Background set: {}", bg.name),
                };
                self.ivars().ui.borrow_mut().status = status;
                self.mutate(|e| e.config.background_image = Some(bg));
            }
            Err(e) => {
                self.ivars().ui.borrow_mut().status.clear();
                self.sync_ui();
                self.alert("Couldn't use that photo", &e);
            }
        }
    }

    // ---- Updates ----------------------------------------------------------

    /// Check the release feed on a worker thread. `manual` = user asked, so
    /// always report the outcome.
    fn check_updates(&self, manual: bool) {
        let feed = self.try_engine(|e| e.config.update_feed()).flatten();
        self.ivars().ui.borrow_mut().last_update_check = crate::engine::now();
        let Some(feed) = feed else {
            self.ivars().ui.borrow_mut().update_checked = true;
            if manual {
                self.alert(
                    "Updates aren't set up",
                    "This build has no update feed. Set update_url in config.toml, or build with RUNRECAP_UPDATE_URL.",
                );
            }
            self.write_snapshot_if_requested();
            return;
        };
        std::thread::spawn(move || {
            let result = update::check(&feed).map_err(|e| format!("{e:#}"));
            DispatchQueue::main()
                .exec_async(move || with_delegate(|d| d.finish_check(result, manual)));
        });
    }

    fn finish_check(&self, result: Result<Option<Release>, String>, manual: bool) {
        self.ivars().ui.borrow_mut().update_checked = true;
        match result {
            Ok(Some(release)) => {
                tracing::info!(version = %release.version, "update available");
                self.ivars().ui.borrow_mut().update = Some(release);
                self.sync_ui();
                if manual {
                    self.start_update();
                }
            }
            Ok(None) => {
                self.ivars().ui.borrow_mut().update = None;
                self.sync_ui();
                if manual {
                    self.alert(
                        "You're up to date",
                        &format!("RunReCap {} is the latest version.", update::VERSION),
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "update check failed");
                if manual {
                    self.alert("Couldn't check for updates", &e);
                }
            }
        }
        self.write_snapshot_if_requested();
    }

    /// Confirm, then download + verify on a worker thread.
    fn start_update(&self) {
        let (release, busy) = {
            let ui = self.ivars().ui.borrow();
            (ui.update.clone(), ui.update_busy)
        };
        let Some(release) = release else { return };
        if busy {
            return;
        }
        let Some(app) = recap_macos::app_bundle_path() else {
            self.alert(
                "Can't update this copy",
                &format!(
                    "Updates install into RunReCap.app, but this is a development build.\n\nDownload {} manually: {}",
                    release.version, release.url
                ),
            );
            return;
        };

        self.activate();
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(&format!(
            "RunReCap {} is available",
            release.version
        )));
        let mut info = format!("You have {}.", update::VERSION);
        if let Some(notes) = release.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            info.push_str(&format!("\n\n{notes}"));
        }
        info.push_str("\n\nRunReCap will download the update, reinstall itself and reopen.");
        alert.setInformativeText(&NSString::from_str(&info));
        alert.addButtonWithTitle(&NSString::from_str("Update & Restart"));
        alert.addButtonWithTitle(&NSString::from_str("Later"));
        if alert.runModal() != objc2_app_kit::NSAlertFirstButtonReturn {
            return;
        }

        {
            let mut ui = self.ivars().ui.borrow_mut();
            ui.update_busy = true;
            ui.status = format!("Downloading RunReCap {}…", release.version);
        }
        self.sync_ui();
        std::thread::spawn(move || {
            let result = update::prepare(&release, &app).map_err(|e| format!("{e:#}"));
            DispatchQueue::main()
                .exec_async(move || with_delegate(|d| d.finish_update(result, release)));
        });
    }

    fn finish_update(&self, result: Result<update::Prepared, String>, release: Release) {
        let outcome = result.and_then(|p| p.launch_swap(true).map_err(|e| format!("{e:#}")));
        match outcome {
            Ok(()) => {
                tracing::info!(version = %release.version, "installing update and restarting");
                let _ = self.try_engine(|e| e.save());
                // Keep the wallpaper as is; the new version picks up where we left off.
                NSApplication::sharedApplication(self.mtm()).terminate(None);
            }
            Err(e) => {
                {
                    let mut ui = self.ivars().ui.borrow_mut();
                    ui.update_busy = false;
                    ui.status.clear();
                }
                self.sync_ui();
                self.alert(
                    "Update failed",
                    &format!(
                        "{e}\n\nYou can download RunReCap {} manually:\n{}",
                        release.version, release.url
                    ),
                );
            }
        }
    }

    /// Dev aid: `RUNRECAP_SNAPSHOT=out.png runrecap` writes the window to a PNG
    /// after the first render, then quits (no Screen Recording permission needed).
    fn write_snapshot_if_requested(&self) {
        let Some(path) = std::env::var_os("RUNRECAP_SNAPSHOT") else {
            return;
        };
        {
            let ui = self.ivars().ui.borrow();
            if ui.rendering || ui.importing || ui.background_busy || !ui.update_checked {
                return;
            }
        }
        let dark = recap_macos::system_is_dark();
        if let Some(png) = self
            .ivars()
            .window
            .get()
            .and_then(|w| w.snapshot_png())
            .and_then(|p| flatten(&p, dark))
        {
            if let Err(e) = std::fs::write(&path, png) {
                tracing::warn!(error = %e, "snapshot failed");
            }
        }
        NSApplication::sharedApplication(self.mtm()).terminate(None);
    }

    /// The window preview: the main screen's shape at 2× the preview's points.
    fn preview_canvas(&self, screens: &[ScreenInfo]) -> Option<Canvas> {
        let size = self.ivars().window.get()?.preview_size;
        let main = screens.first()?;
        let width = (size.width * 2.0).round() as u32;
        let k = width as f32 / main.width_px.max(1) as f32;
        Some(Canvas {
            width,
            height: (main.height_px as f32 * k).round() as u32,
            safe: Insets {
                top: main.safe_px[0] * k,
                right: main.safe_px[1] * k,
                bottom: main.safe_px[2] * k,
                left: main.safe_px[3] * k,
            },
        })
    }

    /// Re-apply the last images (macOS keeps wallpapers per Space).
    fn reapply(&self) {
        if self.try_engine(|e| e.state.paused).unwrap_or(true) {
            return;
        }
        let applied = self.ivars().ui.borrow().applied.clone();
        for (screen, file) in applied {
            if file.exists() {
                let _ = recap_macos::set_wallpaper(self.mtm(), &screen, &file);
            }
        }
    }

    fn restore_originals(&self) {
        let mtm = self.mtm();
        let originals = self
            .try_engine(|e| std::mem::take(&mut e.state.originals))
            .unwrap_or_default();
        for (screen, path) in &originals {
            let path = Path::new(path);
            if path.exists() {
                if let Err(e) = recap_macos::set_wallpaper(mtm, screen, path) {
                    tracing::warn!(screen, error = %e, "restoring wallpaper failed");
                }
            }
        }
        let _ = self.try_engine(|e| e.state.save(&e.dir));
    }

    fn activate(&self) {
        #[allow(deprecated)]
        NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
    }

    fn alert(&self, title: &str, body: &str) {
        self.activate();
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(body));
        alert.runModal();
    }

    fn run_import(&self, strava: bool) {
        let mtm = self.mtm();
        if self.ivars().ui.borrow().importing {
            NSBeep();
            return;
        }
        self.activate();
        let panel = objc2_app_kit::NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(!strava);
        panel.setPrompt(Some(&NSString::from_str("Import")));
        panel.setMessage(Some(&NSString::from_str(if strava {
            "Choose your Strava export (.zip, or the unzipped folder)."
        } else {
            "Choose .gpx / .gpx.gz files or folders containing them."
        })));
        if panel.runModal() != MODAL_OK {
            return;
        }
        let paths: Vec<PathBuf> = panel
            .URLs()
            .iter()
            .filter_map(|u| u.to_file_path())
            .collect();
        self.import_paths(paths);
    }

    fn import_paths(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        {
            let mut ui = self.ivars().ui.borrow_mut();
            if ui.importing {
                drop(ui);
                NSBeep();
                return;
            }
            ui.importing = true;
            ui.status = "Importing…".into();
        }
        self.sync_ui();
        let engine = Arc::clone(&self.ivars().engine);
        std::thread::spawn(move || {
            let report = match engine.lock() {
                Ok(mut e) => {
                    let r = e.import(&paths);
                    if let Err(err) = e.state.save(&e.dir) {
                        tracing::warn!(%err, "saving state failed");
                    }
                    r
                }
                Err(_) => ImportReport {
                    failed: vec![("library".into(), "engine unavailable".into())],
                    ..Default::default()
                },
            };
            DispatchQueue::main().exec_async(move || with_delegate(|d| d.finish_import(report)));
        });
    }

    fn finish_import(&self, report: ImportReport) {
        {
            let mut ui = self.ivars().ui.borrow_mut();
            ui.importing = false;
            ui.status = report.summary();
        }
        let mut body = report.summary();
        for (src, err) in report.failed.iter().take(5) {
            let name = Path::new(src)
                .file_name()
                .map_or(src.clone(), |n| n.to_string_lossy().into_owned());
            body.push_str(&format!("\n• {name}: {err}"));
        }
        if report.failed.len() > 5 {
            body.push_str(&format!("\n…and {} more", report.failed.len() - 5));
        }
        self.ivars().ui.borrow_mut().dirty = false;
        self.refresh();
        if !report.failed.is_empty() {
            self.alert("Some files couldn't be imported", &body);
        }
    }

    /// Update the status-bar menu and the window from current state.
    fn sync_ui(&self) {
        self.rebuild_menu();
        self.sync_window();
    }

    fn sync_window(&self) {
        let Some(window) = self.ivars().window.get() else {
            return;
        };
        let ui = self.ivars().ui.borrow();
        let Ok(engine) = self.ivars().engine.try_lock() else {
            // An import holds the engine; show progress only.
            let vm = ViewModel {
                title: String::new(),
                subtitle: String::new(),
                status: ui.status.clone(),
                has_runs: ui.preview.is_some(),
                busy: true,
                layout_index: 0,
                layouts: Layout::ALL.iter().map(|l| l.label()).collect(),
                themes: Vec::new(),
                theme_index: 0,
                active: false,
                preview_png: ui.preview.as_deref(),
                version: format!("Version {}", update::VERSION),
                update_title: None,
                update_enabled: false,
                background_hint: String::new(),
                has_background: false,
                dim: background::DEFAULT_DIM,
            };
            window.update(&vm);
            return;
        };
        let units = engine.config.units;
        let (title, subtitle) = match (engine.config.layout, engine.current()) {
            (_, None) => ("No runs yet".to_owned(), String::new()),
            (Layout::Single, Some(run)) => {
                let mut parts = vec![caption::distance(run.distance_m, units)];
                parts.extend(run.duration_s.map(caption::duration));
                parts.extend(run.pace_s_per_km().map(|p| caption::pace(p, units)));
                parts.extend(
                    run.start
                        .map(|t| caption::format_date(t, recap_macos::utc_offset_s())),
                );
                (run.name.clone(), parts.join("  ·  "))
            }
            (layout, Some(_)) => {
                let runs = engine.eligible();
                let km: f64 = runs.iter().map(|a| a.distance_m).sum();
                (
                    format!("{} · {} runs", layout.label(), runs.len()),
                    caption::distance(km, units),
                )
            }
        };
        let total = engine.library.activities().len();
        let mut status = if ui.status.is_empty() {
            format!("{total} activities in library")
        } else {
            ui.status.clone()
        };
        if let Some(w) = ui.warnings.first() {
            status = format!("⚠︎ {w}");
        }
        let rec = background::recommended_size(&recap_macos::screens(self.mtm()));
        let background_hint = match &engine.config.background_image {
            Some(bg) => {
                let mut hint = format!("{} · {} × {} px", bg.name, bg.width, bg.height);
                if background::low_res_warning(bg, rec).is_some() {
                    hint.push_str(&format!(" · ⚠︎ low resolution (use {} × {}+)", rec.0, rec.1));
                }
                hint
            }
            None => background::recommendation_text(rec),
        };
        let themes = engine.config.all_themes();
        let theme_index = if engine.config.theme == AUTOMATIC {
            0
        } else {
            themes
                .iter()
                .position(|t| t.name == engine.config.theme)
                .map_or(0, |i| i + 1)
        };
        let vm = ViewModel {
            title,
            subtitle,
            status,
            has_runs: engine.current().is_some(),
            busy: ui.importing,
            layout_index: Layout::ALL
                .iter()
                .position(|l| *l == engine.config.layout)
                .unwrap_or(0),
            layouts: Layout::ALL.iter().map(|l| l.label()).collect(),
            themes: std::iter::once("Automatic (Light/Dark)".to_owned())
                .chain(themes.iter().map(|t| t.name.clone()))
                .collect(),
            theme_index,
            active: !engine.state.paused,
            preview_png: ui.preview.as_deref(),
            version: format!("Version {}", update::VERSION),
            update_title: ui.update.as_ref().map(|r| {
                if ui.update_busy {
                    "Updating…".to_owned()
                } else {
                    format!("Update to {}", r.version)
                }
            }),
            update_enabled: !ui.update_busy,
            background_hint,
            has_background: engine.config.background_image.is_some(),
            dim: engine
                .config
                .background_image
                .as_ref()
                .map_or(background::DEFAULT_DIM, |b| b.dim),
        };
        drop(engine);
        window.update(&vm);
    }

    // ---- Menu -------------------------------------------------------------

    fn item(&self, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
        let mtm = self.mtm();
        // SAFETY: the action selectors are implemented by `self`, the target.
        unsafe {
            let it = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                action,
                &NSString::from_str(key),
            );
            if action.is_some() {
                it.setTarget(Some(self.target()));
            } else {
                it.setEnabled(false);
            }
            it
        }
    }

    fn check(item: &NSMenuItem, on: bool) {
        item.setState(if on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    fn rebuild_menu(&self) {
        let mtm = self.mtm();
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        let ui = self.ivars().ui.borrow();

        let Ok(engine) = self.ivars().engine.try_lock() else {
            menu.addItem(&self.item("Importing runs…", None, ""));
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            menu.addItem(&self.item("Quit RunReCap", Some(sel!(quit:)), "q"));
            self.set_menu(&menu);
            return;
        };

        // Header: the current run.
        if ui.importing {
            menu.addItem(&self.item("Importing runs…", None, ""));
        } else if let Some(run) = engine.current() {
            let units = engine.config.units;
            let header = self.item(&run.name, None, "");
            if let Some(img) = ui.thumb.as_deref().and_then(thumb_image) {
                header.setImage(Some(&img));
            }
            menu.addItem(&header);
            let mut stats = vec![caption::distance(run.distance_m, units)];
            stats.extend(run.duration_s.map(caption::duration));
            stats.extend(run.pace_s_per_km().map(|p| caption::pace(p, units)));
            menu.addItem(&self.item(&stats.join(" · "), None, ""));
            if let Some(t) = run.start {
                menu.addItem(&self.item(
                    &caption::format_date(t, recap_macos::utc_offset_s()),
                    None,
                    "",
                ));
            }
        } else {
            menu.addItem(&self.item("No runs yet — import to get started", None, ""));
        }
        for w in &ui.warnings {
            menu.addItem(&self.item(&format!("⚠︎ {w}"), None, ""));
        }
        if engine.state.paused {
            menu.addItem(&self.item("Preview only — your own wallpaper is shown", None, ""));
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&self.item("Open RunReCap", Some(sel!(openWindow:)), "0"));
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let has_runs = engine.current().is_some();
        for (title, action, key) in [
            ("Next Run", sel!(next:), "]"),
            ("Previous Run", sel!(previous:), "["),
            ("Shuffle", sel!(shuffle:), "s"),
        ] {
            let it = self.item(title, Some(action), key);
            it.setEnabled(has_runs);
            menu.addItem(&it);
        }
        let pin = self.item("Pin This Run", Some(sel!(togglePin:)), "p");
        Self::check(&pin, engine.state.pinned);
        menu.addItem(&pin);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        // Layout ▸
        let layouts = NSMenu::new(mtm);
        for (i, l) in Layout::ALL.iter().enumerate() {
            let it = self.item(l.label(), Some(sel!(setLayout:)), "");
            it.setTag(i as isize);
            Self::check(&it, engine.config.layout == *l);
            layouts.addItem(&it);
        }
        self.submenu(&menu, "Layout", &layouts);

        // Theme ▸
        let themes = NSMenu::new(mtm);
        let auto = self.item("Automatic (Light/Dark)", Some(sel!(setTheme:)), "");
        auto.setTag(0);
        Self::check(&auto, engine.config.theme == AUTOMATIC);
        themes.addItem(&auto);
        themes.addItem(&NSMenuItem::separatorItem(mtm));
        for (i, t) in engine.config.all_themes().iter().enumerate() {
            let it = self.item(&t.name, Some(sel!(setTheme:)), "");
            it.setTag(i as isize + 1);
            Self::check(&it, engine.config.theme == t.name);
            themes.addItem(&it);
        }
        self.submenu(&menu, "Theme", &themes);

        // Change Wallpaper ▸
        let rot = NSMenu::new(mtm);
        for (minutes, label) in ROTATION_CHOICES {
            let it = self.item(label, Some(sel!(setRotation:)), "");
            it.setTag(minutes as isize);
            Self::check(&it, engine.config.rotation.every_minutes == minutes);
            rot.addItem(&it);
        }
        rot.addItem(&NSMenuItem::separatorItem(mtm));
        for (tag, label, order) in [
            (0, "Newest First", Order::Newest),
            (1, "Shuffle", Order::Shuffle),
        ] {
            let it = self.item(label, Some(sel!(setOrder:)), "");
            it.setTag(tag);
            Self::check(&it, engine.config.rotation.order == order);
            rot.addItem(&it);
        }
        self.submenu(&menu, "Change Wallpaper", &rot);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let busy = ui.importing;
        let import_files = self.item("Import GPX Files…", Some(sel!(importFiles:)), "i");
        import_files.setEnabled(!busy);
        menu.addItem(&import_files);
        let import_strava = self.item("Import Strava Export…", Some(sel!(importStrava:)), "");
        import_strava.setEnabled(!busy);
        menu.addItem(&import_strava);
        menu.addItem(&self.item("Open Settings File…", Some(sel!(openSettings:)), ","));
        menu.addItem(&self.item("Show Data Folder", Some(sel!(showLibrary:)), ""));
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        if let Some(release) = &ui.update {
            let it = self.item(
                &format!("Update to {}…", release.version),
                Some(sel!(installUpdate:)),
                "",
            );
            it.setEnabled(!ui.update_busy);
            menu.addItem(&it);
        }
        menu.addItem(&self.item("Check for Updates…", Some(sel!(checkForUpdates:)), ""));
        menu.addItem(&self.item(&format!("RunReCap {}", update::VERSION), None, ""));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let login = self.item("Launch at Login", Some(sel!(toggleLogin:)), "");
        Self::check(&login, recap_macos::login_item_enabled());
        menu.addItem(&login);
        let pause_title = if engine.state.paused {
            "Set as Wallpaper"
        } else {
            "Restore My Wallpaper"
        };
        menu.addItem(&self.item(pause_title, Some(sel!(togglePause:)), ""));
        menu.addItem(&self.item("Quit RunReCap", Some(sel!(quit:)), "q"));

        drop(engine);
        drop(ui);
        self.set_menu(&menu);
    }

    fn submenu(&self, parent: &NSMenu, title: &str, sub: &NSMenu) {
        let it = self.item(title, None, "");
        it.setEnabled(true);
        it.setSubmenu(Some(sub));
        parent.addItem(&it);
    }

    fn set_menu(&self, menu: &NSMenu) {
        if let Some(item) = self.ivars().status_item.get() {
            item.setMenu(Some(menu));
        }
    }
}

fn thumb_image(png: &[u8]) -> Option<Retained<NSImage>> {
    let img = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png))?;
    img.setSize(NSSize::new(THUMB_W as f64 / 2.0, THUMB_H as f64 / 2.0));
    Some(img)
}

const THUMB_W: u32 = 96;
const THUMB_H: u32 = 60;

/// Worker thread: the window preview, a menu thumbnail, and (when `apply`)
/// one PNG per screen.
fn render_all(
    job: &RenderJob,
    screens: &[ScreenInfo],
    apply: bool,
    preview: Option<Canvas>,
    dir: &Path,
) -> RenderOutput {
    static FONTS: Mutex<Option<Fonts>> = Mutex::new(None);
    let mut guard = FONTS.lock().unwrap_or_else(|p| p.into_inner());
    let fonts = guard.get_or_insert_with(Fonts::new);

    let mut out = RenderOutput {
        files: Vec::new(),
        thumb: None,
        preview: None,
        warnings: Vec::new(),
    };
    if let Some(canvas) = preview {
        match job.render(canvas, fonts, true) {
            Ok((pixmap, notes)) => {
                out.preview = pixmap.encode_png().ok();
                out.warnings.extend(notes.warnings);
            }
            Err(e) => out.warnings.push(format!("Preview failed: {e:#}")),
        }
    }
    let screens = if apply { screens } else { &[] };
    if let Err(e) = std::fs::create_dir_all(dir) {
        out.warnings
            .push(format!("Can't create {}: {e}", dir.display()));
        return out;
    }
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    for s in screens {
        let canvas = Canvas {
            width: s.width_px,
            height: s.height_px,
            safe: Insets {
                top: s.safe_px[0],
                right: s.safe_px[1],
                bottom: s.safe_px[2],
                left: s.safe_px[3],
            },
        };
        match job.render(canvas, fonts, true) {
            Ok((pixmap, notes)) => {
                for w in notes.warnings {
                    if !out.warnings.contains(&w) {
                        out.warnings.push(w);
                    }
                }
                // A new file name each time: macOS caches desktop pictures by path.
                let file = dir.join(format!("{}-{stamp}.png", s.id));
                match pixmap.save_png(&file) {
                    Ok(()) => out.files.push((s.id.clone(), file)),
                    Err(e) => out.warnings.push(format!("Couldn't write wallpaper: {e}")),
                }
            }
            Err(e) => out.warnings.push(format!("Render failed: {e:#}")),
        }
    }
    let mut thumb_job = job.clone();
    thumb_job.theme.route.glow = None;
    thumb_job.theme.route.markers = false;
    thumb_job.theme.route.width_pct = 3.0;
    if let Ok((pixmap, _)) = thumb_job.render(Canvas::new(THUMB_W, THUMB_H), fonts, false) {
        out.thumb = pixmap.encode_png().ok();
    }
    out
}

fn cleanup_old_renders(dir: &Path, screen: &str, keep: &Path) {
    let prefix = format!("{screen}-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        if p != keep && name.to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Composite a transparent window snapshot onto the window background color.
fn flatten(png: &[u8], dark: bool) -> Option<Vec<u8>> {
    let src = tiny_skia::Pixmap::decode_png(png).ok()?;
    let mut out = tiny_skia::Pixmap::new(src.width(), src.height())?;
    let (bg, v) = if dark { (0x1e, 0x1e) } else { (0xec, 0xec) };
    out.fill(tiny_skia::Color::from_rgba8(bg, v, v, 255));
    out.draw_pixmap(
        0,
        0,
        src.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        tiny_skia::Transform::identity(),
        None,
    );
    out.encode_png().ok()
}

/// Files dropped on the window's preview.
/// Files dropped on the window's preview: a photo becomes the background,
/// GPX files / Strava exports are imported.
pub fn import_dropped(paths: Vec<PathBuf>) {
    let (images, runs): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| background::is_image(p));
    with_delegate(|d| {
        if let Some(photo) = images.into_iter().next() {
            d.set_background(photo);
        }
        d.import_paths(runs);
    });
}

pub fn run(dir: &Path) -> Result<()> {
    let mtm = MainThreadMarker::new().context("RunReCap must start on the main thread")?;
    let engine = Engine::open(dir)?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate = Delegate::new(mtm, engine);
    DELEGATE.with(|d| {
        let _ = d.set(delegate.clone());
    });
    delegate.setup();
    tracing::info!(dir = %dir.display(), "RunReCap started");
    app.run();
    Ok(())
}
