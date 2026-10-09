//! Scene → pixels. Three layouts: a single run, a grid of runs, and a heatmap
//! of every run near the current one.

use std::path::Path;

use recap_core::{geo, Activity, Point};
use serde::{Deserialize, Serialize};
use tiny_skia::{
    BlendMode, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Rect, Shader,
    SpreadMode, Stroke, Transform,
};

use crate::caption;
use crate::color::{sample, Rgba};
use crate::text::Fonts;
use crate::theme::{Background, ColorMode, Position, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    #[default]
    Single,
    Grid,
    Heatmap,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Single, Layout::Grid, Layout::Heatmap];

    pub fn label(self) -> &'static str {
        match self {
            Layout::Single => "Single Run",
            Layout::Grid => "Grid",
            Layout::Heatmap => "Heatmap",
        }
    }
}

/// Pixel area to draw into. `safe` keeps content clear of the menu bar,
/// notch and Dock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub safe: Insets,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            safe: Insets::default(),
        }
    }
}

pub struct Scene<'a> {
    pub layout: Layout,
    /// The run being featured.
    pub current: &'a Activity,
    /// Every eligible run (already filtered), newest first. Used by grid/heatmap.
    pub all: &'a [&'a Activity],
    /// Trim this many metres from each run's start and end (privacy).
    pub privacy_trim_m: f64,
    /// Local UTC offset for dates and time-of-day coloring.
    pub utc_offset_s: i32,
    pub show_caption: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("canvas size {0}x{1} is invalid")]
    BadSize(u32, u32),
}

/// Warnings that don't stop the render (shown in the app menu).
#[derive(Debug, Default, Clone)]
pub struct RenderNotes {
    pub warnings: Vec<String>,
}

pub fn render(
    scene: &Scene,
    theme: &Theme,
    canvas: Canvas,
    fonts: &mut Fonts,
    data_dir: &Path,
) -> Result<(Pixmap, RenderNotes), RenderError> {
    let mut pixmap = Pixmap::new(canvas.width, canvas.height)
        .ok_or(RenderError::BadSize(canvas.width, canvas.height))?;
    let mut notes = RenderNotes::default();
    paint_background(&mut pixmap, &theme.background, data_dir, &mut notes);

    let h = canvas.height as f32;
    let margin = 0.06 * canvas.width.min(canvas.height) as f32;
    let area = Rect::from_ltrb(
        canvas.safe.left + margin,
        canvas.safe.top + margin,
        canvas.width as f32 - canvas.safe.right - margin,
        canvas.height as f32 - canvas.safe.bottom - margin,
    )
    .unwrap_or_else(|| Rect::from_xywh(0.0, 0.0, canvas.width as f32, h).unwrap());

    // Reserve a band for the caption and keep routes out of it.
    let mut route_area = area;
    if scene.show_caption && theme.text.show {
        let font = fonts.resolve(&theme.text.font, data_dir);
        if let Some(reason) = &font.fallback_reason {
            notes.warnings.push(format!("Using Inter: {reason}"));
        }
        let lines = caption::lines(scene, &theme.text);
        let block = caption::block_height(&lines, &theme.text, h);
        let gap = block + 0.04 * h;
        route_area = match theme.text.position {
            Position::TopLeft | Position::TopRight => {
                Rect::from_ltrb(area.left(), area.top() + gap, area.right(), area.bottom())
            }
            _ => Rect::from_ltrb(area.left(), area.top(), area.right(), area.bottom() - gap),
        }
        .unwrap_or(area);
        caption::draw(&mut pixmap, fonts, &font, &lines, &theme.text, area, h);
    }

    let width = (theme.route.width_pct / 100.0 * h).max(1.0);
    match scene.layout {
        Layout::Single => {
            let pts = scene.current.trimmed_points(scene.privacy_trim_m);
            let fit = Fit::new(&[pts], route_area);
            draw_route(&mut pixmap, pts, &fit, theme, scene, width, h, true);
        }
        Layout::Grid => draw_grid(&mut pixmap, scene, theme, route_area, width, h),
        Layout::Heatmap => draw_heatmap(&mut pixmap, scene, theme, route_area, width),
    }
    Ok((pixmap, notes))
}

