use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// sRGB color with straight (non-premultiplied) alpha. Serialized as hex:
/// `#RRGGBB` or `#RRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn hex(v: u32) -> Self {
        Self::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self {
            a: (a.clamp(0.0, 1.0) * 255.0).round() as u8,
            ..self
        }
    }

    pub fn scale_alpha(self, k: f32) -> Self {
        Self {
            a: (self.a as f32 * k).clamp(0.0, 255.0).round() as u8,
            ..self
        }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self {
            r: m(self.r, other.r),
            g: m(self.g, other.g),
            b: m(self.b, other.b),
            a: m(self.a, other.a),
        }
    }

    pub fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    /// WCAG relative luminance.
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// WCAG contrast ratio (1.0–21.0), ignoring alpha.
    pub fn contrast(self, other: Self) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

/// Sample a multi-stop gradient at `t` in 0..=1.
pub fn sample(stops: &[Rgba], t: f32) -> Rgba {
    match stops {
        [] => Rgba::rgb(255, 255, 255),
        [only] => *only,
        _ => {
            let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
            let i = (t.floor() as usize).min(stops.len() - 2);
            stops[i].lerp(stops[i + 1], t - i as f32)
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid color {0:?} (expected #RRGGBB or #RRGGBBAA)")]
pub struct ParseColorError(String);

impl FromStr for Rgba {
    type Err = ParseColorError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseColorError(s.to_owned());
        let h = s.trim().strip_prefix('#').ok_or_else(err)?;
        let byte =
            |i: usize| u8::from_str_radix(h.get(i..i + 2).ok_or_else(err)?, 16).map_err(|_| err());
        match h.len() {
            6 => Ok(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Ok(Self {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => Err(err()),
        }
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

impl Serialize for Rgba {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display_roundtrip() {
        let c: Rgba = "#2997ff40".parse().unwrap();
        assert_eq!(
            c,
            Rgba {
                r: 0x29,
                g: 0x97,
                b: 0xff,
                a: 0x40
            }
        );
        assert_eq!(c.to_string(), "#2997ff40");
        assert_eq!("#000000".parse::<Rgba>().unwrap().to_string(), "#000000");
        assert!("2997ff".parse::<Rgba>().is_err());
        assert!("#29".parse::<Rgba>().is_err());
    }

    #[test]
    fn contrast_black_white() {
        let r = Rgba::hex(0x000000).contrast(Rgba::hex(0xffffff));
        assert!((r - 21.0).abs() < 0.01);
    }

    #[test]
    fn gradient_sampling() {
        let s = [Rgba::hex(0x000000), Rgba::hex(0xffffff)];
        assert_eq!(sample(&s, 0.5), Rgba::rgb(128, 128, 128));
        assert_eq!(sample(&s, 2.0), Rgba::hex(0xffffff));
    }
}
