//! Theme model (fonts, colors, layout of the caption) and the built-in themes
//! derived from `Design.md` (see AGENTS.md §6.2).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::color::Rgba;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default)]
    pub text: TextStyle,
    #[serde(default)]
    pub route: RouteStyle,
    #[serde(default)]
    pub background: Background,
}

/// Which font to use. Exactly one of `bundled` / `system` / `file` should be set;
/// if none resolves, the renderer falls back to bundled Inter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundled: Option<String>,
    /// Family name of an installed font, e.g. "Avenir Next".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Path to a .ttf/.otf/.ttc file (relative paths resolve against the app data dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    #[serde(default = "default_weight")]
    pub weight: u16,
}

fn default_weight() -> u16 {
    600
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            bundled: Some("Inter".into()),
            system: None,
            file: None,
            weight: 600,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    BottomLeft,
    BottomCenter,
    BottomRight,
    TopLeft,
    TopRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Name,
    Date,
    Distance,
    Time,
    Pace,
    Elevation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    #[default]
    Km,
    Mi,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextStyle {
    pub show: bool,
    pub font: FontSpec,
    /// Title size as % of screen height. Stats/date lines are derived from it.
    pub size_pct: f32,
    pub color: Rgba,
    /// Extra spacing in em; Design.md asks for slight negative tracking.
    pub letter_spacing: f32,
    pub uppercase: bool,
    pub position: Position,
    pub fields: Vec<Field>,
    pub units: Units,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            show: true,
            font: FontSpec::default(),
            size_pct: 2.2,
            color: Rgba::hex(0xf5f5f7),
            letter_spacing: -0.01,
            uppercase: false,
            position: Position::BottomLeft,
            fields: vec![
                Field::Name,
                Field::Distance,
                Field::Time,
                Field::Pace,
                Field::Date,
            ],
            units: Units::Km,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    Solid,
    /// Start → finish along the route.
    Gradient,
    Pace,
    Elevation,
    HeartRate,
    TimeOfDay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glow {
    pub color: Rgba,
    /// Glow radius as % of screen height.
    pub radius_pct: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RouteStyle {
    pub mode: ColorMode,
    pub color: Rgba,
    /// Stops for gradient and data-driven modes (low → high / start → end).
    pub gradient: Vec<Rgba>,
    /// Stroke width as % of screen height.
    pub width_pct: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glow: Option<Glow>,
    pub markers: bool,
}

impl Default for RouteStyle {
    fn default() -> Self {
        Self {
            mode: ColorMode::Solid,
            color: Rgba::hex(0x2997ff),
            gradient: vec![Rgba::hex(0x2997ff), Rgba::hex(0xf5f5f7)],
            width_pct: 0.35,
            glow: Some(Glow {
                color: Rgba::hex(0x2997ff).with_alpha(0.25),
                radius_pct: 0.6,
            }),
            markers: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Background {
    Solid {
        color: Rgba,
    },
    /// Top → bottom.
    Gradient {
        colors: Vec<Rgba>,
    },
    /// PNG image, aspect-filled. `dim` darkens it (0–1) so the route stays readable.
    Image {
        path: PathBuf,
        #[serde(default)]
        dim: f32,
    },
}

impl Default for Background {
    fn default() -> Self {
        Self::Solid {
            color: Rgba::hex(0x000000),
        }
    }
}

impl Background {
    /// Representative color, used for contrast checks and blend-mode choice.
    pub fn base_color(&self) -> Rgba {
        match self {
            Self::Solid { color } => *color,
            Self::Gradient { colors } => crate::color::sample(colors, 0.5),
            Self::Image { .. } => Rgba::hex(0x000000),
        }
    }
}

impl Theme {
    /// Lowest text contrast against the background; < 4.5 fails WCAG AA.
    pub fn text_contrast(&self) -> f32 {
        self.text.color.contrast(self.background.base_color())
    }
}

pub const APPLE_DARK: &str = "Apple Dark";
pub const APPLE_LIGHT: &str = "Apple Light";
pub const MONO_DARK: &str = "Mono Dark";

/// Built-in themes from Design.md tokens. "Apple Dark" is the default.
pub fn builtin_themes() -> Vec<Theme> {
    let dark_text = Rgba::hex(0xf5f5f7);
    let light_text = Rgba::hex(0x1d1d1f);
    vec![
        Theme {
            name: APPLE_DARK.into(),
            text: TextStyle::default(),
            route: RouteStyle::default(),
            background: Background::Solid {
                color: Rgba::hex(0x000000),
            },
        },
        Theme {
            name: APPLE_LIGHT.into(),
            text: TextStyle {
                color: light_text,
                ..TextStyle::default()
            },
            route: RouteStyle {
                color: light_text,
                gradient: vec![Rgba::hex(0x0071e3), light_text],
                glow: None,
                ..RouteStyle::default()
            },
            background: Background::Solid {
                color: Rgba::hex(0xf5f5f7),
            },
        },
        Theme {
            name: MONO_DARK.into(),
            text: TextStyle {
                color: dark_text,
                ..TextStyle::default()
            },
            route: RouteStyle {
                color: dark_text,
                gradient: vec![Rgba::hex(0x86868b), dark_text],
                glow: Some(Glow {
                    color: dark_text.with_alpha(0.15),
                    radius_pct: 0.5,
                }),
                ..RouteStyle::default()
            },
            background: Background::Solid {
                color: Rgba::hex(0x000000),
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_meet_wcag_aa() {
        for t in builtin_themes() {
            assert!(
                t.text_contrast() >= 4.5,
                "{} contrast {}",
                t.name,
                t.text_contrast()
            );
        }
    }

    #[test]
    fn theme_roundtrips_through_toml_shape() {
        let t = &builtin_themes()[0];
        let json = serde_json::to_string(t).unwrap();
        let back: Theme = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, t);
    }
}
