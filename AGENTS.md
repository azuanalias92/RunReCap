# AGENTS.md — RunReCap

Guidance for AI coding agents (and humans) working on this repository.

> Status: **v1 in progress.** M0–M3 are working and M4 is partly done (see §10 for what's left). Sections marked _(TBD)_ are open decisions.

### Decisions (2026-10-09)
- **Distribution:** direct download (notarized DMG). There is no App Store sandbox, so plain file paths are fine.
- **v1 scope:** **Static mode only.** Live/animated mode is on the roadmap (§10), not in v1.
- **Data input:** **manual upload** only: the user imports files (drag-and-drop or the "Import…" menu). There is no API sync in v1.
- **Primary source:** **Strava**. The Strava bulk export (`.gpx`, `.gpx.gz`) is the first-class import path.
- **All Rust:** no Swift, SwiftUI, or Obj-C source files. AppKit is reached only through `objc2-*` crates. The Preferences UI is built in Rust with AppKit.
- **Minimum macOS:** a rolling policy covering **only macOS versions released in the last 5 years** (§6.1).
- **Customization (v1):** the user chooses the **font** for all text on the wallpaper and the **color** of the GPX route (§5.1).
- **Design system:** [`Design.md`](Design.md) (Apple style) is the visual source of truth. §6.2 explains how it maps onto a native app.

## 1. What this project is

RunReCap is a macOS desktop-wallpaper app, in the spirit of Wallpaper Engine but much narrower:
the wallpaper only ever shows **drawings of the user's past runs, rendered from GPX tracks**.

- Platform: macOS only (Apple Silicon first, Intel best-effort), MacBook and desktop Macs, multi-monitor.
- Language: **Rust only**. No Swift or Obj-C sources in the repo.
- Form factor: a menu-bar app (no Dock icon) that owns the desktop background.

### Non-goals
- Not a general-purpose wallpaper engine (no video, web, or arbitrary scenes).
- Not a training/analytics app. Stats are decoration, not the product.
- No Windows/Linux support.

## 2. Why Rust (not Go)

| Concern | Rust | Go |
|---|---|---|
| AppKit / Cocoa bindings | `objc2`, `objc2-app-kit`, `objc2-foundation` — typed, maintained | cgo + hand-written Obj-C shims |
| GPU rendering | `wgpu` (Metal backend) or `objc2-metal` directly | No mature Metal path |
| 2D vector rendering | `tiny-skia`, `vello` | Limited |
| Idle footprint (runs 24/7) | No GC, small RSS | GC pauses, higher baseline |
| GPX parsing | `gpx` crate | `tkrajina/gpxgo` (fine) |

Go would be fine for the data side, but the wallpaper side is all AppKit + Metal, which is where Rust is much stronger.

## 3. How the wallpaper is shown

There are two display modes. **v1 ships Static only.** Live mode is on the roadmap.

### 3.1 Static mode (default, battery-friendly)
1. Render one image per screen at that screen's native pixel size (`NSScreen.backingScaleFactor`).
2. Write it to `~/Library/Application Support/RunReCap/renders/<screen-id>-<hash>.png`.
3. Apply with `NSWorkspace.shared.setDesktopImageURL(_:for:options:)` for each `NSScreen`.
4. Re-render only when the inputs change: a new GPX, a config change, a screen change, or a scheduled rotation.

CPU/GPU cost between renders is zero. This is how the app should behave on battery.

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

### 3.3 Screen / Space handling
- Listen to `NSApplication.didChangeScreenParametersNotification` (screens added or removed, resolution changes) and rebuild.
- Listen to `NSWorkspace.activeSpaceDidChangeNotification`; in static mode, re-apply the image, since macOS can reset per-Space wallpapers.

## 4. Data: where runs come from

v1: **manual upload, Strava first** (local only, no network):
- **Import files**: the user drags `.gpx` / `.gpx.gz` files onto the menu-bar icon or Preferences window, or uses "Import…" (`NSOpenPanel`, multi-select).
- **Import Strava export**: the user picks the Strava bulk-export `.zip` (or its unzipped folder). The app reads `activities.csv` for the activity type, name and date, and `activities/*.gpx(.gz)` for the routes.
- Imported files are **copied** into `~/Library/Application Support/RunReCap/library/` (keyed by content hash), so the originals can be deleted. Duplicate imports are skipped.
- Activities in the Strava export that are only `.fit` / `.tcx` are skipped, and the import summary reports them.

Roadmap sources (§10): watched folders, Apple Health export, `.fit`/`.tcx`, the Strava API, Garmin Connect, and HealthKit.

### Filtering
- Activity type: run only by default. Read `<type>` from the GPX if present; otherwise use a speed heuristic.
- Date range, minimum distance, and location (a bounding box or "near home").
- **Privacy**: an optional "hide start/end" radius (e.g. 200 m), which trims points near the start and end so the user's home isn't revealed on screen shares.

### Cache
Parsed tracks are stored in `library/index.json` (serde_json), with the imported GPX copies in `library/tracks/<id>.gpx`. `<id>` is a SHA-256 prefix of the file content, which is also how duplicates are detected.
Each record holds the simplified polyline (RDP, 1.5 m tolerance), distance, duration, date, and elevation gain. GPX files are never re-parsed.
Switch to SQLite only if the index gets too slow to load (that would take tens of thousands of runs).

Data dir layout (`~/Library/Application Support/RunReCap/`, overridable with `RUNRECAP_HOME` for dev/tests):
```
config.toml      user settings (rewritten by the menu)
state.json       current run, pinned/paused, the user's original wallpapers per screen
library/         index.json + tracks/
renders/         <screen-id>-<millis>.png  (new name each time; macOS caches wallpapers by path)
fonts/           imported font files (relative `file =` paths resolve here)
```

## 5. Rendering / visual styles

Pipeline: `GPX → points (lat, lon, ele, time) → simplify (Douglas-Peucker / Visvalingam) → project (Web Mercator, or local equirectangular per run) → fit to canvas with padding → draw`.

Planned styles (each is a `Style` implementation):
1. **Single run**: one route, large and centered, with an optional small caption (date · distance · pace).
2. **Grid / poster**: every run as a small multiple in a grid, each normalized to its own bbox ("my year in runs").
3. **Heatmap / overlay**: all runs in one geographic frame, low-alpha strokes stacked additively, so frequently run streets glow.
4. **Animated trace** (Live mode only): the route draws itself over N seconds, colored by pace or elevation.

Style parameters: background color or gradient, stroke color / width / glow, coloring mode (solid | pace | elevation | heart rate | time of day), caption on or off, and the light/dark variant (follow the system appearance).

### 5.1 User customization: font and route color (v1)

**Text / font.** This covers every piece of text drawn on the wallpaper (run name, date, distance, pace, grid labels).
- Font family and weight: choose any **installed font** with the native `NSFontPanel` (via `objc2-app-kit`), **or import a `.ttf` / `.otf` / `.ttc` file**. Imported fonts are copied to `~/Library/Application Support/RunReCap/fonts/`.
- Resolve the chosen `NSFont` to its file URL through CoreText (`kCTFontURLAttribute`) and load that file into the renderer. The renderer itself never calls AppKit.
- Font size (relative to screen height, so it scales across displays), text color plus opacity, alignment and position (corner or below the route), uppercase toggle, letter spacing.
- Which fields appear, in order: name, date, distance, time, pace, elevation gain. The date and number format follow the system locale, and the user can choose km or mi.
- Default and fallback font: **Inter**, bundled in `assets/fonts/` (SIL OFL, so include its license file). If a chosen font file is missing (e.g. it was uninstalled), fall back to Inter and show a notice in the menu. Never fail the render.
- Text shaping and rasterizing: `cosmic-text` (with `fontdb`), with glyphs composited onto the `tiny-skia` pixmap.

**Route (GPX line) color.**
- **Solid**: one color picked with `NSColorPanel` / `NSColorWell`, or entered as a hex code. Alpha is supported.
- **Gradient**: 2+ color stops along the route (start → end).
- **Data-driven**: map pace / elevation / heart rate / time of day onto a user-chosen gradient (the stops are editable). Ship a few presets.
- Also adjustable: stroke width, line cap/join, glow (color + radius), and start/finish marker dots on/off with their colors.
- Background: solid color, gradient, or an image chosen by the user (the route is drawn on top).

**Background photo upload** (`crates/recap-app/src/background.rs`):
- Upload with **Choose Photo…** in the window, by dropping an image on the preview, or with `runrecap background <file> [--dim 0.3] | --clear`.
- Accepted: anything macOS ImageIO reads (JPEG, PNG, HEIC, TIFF, WebP…).
- It's converted once in `recap-macos::convert_image_to_png`: EXIF orientation is applied, it's downscaled to a long side of at most max(largest display, 3840) and never above 8192, and it's saved to `backgrounds/<hash>.png`. The renderer only reads PNG.
- **Recommended size** = the largest width and the largest height across the connected displays (e.g. 2880 × 1800 on a 14" MacBook, 5120 × 2880 with a Studio Display). Landscape. Photos are aspect-filled (cropped), never stretched. If the photo would need upscaling, the window and CLI warn.
- **Dim** (0–70%, default 30%) darkens the photo so the route and caption stay readable. With a photo set, "Automatic" uses Apple Dark (light text) whatever the system appearance.
- The photo is stored as `background_image` in config.toml and overrides the theme's background, so it stays when switching themes.
- Separate light and dark palettes, optionally following the system appearance.

**Presets / themes.** Save the current font and colors as a named theme, and switch themes from the menu-bar Style ▸ menu. Themes are stored in `config.toml`.

Example config:
```toml
[theme]
name = "Apple Dark"     # default theme, from Design.md colors-dark

[theme.text]
font = { bundled = "Inter", weight = 600 }   # or { system = "SF Pro Display", weight = 600 } / { file = "fonts/MyFont.otf" }
size_pct = 2.2          # % of screen height
color = "#f5f5f7"
letter_spacing = -0.01  # Design.md: slight negative tracking
position = "bottom-left"
fields = ["name", "date", "distance", "pace"]
units = "km"

[theme.route]
mode = "solid"          # solid | gradient | pace | elevation | heart_rate | time_of_day
color = "#2997ff"
gradient = ["#2997ff", "#f5f5f7"]
width_pct = 0.35
glow = { color = "#2997ff40", radius_pct = 0.6 }

[theme.background]
kind = "solid"
color = "#000000"
```

No map tiles in Phase 1. The lines alone are the art. _(TBD: optional faint basemap later. Tile licensing applies.)_

Rotation: show a new run every N minutes/hours, daily, or "run of the day".

## 6. App shell

- **Main window opens on launch** (`crates/recap-app/src/window.rs`). It contains:
  - a live wallpaper preview, shaped like the main screen, that doubles as a **drop zone** for `.gpx`, `.gpx.gz` and Strava `.zip` files;
  - the run name and stats;
  - Previous / Next / Shuffle;
  - Layout and Theme pop-ups;
  - Import GPX Files… and Import Strava Export…;
  - a Background row: Choose Photo… / Remove, a Dim slider, and the recommended resolution (or the current photo's size);
  - a status line;
  - the primary **Set as Wallpaper** / **Restore My Wallpaper** button.
- **No prompts at startup.** An empty library shows the drop zone with Strava export instructions.
- **Preview first.** New installs start in preview-only mode (`state.paused = true`), and the desktop is changed only after "Set as Wallpaper". After that, changes apply live.
- While the window is open the app is a regular app (Dock icon, ⌘O import, ⌘W close). Closing the window returns it to menu-bar-only (`Accessory`). Reopen it from the menu bar with "Open RunReCap".
- **Version & updates** (`crates/recap-app/src/update.rs`):
  - The window shows "Version x.y.z", and the menu has "Check for Updates…".
  - The app checks the release feed at launch and then daily. When a newer version exists, an **Update to x.y.z** button appears.
  - Clicking it confirms, then downloads the DMG with `curl` and verifies it: the SHA-256 checksum, the code signature, the same bundle ID, the expected version, and (once Developer ID signed) the same team ID.
  - It stages the new app next to the installed one and quits. A helper script swaps the bundles after the app exits (rolling back on failure) and reopens it.
  - The feed URL is baked in at build time (`RUNRECAP_UPDATE_URL`) or set with `update_url` in config.toml.
  - CLI: `runrecap update [--check] [--no-relaunch]`.
- Menu-bar item (`NSStatusItem`), `LSUIElement = true`.
- Menu: current run info, Next / Previous, Style ▸, Import Files…, Import Strava Export…, Preferences…, Quit. (Mode ▸ Static/Live is added when Live mode lands.)
- Preferences window: **pure Rust + AppKit** (`objc2-app-kit`). The tabs are Library, Appearance (fonts, colors, themes, with a live preview thumbnail), Rotation, and General. _Not built yet:_ for now, "Open Settings File…" opens `config.toml`, and the app reloads it within 60 s of a save.
- Launch at login: `SMAppService.mainApp.register()` (macOS 13+).

### 6.1 Supported macOS versions
- **Policy:** support only macOS versions **released within the last 5 years**. Check this policy at every release and raise the minimum when the oldest version ages out.
- **Currently: macOS 13 Ventura or later** (`LSMinimumSystemVersion = 13.0`, `MACOSX_DEPLOYMENT_TARGET=13.0`). macOS 12 Monterey (released 2021-10-25) ages out in Oct 2026, before v1 ships.
- Within that range, APIs can be used freely: `SMAppService`, modern `NSWorkspace` desktop-image APIs, etc. For anything newer than the minimum, guard it with a runtime availability check.
- Test on the minimum version and on the latest version before each release.

Config lives at `~/Library/Application Support/RunReCap/config.toml` (serde + `toml`).

### 6.2 Design system: how `Design.md` applies

`Design.md` is the visual source of truth, but it is written for the **web** (CSS, px, hover, `backdrop-filter`). RunReCap is a **native AppKit app**, so apply it in three tiers.

**Tier 1: wallpaper default themes (apply fully).**
Ship these built-in themes, built from the `Design.md` tokens:

| Theme | Background | Text | Route | Font |
|---|---|---|---|---|
| **Apple Dark** (default) | `#000000` | `#f5f5f7` | `#2997ff` | Inter 600 |
| **Apple Light** | `#f5f5f7` | `#1d1d1f` | `#1d1d1f` (or `#0071e3`) | Inter 600 |
| **Mono Dark** | `#000000` | `#f5f5f7` | `#f5f5f7` | Inter 600 |

Captions use the type scale from `Design.md` (display 600 for the run name, body 400 for stats, caption for the date), with slight negative letter spacing and **no weight 800/900**.
"Apple Dark" or "Apple Light" follows the system appearance automatically when the theme is set to "Automatic".

**Tier 2: custom-drawn app surfaces (apply the tokens).**
These are the parts we draw ourselves: the welcome/onboarding window, the library run cards, the Appearance live-preview frame, empty states, and the import summary.
- Cards: `surface` color (`#f5f5f7` light / `#272729` dark), 8px corners, **no border**, no shadow at rest.
- Spacing comes only from the `Design.md` scale (2/4/8/16/24 px). Corner radius is never more than 12px on rectangles.
- Accent: one color only, used solely for interactive elements and focus.
- Translucent panels use native `NSVisualEffectView`, not a hand-made blur.
- Read colors from a single Rust `tokens` module (light/dark pairs generated from or mirrored from `Design.md`'s front matter). **Never hard-code hex values in UI code.**

**Tier 3: standard controls (native macOS wins).**
Buttons, switches (`NSSwitch`), pop-ups, sliders, checkboxes, the color well, the font panel, and the menu-bar menu all use **stock AppKit controls**, unstyled.
They already look Apple and track the user's system accent color, dark mode, Reduce Transparency, and Increase Contrast.
Do **not** recreate the web patterns from `Design.md` (iOS-size switches, 980px pill CTAs, hover-opacity buttons) natively.
UI text in the app chrome uses the **system font** (SF Pro via `NSFont.systemFont`). Inter is for the wallpaper and the custom surfaces in Tier 2 only.

**Scope of the "Don'ts".** The `Design.md` rules "no gradients on backgrounds" and "no additional accent colors" apply to the **app UI only**.
The wallpaper is the user's artwork: user-chosen route colors, gradients and backgrounds are allowed (§5.1). The built-in default themes still follow the rules.

**Accessibility.** Text on the wallpaper must meet WCAG AA contrast against its background, or get a subtle backing. Warn in Appearance when a user-chosen combination doesn't meet it.

## 7. Proposed repository layout

```
RunReCap/
├── AGENTS.md
├── Design.md                 # visual design system (Apple style), see §6.2
├── Cargo.toml                # workspace
├── crates/
│   ├── recap-core/           # GPX parsing, filtering, simplification, projection, cache. NO macOS deps.
│   ├── recap-render/         # Styles → pixels (tiny-skia for static, wgpu for live). NO AppKit deps.
│   ├── recap-macos/          # objc2 glue: NSWorkspace, desktop windows, screens, power, status item
│   └── recap-app/            # binary: wires everything together, event loop, config
├── assets/                   # icons, fonts/ (Inter + OFL license), sample GPX files for tests
└── packaging/                # Info.plist, entitlements, `bundle.sh` (universal .app, codesign)
```

Rule: `recap-core` and `recap-render` must stay platform-independent and unit-testable on any OS.
All `unsafe` / Objective-C code lives in `recap-macos`.

## 8. Key dependencies (initial picks)

| Purpose | Crate |
|---|---|
| macOS bindings | `objc2`, `objc2-foundation`, `objc2-app-kit`, `objc2-quartz-core`, `block2` |
| GPX | `gpx` |
| Geometry / simplification | `geo` |
| Static 2D render | `tiny-skia` (+ `png`) |
| Text shaping / fonts | `cosmic-text` (+ `fontdb`) |
| Colors (hex parsing, gradients) | `csscolorparser` or a small in-house type |
| Live render | `wgpu` (Metal backend) |
| File watching (post-v1) | `notify` |
| Config | `serde`, `toml` |
| Library index | `serde_json` |
| Import | `quick-xml` (GPX), `flate2` (.gz), `zip`, `csv`, `sha2` |
| Main-thread hand-off | `dispatch2` |
| CLI | `clap` |
| Logging | `tracing`, `tracing-subscriber` |
| Errors | `thiserror` in libs, `anyhow` in the app |

Add new dependencies sparingly, and justify each one in the PR description.

## 9. Conventions for agents

- **Build / run:** `cargo run -p recap-app` starts the menu-bar app. The binary is `runrecap`.
- **CLI** (no UI, never touches the wallpaper):
  - `runrecap import <files|folders|strava.zip>`
  - `runrecap list`
  - `runrecap themes`
  - `runrecap screens`
  - `runrecap render [--gpx FILE] [--layout single|grid|heatmap] [--theme NAME] [--light] -o out.png`
- **Dev data:** set `RUNRECAP_HOME=/some/tmp/dir` so you don't touch the real library. Sample runs are in `assets/test-gpx/`.
- **App bundle / installer:** `packaging/bundle.sh` → `target/RunReCap.app` (universal); `packaging/make-dmg.sh` → `target/RunReCap-<version>.dmg`. Both are ad-hoc signed by default; set `SIGN_ID` (+ `NOTARY_PROFILE` for the DMG) for Developer ID signing and notarization. Bump the version in `[workspace.package]` in `Cargo.toml`.
- **Releasing:** run `NOTES="…" packaging/make-dmg.sh` (the feed defaults to this repo's GitHub Releases; override it with `RUNRECAP_UPDATE_URL`), then upload `target/RunReCap-<v>.dmg` **and** `target/latest.json` to the release. With GitHub Releases, the feed URL `https://github.com/<owner>/<repo>/releases/latest/download/latest.json` always points at the newest release. **Every** release must be built with the feed URL, or that version can't update itself.
- **Don't run the GUI against the user's real desktop in automated checks.** Preview-only mode (`"paused": true`, the default for new data dirs) never touches the wallpaper.
- **UI checks without Screen Recording permission:** `RUNRECAP_HOME=<tmp> RUNRECAP_SNAPSHOT=out.png runrecap` writes the window to a PNG after the first render, then quits. Add `RUNRECAP_DEV_IMPORT=<path[:path…]>` to exercise the same import path a drop uses.
- **Check before finishing any change:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- AppKit calls happen **on the main thread only**. Use `MainThreadMarker` from `objc2`, and never fake it.
- Rendering and parsing run off the main thread. Hand results back via the main-thread dispatch queue.
- Keep `unsafe` blocks small and put a `// SAFETY:` comment on each.
- Never block the main thread on file I/O or GPX parsing.
- Battery behavior is a feature: any change to Live mode must keep the pause conditions in §3.2.
- Never log raw coordinates at `info` level or above (privacy).
- UI colors, spacing, radii, and type come from the `tokens` module, which mirrors `Design.md`. If you change a token, update both. Follow §6.2 on whether to use native controls or custom drawing.
- Tests: put sample GPX files in `assets/test-gpx/`. Render tests compare against golden PNGs with a tolerance.
- Commit messages: imperative mood, scoped, e.g. `render: add pace coloring to heatmap style`.

## 10. Roadmap

### v1 — Static wallpaper from manually imported Strava runs
1. ✅ **M0 – Spike**: CLI that reads one GPX and writes a PNG (`recap-core` + `recap-render`, Single-run style).
2. ✅ **M1 – Static wallpaper** (plus the main window: preview, drop-to-import, Set as Wallpaper): menu-bar app that sets the PNG as the wallpaper on all screens, with Next/Previous, multi-monitor support and Space re-apply.
3. ✅ **M2 – Import & library** (drag-and-drop works on the window preview; dropping onto the menu-bar icon is still TODO): manual file import (drag-and-drop / "Import…"), Strava bulk-export import (`.zip`, `activities.csv`, `.gpx.gz`), library copy and dedupe, cache, filters (type, date, distance), privacy trim, rotation schedule.
4. ✅ **M3 – Styles & customization** (fonts and colors are set via `config.toml` until Preferences exists): built-in themes from `Design.md` (Apple Dark/Light, Mono Dark), grid poster, heatmap/overlay, **custom font** (system or imported file), **custom route color** (solid, gradient, pace, elevation, heart rate, time of day), background options, saved themes, light/dark following the system.
5. 🟡 **M4 – Ship v1**: ✅ `.app` bundle (universal), ✅ app icon (`packaging/AppIcon.icns`, generated by `cargo run -p recap-render --example icon`), ✅ DMG (`packaging/make-dmg.sh`), ✅ launch-at-login toggle. Still TODO: a **Developer ID Application** certificate (paid Apple Developer Program; only Apple Development certs exist today), then run `make-dmg.sh` with `SIGN_ID` + `NOTARY_PROFILE` so the DMG opens without warnings. Also TODO: the Rust/AppKit Preferences window with live preview (including `NSFontPanel`/`NSColorPanel` pickers), and a Library tab for hiding/deleting runs.

### Post-v1 (all ideas kept here, roughly in priority order)
- **Live mode**: desktop-level Metal windows, animated route tracing, all the pause conditions in §3.2, and a "static on battery" toggle.
- **Strava API sync**: OAuth, incremental fetch of new activities, streams converted to tracks, token kept in the Keychain.
- **Watched folders**: auto-import new GPX files from user-chosen folders (`notify` / FSEvents).
- **Apple Health export import**: `export.zip` → `workout-routes/*.gpx`.
- **`.fit` / `.tcx` support**: covers the remaining Strava export files and Garmin devices.
- **Garmin Connect** import/sync.
- **HealthKit**: direct access (needs entitlements; check whether it's possible outside the App Store).
- **Optional faint basemap** under the routes (check tile licensing).
- **More rotation modes**: "run of the day", "on this day N years ago", newest run.
- **Lock screen**: confirm whether macOS uses the desktop picture there, and support it if possible.
- **Auto-update**: Sparkle for direct-download builds.
- **Export / share**: save the current render as a PNG poster at an arbitrary resolution.

## 11. Open questions

_None right now._ Add new ones here as they come up.