fn paint_background(
    pixmap: &mut Pixmap,
    bg: &Background,
    data_dir: &Path,
    notes: &mut RenderNotes,
) {
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    match bg {
        Background::Solid { color } => pixmap.fill(color.to_skia()),
        Background::Gradient { colors } => {
            let stops: Vec<_> = colors
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    GradientStop::new(i as f32 / (colors.len().max(2) - 1) as f32, c.to_skia())
                })
                .collect();
            let shader = LinearGradient::new(
                (0.0, 0.0).into(),
                (0.0, h).into(),
                stops,
                SpreadMode::Pad,
                Transform::identity(),
            );
            match shader {
                Some(shader) => {
                    let paint = Paint {
                        shader,
                        ..Paint::default()
                    };
                    pixmap.fill_rect(
                        Rect::from_xywh(0.0, 0.0, w, h).unwrap(),
                        &paint,
                        Transform::identity(),
                        None,
                    );
                }
                None => pixmap.fill(colors.first().copied().unwrap_or(Rgba::hex(0)).to_skia()),
            }
        }
        Background::Image { path, dim } => {
            pixmap.fill(tiny_skia::Color::BLACK);
            let path = if path.is_absolute() {
                path.clone()
            } else {
                data_dir.join(path)
            };
            match Pixmap::load_png(&path) {
                Ok(img) => {
                    let s = (w / img.width() as f32).max(h / img.height() as f32);
                    let tx = (w - img.width() as f32 * s) / 2.0;
                    let ty = (h - img.height() as f32 * s) / 2.0;
                    pixmap.draw_pixmap(
                        0,
                        0,
                        img.as_ref(),
                        &tiny_skia::PixmapPaint {
                            quality: tiny_skia::FilterQuality::Bicubic,
                            ..Default::default()
                        },
                        Transform::from_row(s, 0.0, 0.0, s, tx, ty),
                        None,
                    );
                    if *dim > 0.0 {
                        let mut paint = Paint::default();
                        paint.set_color(Rgba::hex(0).with_alpha(*dim).to_skia());
                        pixmap.fill_rect(
                            Rect::from_xywh(0.0, 0.0, w, h).unwrap(),
                            &paint,
                            Transform::identity(),
                            None,
                        );
                    }
                }
                Err(e) => notes.warnings.push(format!(
                    "Background image {} unreadable ({e}); PNG only",
                    path.display()
                )),
            }
        }
    }
}

/// Maps projected track coordinates into a rectangle, preserving aspect.
struct Fit {
    min: (f64, f64),
    scale: f64,
    offset: (f64, f64),
}

impl Fit {
    fn new(tracks: &[&[Point]], area: Rect) -> Self {
        let mut min = (f64::MAX, f64::MAX);
        let mut max = (f64::MIN, f64::MIN);
        for p in tracks.iter().flat_map(|t| t.iter()) {
            let (x, y) = geo::mercator(p);
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        Self::from_bounds(min, max, area)
    }

    fn from_bounds(min: (f64, f64), max: (f64, f64), area: Rect) -> Self {
        let span = ((max.0 - min.0).max(1e-9), (max.1 - min.1).max(1e-9));
        let scale = (area.width() as f64 / span.0).min(area.height() as f64 / span.1);
        let offset = (
            area.left() as f64 + (area.width() as f64 - span.0 * scale) / 2.0,
            area.top() as f64 + (area.height() as f64 - span.1 * scale) / 2.0,
        );
        Self { min, scale, offset }
    }

    fn map(&self, p: &Point) -> (f32, f32) {
        let (x, y) = geo::mercator(p);
        (
            (self.offset.0 + (x - self.min.0) * self.scale) as f32,
            (self.offset.1 + (y - self.min.1) * self.scale) as f32,
        )
    }
}

fn stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Stroke::default()
    }
}

fn paint(color: Rgba, blend: BlendMode) -> Paint<'static> {
    Paint {
        shader: Shader::SolidColor(color.to_skia()),
        anti_alias: true,
        blend_mode: blend,
        ..Paint::default()
    }
}

fn polyline(pts: &[(f32, f32)]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = pts.split_first()?;
    pb.move_to(first.0, first.1);
    for p in rest {
        pb.line_to(p.0, p.1);
    }
    pb.finish()
}

