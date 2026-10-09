//! Smoke test: every layout × built-in theme renders the sample runs with
//! visible route pixels and without warnings.

use std::path::Path;

use recap_core::{gpx, Activity};
use recap_render::{builtin_themes, render, Canvas, Fonts, Layout, Scene};

fn sample(name: &str) -> Activity {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/test-gpx")
        .join(name);
    let parsed = gpx::parse(&std::fs::read(path).unwrap()).unwrap();
    Activity::from_points(name.into(), parsed.name.unwrap(), None, &parsed.points).unwrap()
}

#[test]
fn all_layouts_and_themes_render() {
    let runs = [
        sample("lake-gardens-loop.gpx"),
        sample("figure-eight-tempo.gpx"),
        sample("sunday-long-run.gpx"),
    ];
    let all: Vec<&Activity> = runs.iter().collect();
    let mut fonts = Fonts::new();
    for theme in builtin_themes() {
        for layout in Layout::ALL {
            let scene = Scene {
                layout,
                current: &runs[0],
                all: &all,
                privacy_trim_m: 200.0,
                utc_offset_s: 8 * 3600,
                show_caption: true,
            };
            let (pixmap, notes) = render(
                &scene,
                &theme,
                Canvas::new(800, 500),
                &mut fonts,
                Path::new("."),
            )
            .unwrap();
            assert!(notes.warnings.is_empty(), "{:?}", notes.warnings);
            let bg = pixmap.pixel(0, 0).unwrap();
            let changed = pixmap.pixels().iter().filter(|p| **p != bg).count();
            assert!(
                changed > 2_000,
                "{} / {:?}: only {changed} pixels drawn",
                theme.name,
                layout
            );
        }
    }
}

#[test]
fn missing_font_falls_back_to_inter_with_warning() {
    let run = sample("lake-gardens-loop.gpx");
    let mut theme = builtin_themes().remove(0);
    theme.text.font.bundled = None;
    theme.text.font.file = Some("/nonexistent/font.otf".into());
    let all = [&run];
    let scene = Scene {
        layout: Layout::Single,
        current: &run,
        all: &all,
        privacy_trim_m: 0.0,
        utc_offset_s: 0,
        show_caption: true,
    };
    let (_, notes) = render(
        &scene,
        &theme,
        Canvas::new(400, 300),
        &mut Fonts::new(),
        Path::new("."),
    )
    .unwrap();
    assert_eq!(notes.warnings.len(), 1);
    assert!(notes.warnings[0].contains("Inter"));
}
