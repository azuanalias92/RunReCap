//! The text block on the wallpaper: run name, stats and date, laid out on
//! the Design.md type scale (title 600, stats regular, date as caption).

use recap_core::Activity;
use time::{OffsetDateTime, UtcOffset};
use tiny_skia::{Pixmap, Rect};

use crate::render::{Layout, Scene};
use crate::text::{Fonts, ResolvedFont};
use crate::theme::{Field, Position, TextStyle, Units};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Title,
    Stats,
    Date,
}

impl Role {
    fn scale(self) -> f32 {
        match self {
            Role::Title => 1.0,
            Role::Stats => 0.56,
            Role::Date => 0.46,
        }
    }

    fn weight(self, base: u16) -> u16 {
        match self {
            Role::Title => base,
            _ => 400,
        }
    }

    fn opacity(self) -> f32 {
        match self {
            Role::Title => 1.0,
            Role::Stats => 0.78,
            Role::Date => 0.58,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Line {
    pub role: Role,
    pub text: String,
}

pub fn lines(scene: &Scene, style: &TextStyle) -> Vec<Line> {
    let mut out = Vec::new();
    let mut push = |role, text: String| {
        if !text.is_empty() {
            out.push(Line { role, text });
        }
    };
    match scene.layout {
        Layout::Single => {
            let a = scene.current;
            let mut stats = Vec::new();
            let mut date = None;
            let mut title = None;
            for f in &style.fields {
                match f {
                    Field::Name => title = Some(a.name.clone()),
                    Field::Date => date = a.start.map(|t| format_date(t, scene.utc_offset_s)),
                    Field::Distance => stats.push(distance(a.distance_m, style.units)),
                    Field::Time => stats.extend(a.duration_s.map(duration)),
                    Field::Pace => stats.extend(a.pace_s_per_km().map(|p| pace(p, style.units))),
                    Field::Elevation => {
                        stats.extend(a.elev_gain_m.map(|e| elevation(e, style.units)))
                    }
                }
            }
            push(Role::Title, title.unwrap_or_default());
            push(Role::Stats, stats.join("  ·  "));
            push(Role::Date, date.unwrap_or_default());
        }
        Layout::Grid | Layout::Heatmap => {
            let runs = scene.all;
            let total: f64 = runs.iter().map(|a| a.distance_m).sum();
            let label = if runs.len() == 1 { "run" } else { "runs" };
            push(Role::Title, format!("{} {label}", runs.len()));
            push(Role::Stats, distance(total, style.units));
            push(Role::Date, date_range(runs, scene.utc_offset_s));
        }
    }
    if style.uppercase {
        for l in &mut out {
            l.text = l.text.to_uppercase();
        }
    }
    out
}

fn size(role: Role, style: &TextStyle, canvas_h: f32) -> f32 {
    (style.size_pct / 100.0 * canvas_h * role.scale()).max(6.0)
}

fn gap(canvas_h: f32, style: &TextStyle) -> f32 {
    style.size_pct / 100.0 * canvas_h * 0.18
}

pub fn block_height(lines: &[Line], style: &TextStyle, canvas_h: f32) -> f32 {
    let body: f32 = lines
        .iter()
        .map(|l| size(l.role, style, canvas_h) * 1.2)
        .sum();
    body + gap(canvas_h, style) * lines.len().saturating_sub(1) as f32
}

pub fn draw(
    pixmap: &mut Pixmap,
    fonts: &mut Fonts,
    font: &ResolvedFont,
    lines: &[Line],
    style: &TextStyle,
    area: Rect,
    canvas_h: f32,
) {
    let total = block_height(lines, style, canvas_h);
    let mut y = match style.position {
        Position::TopLeft | Position::TopRight => area.top(),
        _ => area.bottom() - total,
    };
    for line in lines {
        let px = size(line.role, style, canvas_h);
        let f = ResolvedFont {
            weight: line.role.weight(font.weight),
            ..font.clone()
        };
        let (w, lh) = fonts.measure(&line.text, &f, px, style.letter_spacing);
        let x = match style.position {
            Position::BottomLeft | Position::TopLeft => area.left(),
            Position::BottomCenter => area.left() + (area.width() - w) / 2.0,
            Position::BottomRight | Position::TopRight => area.right() - w,
        };
        let color = style.color.scale_alpha(line.role.opacity());
        fonts.draw(
            pixmap,
            &line.text,
            &f,
            px,
            style.letter_spacing,
            color,
            x,
            y,
        );
        y += lh + gap(canvas_h, style);
    }
}

pub fn distance(m: f64, units: Units) -> String {
    match units {
        Units::Km => format!("{:.1} km", m / 1000.0),
        Units::Mi => format!("{:.1} mi", m / 1609.344),
    }
}

pub fn duration(s: i64) -> String {
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

pub fn pace(s_per_km: f64, units: Units) -> String {
    let (s, unit) = match units {
        Units::Km => (s_per_km, "/km"),
        Units::Mi => (s_per_km * 1.609344, "/mi"),
    };
    let s = s.round() as i64;
    format!("{}:{:02} {unit}", s / 60, s % 60)
}

pub fn elevation(m: f64, units: Units) -> String {
    match units {
        Units::Km => format!("+{:.0} m", m),
        Units::Mi => format!("+{:.0} ft", m * 3.28084),
    }
}

fn local(t: i64, offset_s: i32) -> Option<OffsetDateTime> {
    let off = UtcOffset::from_whole_seconds(offset_s).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::from_unix_timestamp(t)
        .ok()
        .map(|d| d.to_offset(off))
}

pub fn format_date(t: i64, offset_s: i32) -> String {
    let Some(d) = local(t, offset_s) else {
        return String::new();
    };
    let fmt = time::macros::format_description!(
        "[weekday repr:short], [day padding:none] [month repr:short] [year] · [hour repr:12 padding:none]:[minute] [period]"
    );
    d.format(&fmt).unwrap_or_default()
}

fn date_range(runs: &[&Activity], offset_s: i32) -> String {
    let fmt = time::macros::format_description!("[month repr:short] [year]");
    let starts = runs.iter().filter_map(|a| a.start);
    let (Some(lo), Some(hi)) = (starts.clone().min(), starts.max()) else {
        return String::new();
    };
    let (Some(lo), Some(hi)) = (local(lo, offset_s), local(hi, offset_s)) else {
        return String::new();
    };
    let (a, b) = (
        lo.format(&fmt).unwrap_or_default(),
        hi.format(&fmt).unwrap_or_default(),
    );
    if a == b {
        a
    } else {
        format!("{a} – {b}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(distance(10_240.0, Units::Km), "10.2 km");
        assert_eq!(duration(3125), "52:05");
        assert_eq!(duration(3725), "1:02:05");
        assert_eq!(pace(312.0, Units::Km), "5:12 /km");
        // 2026-10-03 22:12 UTC = Sun 4 Oct 06:12 in UTC+8.
        assert_eq!(
            format_date(1_791_065_520, 8 * 3600),
            "Sun, 4 Oct 2026 · 6:12 AM"
        );
    }
}
