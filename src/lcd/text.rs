//! Teko text rendering, matching upstream's `golang.org/x/image/font/opentype` face at 72 DPI
//! (font size = pixels per em, no hinting, baseline at the given point). Teko has no `kern`
//! table and no GPOS `kern` feature, so upstream applies no kerning and neither does this.
//! Both use the variable font's default instance (weight 300).

use ab_glyph::{point, Font, FontRef, GlyphId, PxScale};
use std::sync::OnceLock;

/// Teko (SIL Open Font License 1.1), as shipped by upstream in `static/fonts/teko.ttf`.
static FONT_BYTES: &[u8] = include_bytes!("../../assets/lcd/teko.ttf");

pub(crate) struct Typeface {
    font: FontRef<'static>,
    units_per_em: f32,
}

/// The parsed font, or `None` if the embedded file is unusable (text is then skipped).
pub(crate) fn typeface() -> Option<&'static Typeface> {
    static FACE: OnceLock<Option<Typeface>> = OnceLock::new();
    FACE.get_or_init(|| {
        let font = match FontRef::try_from_slice(FONT_BYTES) {
            Ok(f) => f,
            Err(e) => {
                log::error!("Unable to parse LCD font: {e}");
                return None;
            }
        };
        let units_per_em = font.units_per_em().unwrap_or(1000.0);
        Some(Typeface { font, units_per_em })
    })
    .as_ref()
}

impl Typeface {
    /// Glyphs of `text` with their pen x offsets in pixels (Go: font.Drawer advance).
    fn layout(&self, size: f32, text: &str) -> Vec<(GlyphId, f32)> {
        let s = size / self.units_per_em;
        let mut x = 0.0f32;
        text.chars()
            .map(|c| {
                let id = self.font.glyph_id(c);
                let at = x;
                x += self.font.h_advance_unscaled(id) * s;
                (id, at)
            })
            .collect()
    }

    /// Ink width and height of `text`, rounded up (Go: font.BoundString, then `.Ceil()`).
    pub(crate) fn bounds(&self, size: f32, text: &str) -> (i32, i32) {
        let s = size / self.units_per_em;
        let mut ink: Option<(f32, f32, f32, f32)> = None;
        for (id, x) in self.layout(size, text) {
            let Some(outline) = self.font.outline(id) else { continue };
            // Font units, y-up; ab_glyph stores yMax in `min.y` and yMin in `max.y`.
            // Go's bounds are y-down: (-yMax, -yMin).
            let b = outline.bounds;
            let (x0, x1, y0, y1) = (x + b.min.x * s, x + b.max.x * s, -b.min.y * s, -b.max.y * s);
            ink = Some(match ink {
                None => (x0, y0, x1, y1),
                Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
            });
        }
        ink.map_or((0, 0), |(x0, y0, x1, y1)| ((x1 - x0).ceil() as i32, (y1 - y0).ceil() as i32))
    }

    /// Rasterise `text` with its baseline origin at (`x`, `y`). `plot` receives each covered
    /// pixel and its coverage (0..=1), in drawing order.
    pub(crate) fn draw(&self, x: i32, y: i32, size: f32, text: &str, mut plot: impl FnMut(i32, i32, f32)) {
        let scale = PxScale::from(size * self.font.height_unscaled() / self.units_per_em);
        for (id, dx) in self.layout(size, text) {
            let glyph = id.with_scale_and_position(scale, point(x as f32 + dx, y as f32));
            let Some(outlined) = self.font.outline_glyph(glyph) else { continue };
            let b = outlined.px_bounds();
            let (bx, by) = (b.min.x as i32, b.min.y as i32);
            outlined.draw(|gx, gy, c| {
                if c > 0.0 {
                    plot(bx + gx as i32, by + gy as i32, c.min(1.0));
                }
            });
        }
    }
}
