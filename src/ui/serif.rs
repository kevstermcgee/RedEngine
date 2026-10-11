//! Book type for screens that are read rather than glanced at (a museum's labels, its map): the player's own serif typeface, drawn into an RGBA image.
//!
//! The engine ships no font files (ADR 0008: nothing imported). Every desktop already has a good serif: Georgia on Windows and macOS, Palatino's free
//! twin P052 or Noto, Liberation or DejaVu Serif on Linux. [`Serif::load`] takes the first it finds (`RE2_FONT=path` overrides), regular and italic, and
//! draws with `ab_glyph`; with none it falls back to the engine's 5x7 bitmap capitals, so text always appears. Text is laid out with kerning and wrapped
//! to a width by words.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};

/// Regular and italic faces (italic falls back to regular).
pub struct Serif {
    regular: Option<FontVec>,
    italic: Option<FontVec>,
    /// Which file the regular face came from (for the log), or `None` when drawing with the bitmap capitals.
    pub source: Option<String>,
}

/// A face to draw in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Regular,
    Italic,
}

const REGULAR: &[&str] = &[
    "C:\\Windows\\Fonts\\georgia.ttf",
    "C:\\Windows\\Fonts\\pala.ttf",
    "C:\\Windows\\Fonts\\times.ttf",
    "/System/Library/Fonts/Supplemental/Georgia.ttf",
    "/Library/Fonts/Georgia.ttf",
    "/System/Library/Fonts/Supplemental/Palatino.ttc",
    "/usr/share/fonts/opentype/urw-base35/P052-Roman.otf",
    "/usr/share/fonts/truetype/noto/NotoSerif-Regular.ttf",
    "/usr/share/fonts/noto/NotoSerif-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSerif-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
    "/usr/share/fonts/TTF/DejaVuSerif.ttf",
];

const ITALIC: &[&str] = &[
    "C:\\Windows\\Fonts\\georgiai.ttf",
    "C:\\Windows\\Fonts\\palai.ttf",
    "C:\\Windows\\Fonts\\timesi.ttf",
    "/System/Library/Fonts/Supplemental/Georgia Italic.ttf",
    "/Library/Fonts/Georgia Italic.ttf",
    "/usr/share/fonts/opentype/urw-base35/P052-Italic.otf",
    "/usr/share/fonts/truetype/noto/NotoSerif-Italic.ttf",
    "/usr/share/fonts/noto/NotoSerif-Italic.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSerif-Italic.ttf",
    "/usr/share/fonts/liberation/LiberationSerif-Italic.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Italic.ttf",
];

fn open(path: &str) -> Option<FontVec> {
    let bytes = std::fs::read(path).ok()?;
    FontVec::try_from_vec_and_index(bytes, 0).ok()
}

/// An RGBA image to draw into (row 0 at the top).
pub struct Image<'a> {
    pub w: u32,
    pub h: u32,
    pub px: &'a mut [u8],
}

impl Image<'_> {
    /// Blends `c` (straight alpha) over the pixel at `(x, y)` with coverage `k` (0..1).
    pub fn blend(&mut self, x: i32, y: i32, c: [u8; 4], k: f32) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        let a = (c[3] as f32 / 255.0) * k.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let d = &mut self.px[i..i + 4];
        for ch in 0..3 {
            d[ch] = (d[ch] as f32 * (1.0 - a) + c[ch] as f32 * a).round() as u8;
        }
        d[3] = (d[3] as f32 + (255.0 - d[3] as f32) * a).round() as u8;
    }

    /// A filled rectangle (blended).
    pub fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
        for y in y0.max(0)..y1.min(self.h as i32) {
            for x in x0.max(0)..x1.min(self.w as i32) {
                self.blend(x, y, c, 1.0);
            }
        }
    }

    /// A rectangle's outline `t` pixels thick.
    pub fn frame(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, t: i32, c: [u8; 4]) {
        self.rect(x0, y0, x1, y0 + t, c);
        self.rect(x0, y1 - t, x1, y1, c);
        self.rect(x0, y0 + t, x0 + t, y1 - t, c);
        self.rect(x1 - t, y0 + t, x1, y1 - t, c);
    }

    /// A filled disc, antialiased.
    pub fn disc(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
        for y in (cy - r - 1.0).floor() as i32..=(cy + r + 1.0).ceil() as i32 {
            for x in (cx - r - 1.0).floor() as i32..=(cx + r + 1.0).ceil() as i32 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                self.blend(x, y, c, (r - d + 0.5).clamp(0.0, 1.0));
            }
        }
    }

    /// A line `w` pixels wide, antialiased.
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), w: f32, c: [u8; 4]) {
        let (minx, maxx) = (a.0.min(b.0) - w - 1.0, a.0.max(b.0) + w + 1.0);
        let (miny, maxy) = (a.1.min(b.1) - w - 1.0, a.1.max(b.1) + w + 1.0);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for y in miny.floor() as i32..=maxy.ceil() as i32 {
            for x in minx.floor() as i32..=maxx.ceil() as i32 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0);
                let d = ((px - a.0 - dx * t).powi(2) + (py - a.1 - dy * t).powi(2)).sqrt();
                self.blend(x, y, c, (w * 0.5 - d + 0.5).clamp(0.0, 1.0));
            }
        }
    }
}