/// Per-point value in 0..=1 for data-driven color modes; None = fall back to solid.
fn values(points: &[Point], mode: ColorMode, utc_offset_s: i32) -> Option<Vec<f32>> {
    let n = points.len();
    let norm = |raw: Vec<Option<f64>>| -> Option<Vec<f32>> {
        let mut known: Vec<f64> = raw.iter().flatten().copied().collect();
        if known.len() < 2 {
            return None;
        }
        known.sort_by(|a, b| a.total_cmp(b));
        let lo = known[known.len() * 5 / 100];
        let hi = known[(known.len() * 95 / 100).min(known.len() - 1)];
        let span = (hi - lo).max(1e-9);
        let mut last = 0.5;
        Some(
            raw.into_iter()
                .map(|v| {
                    if let Some(v) = v {
                        last = ((v - lo) / span).clamp(0.0, 1.0) as f32;
                    }
                    last
                })
                .collect(),
        )
    };
    match mode {
        ColorMode::Solid => None,
        ColorMode::Gradient => Some((0..n).map(|i| i as f32 / (n.max(2) - 1) as f32).collect()),
        ColorMode::Elevation => norm(points.iter().map(|p| p.ele.map(f64::from)).collect()),
        ColorMode::HeartRate => norm(points.iter().map(|p| p.hr.map(f64::from)).collect()),
        ColorMode::Pace => {
            // Speed between neighbours, smoothed over ±3 points. Faster = higher.
            let speed: Vec<Option<f64>> = (0..n)
                .map(|i| {
                    let (a, b) = (&points[i.saturating_sub(3)], &points[(i + 3).min(n - 1)]);
                    match (a.t, b.t) {
                        (Some(ta), Some(tb)) if tb > ta => {
                            let d = geo::path_length_m(
                                &points[i.saturating_sub(3)..=(i + 3).min(n - 1)],
                            );
                            Some(d / (tb - ta) as f64)
                        }
                        _ => None,
                    }
                })
                .collect();
            norm(speed)
        }
        ColorMode::TimeOfDay => {
            let t = points.iter().find_map(|p| p.t)? + utc_offset_s as i64;
            let hour = t.rem_euclid(86_400) as f32 / 86_400.0;
            Some(vec![hour; n])
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_route(
    pixmap: &mut Pixmap,
    points: &[Point],
    fit: &Fit,
    theme: &Theme,
    scene: &Scene,
    width: f32,
    h: f32,
    markers: bool,
) {
    let style = &theme.route;
    let xy: Vec<(f32, f32)> = points.iter().map(|p| fit.map(p)).collect();
    let Some(path) = polyline(&xy) else { return };

    if let Some(glow) = &style.glow {
        let radius = glow.radius_pct / 100.0 * h;
        // Fake blur: a few widening, fading strokes.
        for k in (1..=4).rev() {
            let w = width + radius * 2.0 * k as f32 / 4.0;
            let p = paint(glow.color.scale_alpha(0.35), BlendMode::SourceOver);
            pixmap.stroke_path(&path, &p, &stroke(w), Transform::identity(), None);
        }
    }

    match values(points, style.mode, scene.utc_offset_s) {
        None => {
            pixmap.stroke_path(
                &path,
                &paint(style.color, BlendMode::SourceOver),
                &stroke(width),
                Transform::identity(),
                None,
            );
        }
        Some(vals) => {
            // Batch consecutive segments that quantize to the same color.
            let q = |v: f32| (v * 48.0).round() as i32;
            let mut start = 0;
            for i in 1..=xy.len() {
                let boundary = i == xy.len() || q(vals[i]) != q(vals[start]);
                if boundary && i - start >= 1 {
                    let end = i.min(xy.len() - 1);
                    if let Some(seg) = polyline(&xy[start..=end]) {
                        let c = sample(&style.gradient, vals[start]);
                        pixmap.stroke_path(
                            &seg,
                            &paint(c, BlendMode::SourceOver),
                            &stroke(width),
                            Transform::identity(),
                            None,
                        );
                    }
                    start = end;
                }
            }
        }
    }

    if markers && style.markers && xy.len() > 1 {
        let r = width * 1.6;
        let start_c = match style.mode {
            ColorMode::Solid => style.color,
            _ => sample(&style.gradient, 0.0),
        };
        dot(pixmap, xy[0], r, start_c);
        let end = xy[xy.len() - 1];
        dot(pixmap, end, r, theme.text.color);
        dot(pixmap, end, r * 0.5, theme.background.base_color());
    }
}

fn dot(pixmap: &mut Pixmap, (x, y): (f32, f32), r: f32, color: Rgba) {
    if let Some(c) = PathBuilder::from_circle(x, y, r) {
        pixmap.fill_path(
            &c,
            &paint(color, BlendMode::SourceOver),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

fn draw_grid(pixmap: &mut Pixmap, scene: &Scene, theme: &Theme, area: Rect, width: f32, h: f32) {
    const MAX_CELLS: usize = 240;
    let runs: Vec<&Activity> = scene.all.iter().copied().take(MAX_CELLS).collect();
    let n = runs.len().max(1);
    // Choose the column count that gives the largest square cells.
    let (mut cols, mut cell) = (1, 0.0f32);
    for c in 1..=n {
        let rows = n.div_ceil(c);
        let size = (area.width() / c as f32).min(area.height() / rows as f32);
        if size > cell {
            cell = size;
            cols = c;
        }
    }
    let rows = n.div_ceil(cols);
    let ox = area.left() + (area.width() - cell * cols as f32) / 2.0;
    let oy = area.top() + (area.height() - cell * rows as f32) / 2.0;
    let stroke_w = (width * (cell / (0.5 * h)).sqrt()).clamp(1.0, width);
    let mut dim_theme = theme.clone();
    dim_theme.route.glow = None;
    for (i, run) in runs.iter().enumerate() {
        let (r, c) = (i / cols, i % cols);
        let pad = cell * 0.12;
        let Some(rect) = Rect::from_xywh(
            ox + c as f32 * cell + pad,
            oy + r as f32 * cell + pad,
            cell - 2.0 * pad,
            cell - 2.0 * pad,
        ) else {
            continue;
        };
        let pts = run.trimmed_points(scene.privacy_trim_m);
        let fit = Fit::new(&[pts], rect);
        let is_current = run.id == scene.current.id;
        let t = if is_current { theme } else { &dim_theme };
        draw_route(pixmap, pts, &fit, t, scene, stroke_w, h, false);
    }
}

fn draw_heatmap(pixmap: &mut Pixmap, scene: &Scene, theme: &Theme, area: Rect, width: f32) {
    const RADIUS_M: f64 = 30_000.0;
    let Some(anchor) = scene.current.points.first() else {
        return;
    };
    let runs: Vec<&[Point]> = scene
        .all
        .iter()
        .filter(|a| {
            a.points
                .first()
                .is_some_and(|p| geo::haversine_m(anchor, p) < RADIUS_M)
        })
        .map(|a| a.trimmed_points(scene.privacy_trim_m))
        .collect();

    // Bounds from the 1st–99th percentile so one stray run doesn't shrink everything.
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for p in runs.iter().flat_map(|r| r.iter()) {
        let (x, y) = geo::mercator(p);
        xs.push(x);
        ys.push(y);
    }
    if xs.is_empty() {
        return;
    }
    xs.sort_by(|a, b| a.total_cmp(b));
    ys.sort_by(|a, b| a.total_cmp(b));
    let pct = |v: &[f64], q: f64| v[((v.len() - 1) as f64 * q) as usize];
    let fit = Fit::from_bounds(
        (pct(&xs, 0.01), pct(&ys, 0.01)),
        (pct(&xs, 0.99), pct(&ys, 0.99)),
        area,
    );

    let dark = theme.background.base_color().luminance() < 0.5;
    let blend = if dark {
        BlendMode::Plus
    } else {
        BlendMode::SourceOver
    };
    let alpha = (1.6 / (runs.len() as f32).sqrt()).clamp(0.08, 0.9);
    let color = theme.route.color.scale_alpha(alpha);
    let w = width * 0.7;
    for run in runs {
        let xy: Vec<_> = run.iter().map(|p| fit.map(p)).collect();
        if let Some(path) = polyline(&xy) {
            pixmap.stroke_path(
                &path,
                &paint(color, blend),
                &stroke(w),
                Transform::identity(),
                None,
            );
        }
    }
}
