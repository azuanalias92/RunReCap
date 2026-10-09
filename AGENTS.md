# AGENTS.md — RunReCap

Guidance for AI coding agents (and humans) working on this repository.
Repo: https://github.com/azuanalias92/RunReCap

> Status: **v0.1.0, v1 in progress.** M0–M3 work, and M4 is partly done (see §10 for what's left). Sections marked _(TBD)_ are open decisions.

### Decisions (2026-10-09)
- **Distribution:** direct download (DMG from GitHub Releases), with no App Store sandbox, so plain file paths are fine. The goal is Developer ID signing + notarization. Builds are ad-hoc signed until there's a certificate.
- **v1 scope:** **Static mode only.** Live/animated mode is on the roadmap (§10), not in v1.
- **Data input:** **manual upload** only (drop on the window or use "Import…"). There is no API sync in v1.
- **Primary source:** **Strava**. The Strava bulk export (`.zip` with `activities.csv` and `.gpx`/`.gpx.gz`) is the first-class import path.
- **Preview first:** the app opens a window with a live preview. The desktop is only changed after "Set as Wallpaper".
- **Background photo:** the user can upload a photo, and the route is drawn on top of it (§5.2).
- **Updates:** a built-in self-updater reads a `latest.json` feed from GitHub Releases (§6).
- **All Rust:** no Swift, SwiftUI, or Obj-C source files. AppKit is reached only through `objc2-*` crates, and the UI is built in Rust with AppKit.
- **Minimum macOS:** a rolling policy covering **only macOS versions released in the last 5 years** (§6.1).
- **Customization (v1):** the user chooses the **font** for the text on the wallpaper and the **color** of the GPX route (§5.1).
- **Roadmap headline (public, on the landing page):** next is **Strava integration** (account sync), later a **Windows version**, "and so much more".
- **Design system:** [`Design.md`](Design.md) (Apple style) is the visual source of truth. §6.2 explains how it maps onto a native app.

## 1. What this project is

RunReCap is a macOS desktop-wallpaper app, in the spirit of Wallpaper Engine but much narrower:
the wallpaper only ever shows **drawings of the user's past runs, rendered from GPX tracks**, optionally on top of a photo the user uploads.

- Platform: macOS 13+ only, universal binary (Apple Silicon + Intel), MacBook and desktop Macs, multi-monitor.
- Language: **Rust only**. No Swift or Obj-C sources in the repo.
- Form factor: a **main window** (preview, import, settings), plus a **menu-bar item** that stays when the window is closed. The Dock icon shows only while the window is open.

### Non-goals
- Not a general-purpose wallpaper engine (no video, web, or arbitrary scenes).
- Not a training/analytics app. Stats are decoration, not the product.
- No Linux support. **Windows is on the roadmap** (§10). Keep `recap-core` / `recap-render` platform-independent so a Windows shell can reuse them.

## 2. Why Rust (not Go)

| Concern | Rust | Go |
|---|---|---|
| AppKit / Cocoa bindings | `objc2`, `objc2-app-kit`, `objc2-foundation` — typed, maintained | cgo + hand-written Obj-C shims |
| GPU rendering (Live mode, later) | `wgpu` (Metal backend) or `objc2-metal` directly | No mature Metal path |
| 2D vector rendering | `tiny-skia` | Limited |
| Idle footprint (runs 24/7) | No GC, small RSS | GC pauses, higher baseline |

## 3. How the wallpaper is shown

There are two display modes. **v1 ships Static only.** Live mode is on the roadmap.

### 3.1 Static mode
1. Nothing touches the desktop until the user clicks **Set as Wallpaper** (`state.paused = false`). Before that, only the window preview is rendered.
2. Render one image per screen at that screen's native pixel size (`NSScreen.backingScaleFactor`), keeping clear of the menu bar, notch and Dock (`visibleFrame` + `safeAreaInsets`).
3. Write it to `renders/<screen-id>-<millis>.png`, with a new name each time because macOS caches desktop pictures by path. Old renders are deleted.
4. Apply with `NSWorkspace.setDesktopImageURL(_:forScreen:options:)`. The user's original wallpaper per screen is saved in `state.json` first, and restored by "Restore My Wallpaper" and on Quit (not on an update restart).
5. Re-render only when inputs change: an import, a config change, a screen change, the appearance changing, or a scheduled rotation.

CPU/GPU cost between renders is zero. Rendering happens on a worker thread.

### 3.2 Live mode (animated) — _roadmap, not v1_
Do not implement this until the roadmap reaches it. Keep the render code structured so that it can be added later.
- One borderless `NSWindow` per `NSScreen`, at window level `CGWindowLevelForKey(kCGDesktopWindowLevelKey)`.
- `collectionBehavior = canJoinAllSpaces | stationary | ignoresCycle`, `ignoresMouseEvents = true`.
- A Metal-backed layer (`CAMetalLayer` via `wgpu`) draws the animation, for example a route being traced in, or a pulsing heatmap.
- **Must pause** (stop the render loop completely, not just throttle) when:
  - the window is occluded (`NSWindow.occlusionState`), e.g. by a full-screen app,
  - the display is asleep or the screen is locked,
  - the Mac is on battery and the user has enabled "static on battery",
  - Low Power Mode is on (`ProcessInfo.isLowPowerModeEnabled`).
- Target 30 fps maximum. The frame budget is tiny; this is a background app.

### 3.3 Screen / Space / system events (implemented in `app.rs`)
- `NSApplicationDidChangeScreenParametersNotification` → re-render.
- `NSWorkspaceActiveSpaceDidChangeNotification` → re-apply the last images (macOS keeps wallpapers per Space).
- `NSWorkspaceDidWakeNotification` → re-apply, then run the tick.
- `AppleInterfaceThemeChangedNotification` (distributed) → re-render when the theme is "Automatic".
- A 60 s `NSTimer` tick handles rotation, `config.toml` edits, the appearance fallback, and the daily update check.

## 4. Data: where runs come from

v1: **manual upload, Strava first** (local only; the only network use is the update check):
- **Import files**: drop `.gpx` / `.gpx.gz` files (or folders) on the window preview, or use "Import GPX Files…" (`NSOpenPanel`, multi-select).
- **Import Strava export**: drop the Strava bulk-export `.zip` (or its unzipped folder), or use "Import Strava Export…". `activities.csv` supplies the name and type, and `activities/*.gpx(.gz)` supplies the routes.
- Imported files are **copied** into the library (keyed by content hash), so the originals can be deleted. Duplicate imports are skipped.
- `.fit` / `.tcx` files are skipped and counted in the import summary.
- The GPX reader is our own streaming `quick-xml` parser (`recap-core/src/gpx.rs`). It reads `trkpt` lat/lon, `ele`, `time` and the Garmin/Strava `hr` extension, plus the track `name` and `type` (Strava uses numeric types; 9 = run).

Roadmap sources (§10): watched folders, Apple Health export, `.fit`/`.tcx`, the Strava API, Garmin Connect, and HealthKit.

### Filtering
- Activity type: runs only by default (`filter.runs_only`). The type comes from the CSV or GPX `<type>`, with an average-speed heuristic as fallback.
- Minimum distance (`filter.min_distance_km`). The `Filter` type also supports a date range, which has no UI yet.
- **Privacy**: `privacy_trim_m` (default 200 m) trims the first and last metres of every route so a home address isn't visible on screen shares.

### Library and data dir
Parsed tracks are stored in `library/index.json` (serde_json), with the imported GPX copies in `library/tracks/<id>.gpx`. `<id>` is a SHA-256 prefix of the file content, which is also how duplicates are detected.
Each record holds the simplified polyline (RDP, 1.5 m tolerance), distance, duration, date, and elevation gain. GPX files are never re-parsed.
Switch to SQLite only if the index gets too slow to load (that would take tens of thousands of runs).

Data dir layout (`~/Library/Application Support/RunReCap/`, overridable with `RUNRECAP_HOME` for dev/tests):
```
config.toml      user settings (rewritten by the app; hand edits are reloaded within 60 s)
state.json       current run, pinned, paused (= preview only), the user's original wallpapers per screen
library/         index.json + tracks/
renders/         <screen-id>-<millis>.png
backgrounds/     the uploaded photo, converted to <hash>.png
fonts/           imported font files (relative `file =` paths resolve here)
```

## 5. Rendering / visual styles (`recap-render`)

Pipeline: `GPX → points (lat, lon, ele, time, hr) → simplify (RDP) → privacy trim → project (Web Mercator) → fit to area (aspect kept, safe-area and caption band excluded) → draw (tiny-skia)`.

Layouts (`Layout` enum):
1. **Single run**: one route, large and centered, with a caption (name / stats / date).
2. **Grid**: up to 240 runs as small multiples, each normalized to its own bbox. The current run gets the glow.
3. **Heatmap**: all runs within 30 km of the current run's start, in one geographic frame (1st–99th percentile bounds), with low-alpha strokes stacked (additive on dark backgrounds) so frequently run streets glow.
4. **Animated trace** (Live mode only, roadmap).

### 5.1 Font and route color

**Text / font.** This covers every piece of text drawn on the wallpaper (run name, stats, date, grid/heatmap labels).
- `font = { bundled = "Inter" }`, `{ system = "Avenir Next" }`, or `{ file = "fonts/MyFont.otf" }`, plus a `weight`. System fonts are scanned only when one is requested.
- If a font is missing it falls back to bundled **Inter** (`assets/fonts/`, SIL OFL, license shipped in the app) with a warning in the menu and window. The render never fails.
- `size_pct` (title size as % of screen height; stats 0.56×, date 0.46×), `color`, `letter_spacing` (em), `uppercase`, `position` (bottom-left/center/right, top-left/right), `fields` (name, date, distance, time, pace, elevation), and units km/mi (global `units`).
- Shaping and rasterizing: `cosmic-text`, with glyphs blended onto the pixmap. Letter spacing is applied per glyph, since cosmic-text 0.12 has no tracking.
- _TBD UI:_ `NSFontPanel` picker in the Preferences window. Resolve the chosen `NSFont` to its file through CoreText (`kCTFontURLAttribute`). The renderer itself never calls AppKit.

**Route (GPX line) color.**
- `mode`: `solid` (`color`, hex with optional alpha), `gradient` (start → finish along `gradient` stops), or data-driven `pace` / `elevation` / `heart_rate` / `time_of_day` mapped onto `gradient` (5th–95th percentile normalization).
- Also: `width_pct`, `glow` (color + `radius_pct`), and `markers` (start dot, finish ring).
- _TBD UI:_ `NSColorWell`/`NSColorPanel` in Preferences.

**Background** (theme): `{ kind = "solid", color }`, `{ kind = "gradient", colors }` (top → bottom), or `{ kind = "image", path, dim }` (PNG). An uploaded photo (§5.2) overrides it.

**Themes.** Built-ins come from `Design.md` (§6.2). Custom themes are `[[themes]]` tables in `config.toml`, and a custom theme with a built-in's name replaces it. The theme is chosen from the window pop-up or the menu-bar Theme ▸ menu. "Automatic" = Apple Dark/Light following the system appearance. _TBD:_ "save current settings as a theme" from the UI.

Example `config.toml` (only the keys you change are needed):
```toml
theme = "Midnight"          # a theme name, or "Automatic"
layout = "single"           # single | grid | heatmap
units = "km"

[[themes]]
name = "Midnight"
background = { kind = "solid", color = "#000000" }

[themes.text]
font = { system = "Avenir Next", weight = 600 }   # or { bundled = "Inter" } / { file = "fonts/MyFont.otf" }
color = "#f5f5f7"
position = "bottom-left"

[themes.route]
mode = "pace"               # solid | gradient | pace | elevation | heart_rate | time_of_day
color = "#2997ff"
gradient = ["#2997ff", "#f5f5f7"]
```

**Rotation:** `[rotation] every_minutes` (0 / 15 / 60 / 240 / 1440 / 10080) and `order` (newest | shuffle). "Pin This Run" stops rotation.

No map tiles in v1. The lines alone are the art. _(TBD: optional faint basemap later. Tile licensing applies.)_

### 5.2 Background photo (`crates/recap-app/src/background.rs`)
- Upload with **Choose Photo…** in the window, by dropping an image on the preview (images become the background, everything else is imported as runs), or with `runrecap background <file> [--dim 0.3] | --clear`.
- Accepted: anything macOS ImageIO reads (JPEG, PNG, HEIC, TIFF, WebP…).
- It's converted once in `recap_macos::convert_image_to_png`: EXIF orientation is applied, the long side is capped at max(largest display, 3840) and never above 8192 (it's never enlarged), and it's saved to `backgrounds/<hash>.png`. The previous photo is deleted. The renderer only reads PNG.
- **Recommended size** = the largest width and the largest height across the connected displays (e.g. 2880 × 1800 on a 14" MacBook, 5120 × 2880 with a 5K display). Landscape. Photos are aspect-filled (cropped), never stretched. If the photo would need upscaling, the window and CLI warn.
- **Dim** (0–70%, default 30%) darkens the photo so the route and caption stay readable. With a photo set, "Automatic" uses Apple Dark (light text) whatever the system appearance.
- Stored as `background_image` in `config.toml`. It overrides the theme's background, so it stays when switching themes.

## 6. App shell (`crates/recap-app`)

- **Main window opens on launch** (`window.rs`). It contains:
  - a live wallpaper preview, shaped like the main screen, that doubles as a **drop zone** (runs, Strava zips, photos);
  - the run name and stats, plus "Version x.y.z";
  - Previous / Next / Shuffle, and Layout and Theme pop-ups;
  - a Background row: Choose Photo… / Remove, a Dim slider, and the recommended resolution (or the current photo's size);
  - Import GPX Files… / Import Strava Export…, a status line, **Update to x.y.z** (only when available), and the primary **Set as Wallpaper** / **Restore My Wallpaper** button.
- **No prompts at startup.** An empty library shows the drop zone with Strava export instructions.
- While the window is open the app uses the `Regular` activation policy (Dock icon, app menu with Check for Updates…, ⌘O import, ⌘W close). Closing the window switches back to `Accessory` (menu bar only).
- **Menu-bar menu** (`NSStatusItem`, SF Symbol `figure.run`, `LSUIElement = true`), from top to bottom:
  - the current run (thumbnail, stats, date) and any warnings;
  - Open RunReCap;
  - Next / Previous / Shuffle / Pin This Run;
  - Layout ▸ / Theme ▸ / Change Wallpaper ▸ (interval + order);
  - Import GPX Files… / Import Strava Export…;
  - Open Settings File… / Show Data Folder;
  - Update to x.y.z… / Check for Updates… / the version;
  - Launch at Login / Set as Wallpaper or Restore My Wallpaper / Quit.
- **Version & updates** (`update.rs`):
  - The app checks the feed at launch and then daily. When a newer version exists, an **Update to x.y.z** button appears.
  - Clicking it confirms, then downloads the DMG with `/usr/bin/curl` and verifies it: the SHA-256 checksum, `codesign --verify`, the same bundle ID, the expected version, and (once Developer ID signed) the same team ID.
  - It stages the new app next to the installed one and quits. A helper script swaps the bundles after the app exits (rolling back on failure), strips quarantine, and reopens it.
  - It refuses to update when running from the DMG or outside a `.app`.
  - The feed URL is baked in at build time (`RUNRECAP_UPDATE_URL`, which `make-dmg.sh` defaults to this repo's GitHub Releases) or overridden with `update_url` in `config.toml`.
- Launch at login: `SMAppService.mainApp` (works from the `.app` bundle only).
- _TBD:_ Preferences window (pure Rust + AppKit) with Appearance (font/color pickers, live preview, save as theme), Library (hide/delete runs), Rotation and General tabs. Until it exists, "Open Settings File…" opens `config.toml`.

### 6.1 Supported macOS versions
- **Policy:** support only macOS versions **released within the last 5 years**. Check this policy at every release and raise the minimum when the oldest version ages out.
- **Currently: macOS 13 Ventura or later** (`LSMinimumSystemVersion = 13.0`; `MACOSX_DEPLOYMENT_TARGET=13.0` is set in `.cargo/config.toml`). macOS 12 Monterey (released 2021-10-25) ages out in Oct 2026.
- Within that range, APIs can be used freely: `SMAppService`, modern `NSWorkspace` desktop-image APIs, etc. For anything newer than the minimum, guard it with a runtime availability check.
- Test on the minimum version and on the latest version before each release.

### 6.2 Design system: how `Design.md` applies

`Design.md` is the visual source of truth, but it is written for the **web** (CSS, px, hover, `backdrop-filter`). RunReCap is a **native AppKit app**, so apply it in three tiers.

**Tier 1: wallpaper default themes (apply fully).** These are built into `recap-render/src/theme.rs`:

| Theme | Background | Text | Route | Font |
|---|---|---|---|---|
| **Apple Dark** (default) | `#000000` | `#f5f5f7` | `#2997ff` + glow | Inter 600 |
| **Apple Light** | `#f5f5f7` | `#1d1d1f` | `#1d1d1f` (gradient stop `#0071e3`) | Inter 600 |
| **Mono Dark** | `#000000` | `#f5f5f7` | `#f5f5f7` | Inter 600 |

Captions use the type scale from `Design.md` (600 for the run name, 400 for stats and date), with slight negative letter spacing and **no weight 800/900**. A unit test checks that the built-ins meet WCAG AA text contrast.

**Tier 2: custom-drawn app surfaces (apply the tokens).**
Today that's the preview / drop-zone frame (8px radius, no border at rest, fill color only) and the empty state. Later it will include the Preferences live preview and the library run cards.
- Cards: no border, no shadow at rest. Spacing comes from the `Design.md` scale (2/4/8/16/24 px), and corners are never rounder than 12px.
- Accent: one color only, used solely for interactive elements and focus (the drop zone shows the system accent only while a drag is over it).
- Translucent panels use native `NSVisualEffectView`, not a hand-made blur.
- Colors: the window currently uses only **system semantic colors** (`secondaryLabelColor`, `quaternarySystemFillColor`, `controlAccentColor`…), which already track light/dark mode. If a custom surface ever needs a `Design.md` hex value, add a `tokens` module (light/dark pairs mirroring the `Design.md` front matter) rather than hard-coding hex in UI code.

**Tier 3: standard controls (native macOS wins).**
Buttons, pop-ups, sliders, the color well, the font panel, and the menu-bar menu all use **stock AppKit controls**, unstyled.
They already look Apple and track the user's system accent color, dark mode, Reduce Transparency, and Increase Contrast.
Do **not** recreate the web patterns from `Design.md` (iOS-size switches, 980px pill CTAs, hover-opacity buttons) natively.
UI text in the app chrome uses the **system font**. Inter is for the wallpaper.
The single primary action (blue, Return key) is "Set as Wallpaper". "Update to x.y.z" is a regular button.

**Scope of the "Don'ts".** The `Design.md` rules "no gradients on backgrounds" and "no additional accent colors" apply to the **app UI only**.
The wallpaper is the user's artwork: user-chosen route colors, gradients, backgrounds and photos are allowed. The built-in default themes still follow the rules.

**Accessibility.** Text on the wallpaper must meet WCAG AA contrast against its background (`Theme::text_contrast`). _TBD:_ warn in Preferences when a user-chosen combination doesn't.

## 7. Repository layout

```
RunReCap/
├── AGENTS.md / Design.md / README.md
├── Cargo.toml                # workspace; [workspace.package] version = app version
├── .cargo/config.toml        # MACOSX_DEPLOYMENT_TARGET = 13.0
├── crates/
│   ├── recap-core/           # GPX parser, Strava import, library/index, filter, geometry. No macOS deps, no unsafe.
│   ├── recap-render/         # layouts, themes, colors, captions, fonts → tiny-skia Pixmap. No AppKit deps, no unsafe.
│   │   ├── tests/layouts.rs  # renders every layout × built-in theme
│   │   └── examples/icon.rs  # generates packaging/AppIcon.png
│   ├── recap-macos/          # thin OS services: screens, set/get wallpaper, appearance, time zone,
│   │                         # login item, bundle path, ImageIO photo conversion
│   └── recap-app/            # binary `runrecap`: CLI (main.rs), app delegate + menus (app.rs),
│                             # main window (window.rs), engine, config/state, background, update
├── assets/                   # fonts/ (Inter + OFL license), test-gpx/ sample runs
├── docs/                     # landing page for GitHub Pages: index.html (static, no build step),
│                             # screenshot.png (also used by README), img/ (icon, layout renders)
└── packaging/                # Info.plist, AppIcon.{png,icns}, bundle.sh, make-dmg.sh, dmg-readme.txt
```

Rules:
- `recap-core` and `recap-render` must stay platform-independent, `unsafe`-free and unit-testable on any OS.
- Reusable OS calls (screens, wallpaper, ImageIO, SMAppService…) go in `recap-macos` behind safe functions.
- AppKit UI code (classes from `define_class!`, windows, menus) lives in `recap-app/src/app.rs` and `window.rs`. Keep `unsafe` there minimal, with a `// SAFETY:` comment on each block.

## 8. Dependencies (current)

| Purpose | Crate |
|---|---|
| macOS bindings | `objc2`, `objc2-foundation`, `objc2-app-kit`, `objc2-quartz-core` (CALayer), `objc2-service-management`, `objc2-image-io`, `objc2-core-foundation`, `objc2-core-graphics` |
| Main-thread hand-off | `dispatch2` (`DispatchQueue::main().exec_async`) |
| GPX / import | `quick-xml` (own parser), `flate2` (.gz), `zip`, `csv`, `sha2` |
| Geometry | in-house (`recap-core/src/geo.rs`: haversine, RDP, Web Mercator) |
| 2D render | `tiny-skia` |
| Text shaping / fonts | `cosmic-text` 0.12 (+ its `fontdb`) |
| Colors | in-house `Rgba` (hex parsing, gradients, WCAG contrast) |
| Dates | `time` |
| Config / index | `serde`, `toml`, `serde_json` |
| CLI | `clap` |
| Logging / errors | `tracing`, `tracing-subscriber`; `thiserror` in libs, `anyhow` in the app |
| Planned | `wgpu` (Live mode), `notify` (watched folders) |

Networking uses the system tools (`/usr/bin/curl`, `hdiutil`, `ditto`, `codesign`), so there's no HTTP/TLS crate. Add new dependencies sparingly, and justify each one in the commit/PR description.

## 9. Conventions for agents

- **Build / run:** `cargo run -p recap-app` opens the app (window + menu bar). The binary is `runrecap`.
- **CLI** (no UI, never touches the wallpaper):
  - `runrecap import <files|folders|strava.zip>`
  - `runrecap list`
  - `runrecap themes`
  - `runrecap screens`
  - `runrecap render [--gpx FILE] [--layout single|grid|heatmap] [--theme NAME] [--light] [-o out.png]`
  - `runrecap background [<photo> [--dim 0.3] | --clear]` (no args prints the recommended size)
  - `runrecap update [--check] [--no-relaunch]` (installs only when run from inside `RunReCap.app`)
- **Dev data:** set `RUNRECAP_HOME=/some/tmp/dir` so you don't touch the real library. Sample runs are in `assets/test-gpx/`.
- **Don't run the GUI against the user's real desktop in automated checks.** Preview-only mode (`"paused": true`, the default for new data dirs) never touches the wallpaper.
- **UI checks without Screen Recording permission:** `RUNRECAP_HOME=<tmp> RUNRECAP_SNAPSHOT=out.png runrecap` writes the window to a PNG once rendering, imports, photo conversion and the update check have settled, then quits. `RUNRECAP_DEV_IMPORT=<path[:path…]>` takes the same path as a drop (photos → background, the rest → import).
- **Testing updates locally:** build an old version with `RUNRECAP_UPDATE_URL=http://127.0.0.1:8765/latest.json packaging/make-dmg.sh`, copy the `.app` into a temp folder, bump the version, build again, and serve the new DMG + `latest.json` with `python3 -m http.server 8765`. Then run `<temp>/RunReCap.app/Contents/MacOS/runrecap update --no-relaunch`. **Never test updates against `/Applications`.**
- **Check before finishing any change:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- AppKit calls happen **on the main thread only**. Use `MainThreadMarker` from `objc2`, and never fake it.
- Rendering, importing, photo conversion and update downloads run on worker threads. Results come back through `DispatchQueue::main()` → `with_delegate(...)`. The main thread only `try_lock`s the engine (an import holds it), so it never blocks.
- Never log raw coordinates at `info` level or above (privacy).
- Battery behavior is a feature: any change to Live mode must keep the pause conditions in §3.2.
- `config.toml` is serialized by the app: plain values must come before tables in the `Config` struct (TOML ordering), and a round-trip test guards this.
- Tests: unit tests live next to the code. `recap-render/tests/layouts.rs` smoke-tests every layout × built-in theme (pixels drawn, no warnings). Put sample GPX files in `assets/test-gpx/`.
- **Releasing:**
  1. Bump `[workspace.package] version` in `Cargo.toml`.
  2. Run `NOTES="…" packaging/make-dmg.sh` → `target/RunReCap-<v>.dmg` + `target/latest.json`.
  3. Create a GitHub release `v<version>` and upload **both** files.

  The feed URL (`…/releases/latest/download/latest.json`) is baked in by `make-dmg.sh`. **Every** release must be built by it, or that version can't update itself. Optional: `SIGN_ID` (+ `NOTARY_PROFILE`) for Developer ID signing and notarization. The app icon is regenerated with `cargo run -p recap-render --example icon -- packaging/AppIcon.png`, then the `.icns` is rebuilt with `iconutil`.
- **Landing page** (`docs/index.html`): a single static HTML file served by GitHub Pages (Settings → Pages → `main` / `docs`). It follows `Design.md` **fully** (it's a web page: Inter, black / #f5f5f7 sections, pill CTAs, the single blue accent), with dark-mode tokens, a 16px phone gutter and `prefers-reduced-motion`.
  - Keep its features, install steps and roadmap in sync with this file.
  - Showcase images in `docs/img/` come from `runrecap render` on a scratch library (`RUNRECAP_HOME=<tmp>`) of synthetic runs, never the user's real runs and never copyrighted photos.
  - Check it with headless Chrome: `"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --window-size=1440,7400 --screenshot=out.png file://$PWD/docs/index.html`. For phone width, load it in a 390 px iframe, since headless windows can't be narrower than ~500 px.
- **Git / GitHub:**
  - Branch `main` on `origin` = `github.com/azuanalias92/RunReCap`. Commit messages: imperative mood, scoped (e.g. `render: add pace coloring to heatmap style`). Don't commit or push unless the user asks.
  - This Mac's GitHub CLI has two accounts. `azuan-cps` (work) is normally active and **can't push here**. Push as `azuanalias92` with `gh auth switch -u azuanalias92`, then `git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push`, then `gh auth switch -u azuan-cps`.

## 10. Roadmap

### v1 — Static wallpaper from manually imported Strava runs
1. ✅ **M0 – Spike**: CLI that reads one GPX and writes a PNG.
2. ✅ **M1 – Static wallpaper + main window**: the preview window (drop to import, Set as Wallpaper / Restore), the menu-bar app, multi-monitor support, Space/wake re-apply, saving and restoring the original wallpaper.
3. ✅ **M2 – Import & library**: GPX / .gpx.gz / folders / Strava export (.zip or folder), library copy and dedupe, index cache, runs-only + distance filter, privacy trim, rotation schedule. _TODO:_ dropping onto the menu-bar icon; date-range filter UI.
4. ✅ **M3 – Styles & customization**: Apple Dark/Light and Mono Dark themes, Single/Grid/Heatmap layouts, custom font (bundled/system/file), route color modes (solid, gradient, pace, elevation, heart rate, time of day), custom themes in `config.toml`, light/dark following the system, **background photo upload** with dim and resolution guidance.
5. 🟡 **M4 – Ship v1**:
   - Done: universal `.app`, app icon, DMG + `latest.json` (`make-dmg.sh`), launch-at-login toggle, version display, **built-in self-update from GitHub Releases**, and the repo on GitHub.
   - TODO:
     - publish the first GitHub release (`v0.1.0`);
     - a **Developer ID Application** certificate (paid Apple Developer Program; only Apple Development certs exist today), then build with `SIGN_ID` + `NOTARY_PROFILE` so the DMG opens without warnings;
     - change the bundle ID `com.runrecap.app` (`packaging/Info.plist`) to one the owner controls, before the first public release;
     - the Rust/AppKit Preferences window (font/color pickers, save as theme, contrast warning) and a Library view for hiding/deleting runs.

### Post-v1 (roughly in priority order; the landing page shows the top items)
- **Strava integration (next)**: "Connect your Strava account". OAuth, incremental fetch of new runs, streams converted to tracks, token kept in the Keychain, new runs appearing automatically. Check Strava's API terms and branding guidelines first.
- **Windows version (later)**: reuse `recap-core` + `recap-render` unchanged. Add a `recap-windows` crate (set the wallpaper with `IDesktopWallpaper`, system tray, WinUI/Win32 window via the `windows` crate), plus an installer and its own update channel. The GPX library, themes and config format stay identical.
- **Live mode**: desktop-level Metal windows, animated route tracing, all the pause conditions in §3.2, and a "static on battery" toggle.
- **Watched folders**: auto-import new GPX files from user-chosen folders (`notify` / FSEvents).
- **Apple Health export import**: `export.zip` → `workout-routes/*.gpx`.
- **`.fit` / `.tcx` support**: covers the remaining Strava export files and Garmin devices.
- **Garmin Connect** import/sync.
- **HealthKit**: direct access (needs entitlements; check whether it's possible outside the App Store).
- **Optional faint basemap** under the routes (check tile licensing).
- **More rotation modes**: "run of the day", "on this day N years ago".
- **Lock screen**: confirm whether macOS uses the desktop picture there, and support it if possible.
- **Updater hardening**: an EdDSA signature on `latest.json` (Sparkle-style), so a compromised release host alone can't push an update. Today's checks are the checksum + code signature (+ team ID once Developer ID signed).
- **Export / share**: save the current render as a PNG poster at an arbitrary resolution.

## 11. Open questions

- When will the Developer ID certificate (Apple Developer Program) be bought? Until then every user sees the Gatekeeper "Open Anyway" step on first launch.
- What should the final bundle ID be (currently `com.runrecap.app`)? Changing it after release breaks updates for installed copies, because the updater requires the same bundle ID.
