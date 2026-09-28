//! A labelled contact sheet of pictures: the way an AI (or a person in a hurry) looks at twelve screenshots at once. Used by `playtest`; pure image code, no GPU.

use super::font;
use image::{imageops, Rgb, RgbImage, RgbaImage};

/// One picture and its caption.
pub struct Tile {
    /// The picture.
    pub image: RgbaImage,
    /// What is under it (`03 third`).
    pub title: String,
}

const MARGIN: u32 = 6;
const TITLE_H: u32 = 22;
const BACKGROUND: Rgb<u8> = Rgb([22, 24, 30]);

/// The tiles in a grid `cols` wide, each scaled to `tile_w` pixels across, its title above it. An empty list gives a small blank sheet.
pub fn contact_sheet(tiles: &[Tile], cols: u32, tile_w: u32) -> RgbImage {
    let cols = cols.clamp(1, tiles.len().max(1) as u32);
    let tile_w = tile_w.max(64);
    let scaled: Vec<RgbaImage> = tiles
        .iter()
        .map(|t| {
            let h = (t.image.height() as f32 * tile_w as f32 / t.image.width().max(1) as f32).round().max(1.0) as u32;
            imageops::resize(&t.image, tile_w, h, imageops::FilterType::Triangle)
        })
        .collect();
    let rows = (tiles.len() as u32).div_ceil(cols).max(1);
    let row_h: Vec<u32> = (0..rows).map(|r| scaled.iter().skip((r * cols) as usize).take(cols as usize).map(|i| i.height()).max().unwrap_or(0)).collect();
    let width = cols * (tile_w + MARGIN) + MARGIN;
    let height = row_h.iter().map(|h| h + TITLE_H + MARGIN).sum::<u32>() + MARGIN;
    let mut sheet = RgbImage::from_pixel(width, height.max(MARGIN * 2), BACKGROUND);
    let mut y = MARGIN;
    for (r, rh) in row_h.iter().enumerate() {
        for c in 0..cols {
            let Some(i) = scaled.get(r * cols as usize + c as usize) else { break };
            let x = MARGIN + c * (tile_w + MARGIN);
            font::draw_text(&mut sheet, x as i32 + 2, y as i32 + 4, &tiles[r * cols as usize + c as usize].title, 2, Rgb([235, 235, 235]), None);
            for (px, py, p) in i.enumerate_pixels() {
                sheet.put_pixel(x + px, y + TITLE_H + py, Rgb([p[0], p[1], p[2]]));
            }
        }
        y += rh + TITLE_H + MARGIN;
    }
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(color: [u8; 4], w: u32, h: u32, title: &str) -> Tile {
        Tile { image: RgbaImage::from_pixel(w, h, image::Rgba(color)), title: title.to_string() }
    }

    #[test]
    fn tiles_are_scaled_to_one_width_in_a_grid_with_their_titles_above() {
        let tiles = [tile([255, 0, 0, 255], 320, 180, "01 red"), tile([0, 255, 0, 255], 640, 360, "02 green"), tile([0, 0, 255, 255], 320, 180, "03 blue")];
        let sheet = contact_sheet(&tiles, 2, 160);
        assert_eq!(sheet.width(), 2 * (160 + MARGIN) + MARGIN);
        assert_eq!(sheet.height(), 2 * (90 + TITLE_H + MARGIN) + MARGIN, "two rows of 160x90 tiles");
        // The first tile's picture starts under its title; its colour survives the scaling.
        assert_eq!(sheet.get_pixel(MARGIN + 80, MARGIN + TITLE_H + 45).0, [255, 0, 0]);
        assert_eq!(sheet.get_pixel(MARGIN + 160 + MARGIN + 80, MARGIN + TITLE_H + 45).0, [0, 255, 0]);
        // Text was drawn in the title strip (some pixel there is not the background).
        assert!((0..160).any(|dx| sheet.get_pixel(MARGIN + dx, MARGIN + 6).0 != BACKGROUND.0));
    }

    #[test]
    fn no_tiles_make_a_small_blank_sheet_and_more_columns_than_tiles_shrink_to_fit() {
        let blank = contact_sheet(&[], 4, 100);
        assert!(blank.width() >= MARGIN && blank.height() >= 2 * MARGIN);
        let one = contact_sheet(&[tile([9, 9, 9, 255], 100, 50, "x")], 5, 100);
        assert_eq!(one.width(), 100 + 2 * MARGIN, "one tile needs one column");
    }
}
