//! `config.toml` (user settings) and `state.json` (what the app is currently
//! showing). Both live in `~/Library/Application Support/RunReCap/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use recap_core::library::Filter;
use recap_render::theme::{builtin_themes, Background, Units, APPLE_DARK, APPLE_LIGHT};
use recap_render::{Layout, Theme};
use serde::{Deserialize, Serialize};

use crate::background::BackgroundImage;

/// Theme name meaning "Apple Dark/Light, following the system appearance".
pub const AUTOMATIC: &str = "Automatic";

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RUNRECAP_HOME") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("RunReCap")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    #[default]
    Newest,
    Shuffle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rotation {
    /// Minutes between automatic changes; 0 = never.
    pub every_minutes: u32,
    pub order: Order,
}

impl Default for Rotation {
    fn default() -> Self {
        Self {
            every_minutes: 60,
            order: Order::Newest,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    pub runs_only: bool,
    pub min_distance_km: f64,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            runs_only: true,
            min_distance_km: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub layout: Layout,
    /// A built-in or custom theme name, or "Automatic".
    pub theme: String,
    pub units: Units,
    /// Metres hidden at the start and end of each route.
    pub privacy_trim_m: f64,
    /// Release feed (`latest.json`) for update checks; overrides the URL baked
    /// in at build time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_url: Option<String>,
    pub rotation: Rotation,
    pub filter: FilterConfig,
    /// Uploaded photo the route is drawn on; overrides the theme background.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image: Option<BackgroundImage>,
    /// User-defined themes; same shape as the built-ins.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<Theme>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: Layout::Single,
            theme: AUTOMATIC.into(),
            units: Units::Km,
            privacy_trim_m: 200.0,
            rotation: Rotation::default(),
            filter: FilterConfig::default(),
            update_url: None,
            background_image: None,
            themes: Vec::new(),
        }
    }
}

const HEADER: &str =
    "# RunReCap settings. The app rewrites this file when you change options from the menu.\n\
# Themes: add [[themes]] tables (same fields as the built-ins: name, text, route, background).\n\n";

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("config.toml")
    }

    /// Load config, creating a default file on first run.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = Self::path(dir);
        if !path.exists() {
            let cfg = Self::default();
            cfg.save(dir)?;
            return Ok(cfg);
        }
        let text = fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let body = toml::to_string_pretty(self)?;
        fs::write(Self::path(dir), format!("{HEADER}{body}"))?;
        Ok(())
    }

    pub fn update_feed(&self) -> Option<String> {
        self.update_url
            .clone()
            .or_else(|| crate::update::BUILT_IN_FEED.map(str::to_owned))
            .filter(|u| !u.trim().is_empty())
    }

    pub fn filter(&self) -> Filter {
        Filter {
            runs_only: self.filter.runs_only,
            min_distance_m: self.filter.min_distance_km * 1000.0,
            since: None,
            until: None,
        }
    }

    /// Built-ins followed by custom themes (a custom theme with a built-in's
    /// name replaces it).
    pub fn all_themes(&self) -> Vec<Theme> {
        let mut themes: Vec<Theme> = builtin_themes()
            .into_iter()
            .filter(|b| !self.themes.iter().any(|t| t.name == b.name))
            .collect();
        themes.extend(self.themes.iter().cloned());
        themes
    }

    /// The theme to draw with right now.
    pub fn active_theme(&self, system_dark: bool) -> Theme {
        let themes = self.all_themes();
        let wanted = if self.theme == AUTOMATIC {
            // On a (dimmed) photo, light text reads best whatever the appearance.
            if system_dark || self.background_image.is_some() {
                APPLE_DARK
            } else {
                APPLE_LIGHT
            }
        } else {
            self.theme.as_str()
        };
        let mut theme = themes
            .iter()
            .find(|t| t.name == wanted)
            .or_else(|| themes.iter().find(|t| t.name == APPLE_DARK))
            .cloned()
            .unwrap_or_else(|| builtin_themes().remove(0));
        theme.text.units = self.units;
        if let Some(bg) = &self.background_image {
            theme.background = Background::Image {
                path: bg.file.clone(),
                dim: bg.dim as f32,
            };
        }
        theme
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub current_id: Option<String>,
    pub pinned: bool,
    /// True = preview only; the desktop picture is the user's own. New
    /// installs start here until the user clicks "Set as Wallpaper".
    pub paused: bool,
    /// Unix seconds of the last automatic or manual change.
    pub last_change: i64,
    /// The user's wallpaper before RunReCap, per screen id, restored on pause/quit.
    pub originals: BTreeMap<String, String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            current_id: None,
            pinned: false,
            paused: true,
            last_change: 0,
            originals: BTreeMap::new(),
        }
    }
}

impl State {
    fn path(dir: &Path) -> PathBuf {
        dir.join("state.json")
    }

    pub fn load(dir: &Path) -> Self {
        fs::read(Self::path(dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        fs::write(Self::path(dir), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips_through_toml() {
        let mut cfg = Config::default();
        cfg.themes.push(Theme {
            name: "Mine".into(),
            ..builtin_themes().remove(0)
        });
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn full_config_roundtrips_through_toml() {
        let cfg = Config {
            update_url: Some("https://example.com/latest.json".into()),
            background_image: Some(BackgroundImage {
                file: "backgrounds/ab.png".into(),
                name: "beach.jpg".into(),
                width: 4032,
                height: 3024,
                dim: 0.3,
            }),
            ..Config::default()
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), cfg);
    }

    #[test]
    fn photo_overrides_theme_background() {
        let cfg = Config {
            background_image: Some(BackgroundImage {
                file: "backgrounds/ab.png".into(),
                name: "x".into(),
                width: 1,
                height: 1,
                dim: 0.4,
            }),
            ..Config::default()
        };
        let t = cfg.active_theme(false);
        assert_eq!(t.name, APPLE_DARK, "light text on photos");
        assert_eq!(
            t.background,
            Background::Image {
                path: "backgrounds/ab.png".into(),
                dim: 0.4
            }
        );
    }

    #[test]
    fn automatic_follows_appearance() {
        let cfg = Config::default();
        assert_eq!(cfg.active_theme(true).name, APPLE_DARK);
        assert_eq!(cfg.active_theme(false).name, APPLE_LIGHT);
    }

    #[test]
    fn partial_config_uses_defaults() {
        let cfg: Config =
            toml::from_str("layout = \"grid\"\n[rotation]\nevery_minutes = 5\n").unwrap();
        assert_eq!(cfg.layout, Layout::Grid);
        assert_eq!(cfg.rotation.every_minutes, 5);
        assert_eq!(cfg.privacy_trim_m, 200.0);
    }
}