impl Serif {
    /// The first serif face found on this machine (see the module docs).
    pub fn load() -> Serif {
        let env = std::env::var("RE2_FONT").ok();
        let mut regular = None;
        let mut source = None;
        for p in env.iter().map(String::as_str).chain(REGULAR.iter().copied()) {
            if let Some(f) = open(p) {
                regular = Some(f);
                source = Some(p.to_string());
                break;
            }
        }
        let italic = ITALIC.iter().find_map(|p| open(p));
        Serif { regular, italic, source }
    }

    /// No system font: only the bitmap capitals (tests use this for pixels that do not depend on the machine).
    pub fn bitmap_only() -> Serif {
        Serif { regular: None, italic: None, source: None }
    }

    fn face(&self, f: Face) -> Option<&FontVec> {
        match f {
            Face::Italic => self.italic.as_ref().or(self.regular.as_ref()),
            Face::Regular => self.regular.as_ref(),
        }
    }

    /// The width of `text` at `size` pixels.
    pub fn measure(&self, text: &str, size: f32, f: Face) -> f32 {
        match self.face(f) {
            Some(font) => {
                let sf = font.as_scaled(PxScale::from(size));
                let mut w = 0.0;
                let mut prev = None;
                for ch in text.chars() {
                    let id = sf.glyph_id(ch);
                    if let Some(p) = prev {
                        w += sf.kern(p, id);
                    }
                    w += sf.h_advance(id);
                    prev = Some(id);
                }
                w
            }
            None => {
                let scale = bitmap_scale(size);
                crate::ui::text_width(&text.to_uppercase(), scale) as f32
            }
        }
    }

    /// Words of `text` in lines no wider than `width` at `size`.
    pub fn wrap(&self, text: &str, size: f32, f: Face, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut line = String::new();
        for word in text.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if self.measure(&candidate, size, f) <= width || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    /// The distance from one line's baseline to the next at `size`.
    pub fn line_height(&self, size: f32) -> f32 {
        match self.regular.as_ref() {
            Some(font) => {
                let sf = font.as_scaled(PxScale::from(size));
                (sf.ascent() - sf.descent() + sf.line_gap()).max(size * 1.1)
            }
            None => (bitmap_scale(size) * 9) as f32,
        }
    }

    /// Draws `text` with its top-left at `(x, y)`; returns the width drawn.
    pub fn draw(&self, img: &mut Image<'_>, x: f32, y: f32, text: &str, size: f32, f: Face, c: [u8; 4]) -> f32 {
        let Some(font) = self.face(f) else {
            let scale = bitmap_scale(size);
            let up = text.to_uppercase();
            let mut canvas = crate::ui::Canvas::new(img.w, img.h);
            canvas.ink(x as i32, y as i32, &up, scale, c);
            for (i, a) in canvas.px.chunks(4).enumerate() {
                if a[3] > 0 {
                    img.blend((i % img.w as usize) as i32, (i / img.w as usize) as i32, c, a[3] as f32 / 255.0);
                }
            }
            return crate::ui::text_width(&up, scale) as f32;
        };
        let sf = font.as_scaled(PxScale::from(size));
        let baseline = y + sf.ascent();
        let mut pen = x;
        let mut prev = None;
        for ch in text.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                pen += sf.kern(p, id);
            }
            let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| img.blend(b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, cov));
            }
            pen += sf.h_advance(id);
            prev = Some(id);
        }
        pen - x
    }

    /// Draws `text` centred on `cx`.
    pub fn draw_centred(&self, img: &mut Image<'_>, cx: f32, y: f32, text: &str, size: f32, f: Face, c: [u8; 4]) {
        let w = self.measure(text, size, f);
        self.draw(img, cx - w * 0.5, y, text, size, f, c);
    }
}

/// The bitmap font's integer scale for a requested pixel size.
fn bitmap_scale(size: f32) -> i32 {
    ((size / 9.0).round() as i32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_drawn_wrapped_and_measured_whichever_face_is_found() {
        for serif in [Serif::load(), Serif::bitmap_only()] {
            let mut px = vec![0u8; 400 * 120 * 4];
            let mut img = Image { w: 400, h: 120, px: &mut px };
            let w = serif.draw(&mut img, 10.0, 10.0, "The Girl Who Swallowed the Sun", 22.0, Face::Italic, [255, 240, 220, 255]);
            assert!(w > 50.0, "something was drawn ({:?})", serif.source);
            assert!(px.chunks(4).any(|p| p[3] > 0));
            let lines = serif.wrap("A long story that will not fit on one line of a narrow label at this size", 18.0, Face::Regular, 200.0);
            assert!(lines.len() >= 2);
            for l in &lines {
                assert!(serif.measure(l, 18.0, Face::Regular) <= 200.0 || !l.contains(' '));
            }
        }
    }
}
