//! Renders the 1024×1024 app icon from a sample run, using Design.md colors:
//! `cargo run -p recap-render --example icon -- packaging/AppIcon.png`

use std::path::Path;

use recap_core::{geo, gpx};
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Paint, PathBuilder, Pixmap,
    Rect, Shader, SpreadMode, Stroke, Transform,
};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "AppIcon.png".into());
    let gpx_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/test-gpx/figure-eight-tempo.gpx");
    let track = gpx::parse(&std::fs::read(gpx_path).expect("sample gpx"))
        .expect("parse")
        .points;
    let track = geo::simplify(&track, 3.0);

    let mut pm = Pixmap::new(1024, 1024).unwrap();

    // macOS icon grid: 824pt rounded square centred in 1024, radius ≈ 22.5%.
    let (inset, side) = (100.0, 824.0);
    let shape = rounded_rect(inset, inset, side, side, 185.0);
    let bg = LinearGradient::new(
        (0.0, inset).into(),
        (0.0, inset + side).into(),
        vec![
            GradientStop::new(0.0, rgb(0x1d1d1f)),
            GradientStop::new(1.0, rgb(0x000000)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap();
    pm.fill_path(
        &shape,
        &Paint {
            shader: bg,
            anti_alias: true,
            ..Default::default()
        },
        FillRule::Winding,
        Transform::identity(),
        None,
    );

    // Fit the route into the middle of the tile.
    let xy: Vec<(f64, f64)> = track.iter().map(geo::mercator).collect();
    let (mut min, mut max) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for (x, y) in &xy {
        min = (min.0.min(*x), min.1.min(*y));
        max = (max.0.max(*x), max.1.max(*y));
    }
    let area = Rect::from_xywh(inset + 130.0, inset + 130.0, side - 260.0, side - 260.0).unwrap();
    let scale = (area.width() as f64 / (max.0 - min.0)).min(area.height() as f64 / (max.1 - min.1));
    let ox = area.left() as f64 + (area.width() as f64 - (max.0 - min.0) * scale) / 2.0;
    let oy = area.top() as f64 + (area.height() as f64 - (max.1 - min.1) * scale) / 2.0;
    let pts: Vec<(f32, f32)> = xy
        .iter()
        .map(|(x, y)| {
            (
                (ox + (x - min.0) * scale) as f32,
                (oy + (y - min.1) * scale) as f32,
            )
        })
        .collect();
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    let route = pb.finish().unwrap();

    let stroke = |w: f32| Stroke {
        width: w,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    for (w, a) in [(110.0, 0.05), (80.0, 0.08), (56.0, 0.12), (40.0, 0.18)] {
        let mut glow = Paint {
            anti_alias: true,
            ..Default::default()
        };
        glow.set_color(Color::from_rgba8(0x29, 0x97, 0xff, (a * 255.0) as u8));
        pm.stroke_path(&route, &glow, &stroke(w), Transform::identity(), None);
    }
    let line = Paint {
        shader: Shader::SolidColor(rgb(0x2997ff)),
        anti_alias: true,
        ..Default::default()
    };
    pm.stroke_path(&route, &line, &stroke(26.0), Transform::identity(), None);

    pm.save_png(&out).expect("write icon");
    println!("wrote {out}");
}

fn rgb(v: u32) -> Color {
    Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, 255)
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> tiny_skia::Path {
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish().unwrap()
}
