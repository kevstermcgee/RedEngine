//! The 2D engine's one font: a 5x7 bitmap face (the glyph table the analysis tools use, plus `$ " ; & @ |`), uppercase only (lowercase draws as capitals), with a 1-pixel gap
//! between letters. The glyph table itself is `src/font_glyphs.rs`, shared with the analysis tools' labels (`src/tools/font.rs` draws into an `image` buffer, which the browser build must not depend on).

/// Glyph width in pixels.
pub const GLYPH_W: i32 = 5;
/// Glyph height in pixels.
pub const GLYPH_H: i32 = 7;
/// Advance per character at scale 1 (glyph plus one pixel gap).
pub const ADVANCE: i32 = 6;

#[path = "../../../src/font_glyphs.rs"]
mod glyph_table;
use glyph_table::GLYPHS as SHARED;

/// What the 2D renderer adds to the shared table (the 3D `sign` alphabet is deliberately unchanged).
const EXTRA: &[(char, [&str; 7])] = &[
    ('$', ["..#..", ".####", "#.#..", ".###.", "..#.#", "####.", "..#.."]),
    ('"', [".#.#.", ".#.#.", "#..#.", ".....", ".....", ".....", "....."]),
    (';', [".....", "..#..", ".....", ".....", "..#..", "..#..", ".#..."]),
    ('&', [".##..", "#..#.", "#.#..", ".#...", "#.#.#", "#..#.", ".##.#"]),
    ('@', [".###.", "#...#", "#.###", "#.#.#", "#.###", "#....", ".####"]),
    ('|', ["..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."]),
];

/// The rows of a glyph, if the font has it (lowercase is mapped up).
pub fn glyph(c: char) -> Option<&'static [&'static str; 7]> {
    let c = c.to_ascii_uppercase();
    SHARED.iter().chain(EXTRA).find(|(g, _)| *g == c).map(|(_, rows)| rows)
}

/// Width in pixels of `text` at `scale`.
pub fn text_width(text: &str, scale: i32) -> i32 {
    let n = text.chars().count() as i32;
    if n == 0 {
        0
    } else {
        (n * ADVANCE - 1) * scale
    }
}

/// Calls `plot(x, y)` for every lit pixel of `text` placed with its top-left at `(x, y)` (a character with no glyph is a blank cell, a space is blank).
pub fn for_each_pixel(text: &str, x: i32, y: i32, scale: i32, mut plot: impl FnMut(i32, i32)) {
    for (i, ch) in text.chars().enumerate() {
        let Some(rows) = glyph(ch) else { continue };
        let ox = x + i as i32 * ADVANCE * scale;
        for (ry, row) in rows.iter().enumerate() {
            for (rx, px) in row.chars().enumerate() {
                if px == '#' {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            plot(ox + rx as i32 * scale + dx, y + ry as i32 * scale + dy);
                        }
                    }
                }
            }
        }
    }
}
