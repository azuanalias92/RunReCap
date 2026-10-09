//! Font resolution and text drawing (cosmic-text shaping, glyphs blended
//! straight into the tiny-skia pixmap).

use std::path::Path;

use cosmic_text::{
    fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight,
};
use tiny_skia::{Pixmap, PremultipliedColorU8};

use crate::color::Rgba;
use crate::theme::FontSpec;

const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");
pub const DEFAULT_FAMILY: &str = "Inter";

/// Owns the font database and glyph cache. Create once and reuse; system
/// fonts are only scanned the first time a `system` font is requested.
pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    system_loaded: bool,
}

/// A concrete font to draw with.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedFont {
    pub family: String,
    pub weight: u16,
    /// Set when the requested font was unavailable and Inter was used instead.
    pub fallback_reason: Option<String>,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_font_data(INTER_REGULAR.to_vec());
        db.load_font_data(INTER_SEMIBOLD.to_vec());
        db.set_sans_serif_family(DEFAULT_FAMILY);
        Self {
            system: FontSystem::new_with_locale_and_db("en-US".into(), db),
            cache: SwashCache::new(),
            system_loaded: false,
        }
    }

    fn has_family(&self, name: &str) -> Option<String> {
        self.system.db().faces().find_map(|f| {
            f.families
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(n, _)| n.clone())
        })
    }

    /// Resolve a theme font. `base_dir` resolves relative `file` paths.
    pub fn resolve(&mut self, spec: &FontSpec, base_dir: &Path) -> ResolvedFont {
        let weight = spec.weight.clamp(100, 900);
        let fallback = |reason: String| ResolvedFont {
            family: DEFAULT_FAMILY.into(),
            weight,
            fallback_reason: Some(reason),
        };

        if let Some(file) = &spec.file {
            let path = if file.is_absolute() {
                file.clone()
            } else {
                base_dir.join(file)
            };
            let before: Vec<_> = self.system.db().faces().map(|f| f.id).collect();
            if let Err(e) = self.system.db_mut().load_font_file(&path) {
                return fallback(format!("font file {} unavailable: {e}", path.display()));
            }
            let family = self
                .system
                .db()
                .faces()
                .find(|f| !before.contains(&f.id))
                .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
                // Already loaded earlier: find it by path.
                .or_else(|| {
                    self.system.db().faces().find_map(|f| match &f.source {
                        fontdb::Source::File(p) if *p == path => {
                            f.families.first().map(|(n, _)| n.clone())
                        }
                        _ => None,
                    })
                });
            return match family {
                Some(family) => ResolvedFont {
                    family,
                    weight,
                    fallback_reason: None,
                },
                None => fallback(format!("no usable face in {}", path.display())),
            };
        }

        if let Some(name) = &spec.system {
            if let Some(family) = self.has_family(name) {
                return ResolvedFont {
                    family,
                    weight,
                    fallback_reason: None,
                };
            }
            if !self.system_loaded {
                self.system.db_mut().load_system_fonts();
                self.system_loaded = true;
            }
            return match self.has_family(name) {
                Some(family) => ResolvedFont {
                    family,
                    weight,
                    fallback_reason: None,
                },
                None => fallback(format!("font \"{name}\" is not installed")),
            };
        }

        match &spec.bundled {
            Some(name) if !name.eq_ignore_ascii_case(DEFAULT_FAMILY) => {
                fallback(format!("bundled font \"{name}\" does not exist"))
            }
            _ => ResolvedFont {
                family: DEFAULT_FAMILY.into(),
                weight,
                fallback_reason: None,
            },
        }
    }

    fn layout(&mut self, text: &str, font: &ResolvedFont, size_px: f32) -> Buffer {
        let mut buffer = Buffer::new(&mut self.system, Metrics::new(size_px, size_px * 1.2));
        buffer.set_size(&mut self.system, None, None);
        let attrs = Attrs::new()
            .family(Family::Name(&font.family))
            .weight(Weight(font.weight));
        buffer.set_text(&mut self.system, text, attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.system, false);
        buffer
    }

    /// Width and line height of a single line of text.
    pub fn measure(
        &mut self,
        text: &str,
        font: &ResolvedFont,
        size_px: f32,
        tracking_em: f32,
    ) -> (f32, f32) {
        let buffer = self.layout(text, font, size_px);
        let mut w: f32 = 0.0;
        for run in buffer.layout_runs() {
            let extra = tracking_em * size_px * run.glyphs.len().saturating_sub(1) as f32;
            w = w.max(run.line_w + extra);
        }
        (w, size_px * 1.2)
    }

    /// Draw one line with its top-left corner at (x, y).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        font: &ResolvedFont,
        size_px: f32,
        tracking_em: f32,
        color: Rgba,
        x: f32,
        y: f32,
    ) {
        let buffer = self.layout(text, font, size_px);
        let base = cosmic_text::Color::rgba(color.r, color.g, color.b, color.a);
        let tracking = tracking_em * size_px;
        let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
        let pixels = pixmap.pixels_mut();
        for run in buffer.layout_runs() {
            for (i, glyph) in run.glyphs.iter().enumerate() {
                let phys = glyph.physical((x + i as f32 * tracking, y), 1.0);
                let line_y = run.line_y as i32;
                self.cache
                    .with_pixels(&mut self.system, phys.cache_key, base, |gx, gy, c| {
                        let px = phys.x + gx;
                        let py = line_y + phys.y + gy;
                        if px < 0 || py < 0 || px >= w || py >= h || c.a() == 0 {
                            return;
                        }
                        let idx = (py * w + px) as usize;
                        pixels[idx] = blend_over(pixels[idx], c);
                    });
            }
        }
    }
}

fn blend_over(dst: PremultipliedColorU8, src: cosmic_text::Color) -> PremultipliedColorU8 {
    let sa = src.a() as u32;
    let inv = 255 - sa;
    let ch = |s: u8, d: u8| ((s as u32 * sa + d as u32 * inv + 127) / 255) as u8;
    let a = (sa + (dst.alpha() as u32 * inv + 127) / 255).min(255) as u8;
    let r = ch(src.r(), dst.red()).min(a);
    let g = ch(src.g(), dst.green()).min(a);
    let b = ch(src.b(), dst.blue()).min(a);
    PremultipliedColorU8::from_rgba(r, g, b, a).unwrap_or(dst)
}
