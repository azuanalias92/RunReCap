//! Self-update: check a JSON release feed, then download the new DMG, verify
//! it, and swap the installed `RunReCap.app` for the new one.
//!
//! Feed format (`latest.json`, written by `packaging/make-dmg.sh`):
//! ```json
//! { "version": "0.2.0", "url": "RunReCap-0.2.0.dmg", "sha256": "…", "notes": "…" }
//! ```
//! `url` may be relative to the feed URL. The swap happens in a small helper
//! script after this process exits, so the running binary is never replaced
//! underneath itself.
//!
//! Uses the system tools (`curl`, `hdiutil`, `ditto`, `codesign`) so it
//! honours system proxies and needs no TLS stack in the binary.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The running app's version (from `[workspace.package]` in Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Feed URL baked in at build time (`RUNRECAP_UPDATE_URL=… packaging/make-dmg.sh`).
pub const BUILT_IN_FEED: Option<&str> = option_env!("RUNRECAP_UPDATE_URL");

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Numeric `major.minor.patch` comparison; missing parts count as 0 and any
/// pre-release suffix (`-beta`) is ignored.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

fn resolve(feed: &str, url: &str) -> String {
    if url.contains("://") {
        url.to_owned()
    } else {
        let base = feed.rsplit_once('/').map_or(feed, |(b, _)| b);
        format!("{base}/{}", url.trim_start_matches('/'))
    }
}

fn run(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {cmd:?}"))?;
    if !out.status.success() {
        bail!(
            "{:?} failed: {}",
            cmd.get_program(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Fetch the feed. Returns the release only if it's newer than this build.
pub fn check(feed: &str) -> Result<Option<Release>> {
    let body = run(Command::new("/usr/bin/curl").args(["-fsSL", "--max-time", "20", feed]))
        .context("couldn't reach the update server")?;
    let mut release: Release =
        serde_json::from_str(&body).context("update feed is not valid JSON")?;
    release.url = resolve(feed, &release.url);
    Ok(is_newer(&release.version, VERSION).then_some(release))
}

/// Info.plist value of a bundle.
fn plist(app: &Path, key: &str) -> Result<String> {
    let info = app.join("Contents/Info.plist");
    Ok(run(Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(&info))?
    .trim()
    .to_owned())
}

/// Code-signing team of a bundle, if any ("not set" for ad-hoc builds).
fn team_id(app: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=2"])
        .arg(app)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))
        .filter(|t| *t != "not set")
        .map(str::to_owned)
}

pub struct Prepared {
    /// Verified new app, staged next to the installed one (same volume).
    staged: PathBuf,
    installed: PathBuf,
    work: PathBuf,
}

/// Download, verify and stage the update. Safe to call off the main thread;
/// nothing is replaced until [`Prepared::launch_swap`].
pub fn prepare(release: &Release, installed: &Path) -> Result<Prepared> {
    ensure!(
        installed.extension().is_some_and(|e| e == "app"),
        "updates can only be installed when RunReCap runs from RunReCap.app"
    );
    let parent = installed.parent().context("app has no parent folder")?;
    ensure!(
        !parent.starts_with("/Volumes"),
        "RunReCap is running from the disk image. Drag it to Applications first, then update."
    );

    let work = std::env::temp_dir().join(format!("runrecap-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let dmg = work.join("update.dmg");
    run(Command::new("/usr/bin/curl")
        .args(["-fsSL", "--max-time", "600", "-o"])
        .arg(&dmg)
        .arg(&release.url))
    .context("download failed")?;

    let digest: String = Sha256::digest(fs::read(&dmg)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    ensure!(
        digest.eq_ignore_ascii_case(release.sha256.trim()),
        "downloaded file is corrupted (checksum mismatch)"
    );

    let mount = work.join("mnt");
    fs::create_dir_all(&mount)?;
    run(Command::new("/usr/bin/hdiutil")
        .args([
            "attach",
            "-nobrowse",
            "-readonly",
            "-noautoopen",
            "-mountpoint",
        ])
        .arg(&mount)
        .arg(&dmg))
    .context("couldn't open the downloaded disk image")?;
    let staged = parent.join(format!(".RunReCap-{}.app", release.version));
    let _ = fs::remove_dir_all(&staged);
    let copied = run(Command::new("/usr/bin/ditto")
        .arg(mount.join("RunReCap.app"))
        .arg(&staged));
    let _ = run(Command::new("/usr/bin/hdiutil")
        .args(["detach", "-quiet", "-force"])
        .arg(&mount));
    copied.with_context(|| format!("couldn't write to {} (permission?)", parent.display()))?;

    let verify = (|| -> Result<()> {
        run(Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&staged))
        .context("the new app's code signature is invalid")?;
        ensure!(
            plist(&staged, "CFBundleIdentifier")? == plist(installed, "CFBundleIdentifier")?,
            "the downloaded app is not RunReCap"
        );
        ensure!(
            plist(&staged, "CFBundleShortVersionString")? == release.version,
            "the downloaded app has the wrong version"
        );
        // Once releases are Developer ID signed, only accept the same team.
        if let Some(team) = team_id(installed) {
            ensure!(
                team_id(&staged).as_deref() == Some(&team),
                "the new app is signed by a different developer"
            );
        }
        Ok(())
    })();
    if let Err(e) = verify {
        let _ = fs::remove_dir_all(&staged);
        let _ = fs::remove_dir_all(&work);
        return Err(e);
    }
    Ok(Prepared {
        staged,
        installed: installed.to_path_buf(),
        work,
    })
}

impl Prepared {
    /// Start a detached helper that waits for this process to exit, swaps the
    /// app bundles (rolling back on failure) and optionally relaunches.
    /// The caller should quit right after this returns.
    pub fn launch_swap(self, relaunch: bool) -> Result<()> {
        let script = self.work.join("swap.sh");
        let mut f = fs::File::create(&script)?;
        f.write_all(
            br#"#!/bin/bash
# RunReCap update helper: $1=pid $2=staged app $3=installed app $4=relaunch(1/0) $5=work dir
while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
OLD="$3.old-$$"
if mv "$3" "$OLD"; then
  if mv "$2" "$3"; then rm -rf "$OLD"; else mv "$OLD" "$3"; fi
fi
xattr -dr com.apple.quarantine "$3" 2>/dev/null
[ "$4" = "1" ] && open "$3"
rm -rf "$5"
"#,
        )?;
        drop(f);
        Command::new("/bin/bash")
            .arg(&script)
            .arg(std::process::id().to_string())
            .arg(&self.staged)
            .arg(&self.installed)
            .arg(if relaunch { "1" } else { "0" })
            .arg(&self.work)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("couldn't start the update helper")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-beta", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn relative_urls_resolve_against_feed() {
        let feed = "https://github.com/me/runrecap/releases/latest/download/latest.json";
        assert_eq!(
            resolve(feed, "RunReCap-0.2.0.dmg"),
            "https://github.com/me/runrecap/releases/latest/download/RunReCap-0.2.0.dmg"
        );
        assert_eq!(
            resolve(feed, "https://cdn.example.com/a.dmg"),
            "https://cdn.example.com/a.dmg"
        );
    }

    #[test]
    fn feed_parses() {
        let r: Release =
            serde_json::from_str(r#"{"version":"0.2.0","url":"a.dmg","sha256":"ab"}"#).unwrap();
        assert_eq!(r.notes, None);
    }
}
