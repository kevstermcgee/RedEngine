//! A tiny 5x7 bitmap font so the analysis tools can label images without a font dependency.
//! Uppercase only (lowercase is mapped up), digits, the punctuation ids/coordinates use, and one
//! icon glyph (`↓`, a download/save affordance for UI buttons).

use image::{Rgb, RgbImage};

/// The table, its lookup and cell size are `red_core::font_glyphs` (the `sign` macro asks them too); this module draws.
pub use crate::font_glyphs::{glyph, GLYPH_H, GLYPH_W};

/// Width in pixels of `text` at integer `scale`.
pub fn text_width(text: &str, scale: i32) -> i32 {
    let n = text.chars().count() as i32;
    if n == 0 {
        0
    } else {
        n * (GLYPH_W + 1) * scale - scale
    }
}

fn put(img: &mut RgbImage, x: i32, y: i32, c: Rgb<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, c);
    }
}

/// Draws `text` with its top-left at `(x, y)`; unknown characters render as blanks. `halo`
/// draws a 1-pixel outline in that color first, which keeps labels legible over busy fills.
pub fn draw_text(img: &mut RgbImage, x: i32, y: i32, text: &str, scale: i32, color: Rgb<u8>, halo: Option<Rgb<u8>>) {
    let paint = |img: &mut RgbImage, dx: i32, dy: i32, col: Rgb<u8>| {
        let mut cx = x + dx;
        for ch in text.chars() {
            if let Some(rows) = glyph(ch) {
                for (ry, row) in rows.iter().enumerate() {
                    for (rx, b) in row.bytes().enumerate() {
                        if b == b'#' {
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    put(img, cx + rx as i32 * scale + sx, y + dy + ry as i32 * scale + sy, col);
                                }
                            }
                        }
                    }
                }
            }
            cx += (GLYPH_W + 1) * scale;
        }
    };
    if let Some(h) = halo {
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
            paint(img, dx, dy, h);
        }
    }
    paint(img, 0, 0, color);
}

/// Draws `text` centered on `(cx, cy)`.
pub fn draw_text_centered(img: &mut RgbImage, cx: i32, cy: i32, text: &str, scale: i32, color: Rgb<u8>, halo: Option<Rgb<u8>>) {
    let w = text_width(text, scale);
    draw_text(img, cx - w / 2, cy - GLYPH_H * scale / 2, text, scale, color, halo);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_id_characters() {
        for c in "wall_front.seg0 (12.5,-3) house-2".chars().filter(|c| *c != ' ') {
            assert!(glyph(c).is_some(), "missing glyph for {c:?}");
        }
    }
}
