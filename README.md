# RunReCap

Your past runs, drawn from GPX, as your Mac's wallpaper. A menu-bar app for macOS 13+, written in Rust.

See [AGENTS.md](AGENTS.md) for the design/roadmap and [Design.md](Design.md) for the visual system.

## Quick start

```sh
cargo run -p recap-app
```

The RunReCap window opens. Drop `.gpx` files or your Strava export `.zip` on the preview, or use **Import GPX Files…** / **Import Strava Export…**. The preview updates right away. Your desktop doesn't change until you click **Set as Wallpaper**, and **Restore My Wallpaper** (or Quit) puts yours back. Closing the window leaves RunReCap in the menu bar.

You can also import from the terminal: `cargo run -p recap-app -- import ~/Downloads/export_12345.zip`.

Get your Strava export at strava.com → Settings → My Account → *Download or Delete Your Account* → *Request your archive*.

## Background photo

Click **Choose Photo…**, or drop an image on the preview, and your runs are drawn on top of it. Use a landscape photo at least as large as your display; the window shows the exact size, e.g. **2880 × 1800 px** on a 14" MacBook Pro, or **5120 × 2880** for a 5K display. Photos are cropped to fill the screen, and the **Dim** slider keeps the route readable. JPEG, PNG and HEIC (iPhone photos) all work.

## Customize

Menu bar → **Open Settings File…** edits `~/Library/Application Support/RunReCap/config.toml`; changes apply within a minute. Example custom theme:

```toml
theme = "Midnight"

[[themes]]
name = "Midnight"
background = { kind = "solid", color = "#000000" }

[themes.text]
font = { system = "Avenir Next", weight = 600 }   # or { file = "fonts/MyFont.otf" }
color = "#f5f5f7"
position = "bottom-left"

[themes.route]
mode = "pace"                        # solid | gradient | pace | elevation | heart_rate | time_of_day
color = "#2997ff"
gradient = ["#2997ff", "#f5f5f7"]
```

## Releases & updates

The app shows its version in the window, and **Check for Updates…** is in the menu. Updates come from a `latest.json` feed:

```sh
# bump [workspace.package] version in Cargo.toml, then:
NOTES="What's new in this version" packaging/make-dmg.sh
# create a GitHub release at https://github.com/azuanalias92/RunReCap/releases
# and upload target/RunReCap-<version>.dmg and target/latest.json to it
```

Installed copies see **Update to x.y.z**, then download, verify, reinstall themselves and reopen.

## Build the installer

```sh
packaging/make-dmg.sh   # → target/RunReCap-<version>.dmg (drag-to-Applications)
```

Without a Developer ID certificate the DMG is ad-hoc signed, so users confirm once under
System Settings → Privacy & Security → **Open Anyway** (instructions are included in the DMG).
With one, the DMG is signed, notarized and stapled:

```sh
xcrun notarytool store-credentials runrecap --apple-id you@example.com --team-id TEAMID --password <app-specific>
SIGN_ID="Developer ID Application: Your Name (TEAMID)" NOTARY_PROFILE=runrecap packaging/make-dmg.sh
```
