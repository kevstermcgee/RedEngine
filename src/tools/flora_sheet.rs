//! `flora`: a contact sheet of the plant models, drawn by a small software rasteriser (no GPU), so the shapes can be looked at and iterated on
//! anywhere. Each tile is one plant fitted to its tile, lit from the upper left with a sky-bright hemisphere, labelled with its common and
//! Latin names and its height. The renderer draws the same meshes with the engine's own lighting; this view is for judging the shapes.

use crate::procgen::flora::{self, SpeciesId};
use crate::procgen::geo::Geo;
use crate::procgen::shapes;
use crate::tools::font;
use glam::{Vec2, Vec3};
use image::{Rgb, RgbImage};

fn srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// A view of a mesh: where the camera is turned and how the picture is fitted.
struct View {
    right: Vec3,
    up: Vec3,
    toward: Vec3,
    centre: Vec2,
    scale: f32,
    size: u32,
}

impl View {
    fn project(&self, p: Vec3) -> (f32, f32, f32) {
        let x = (p.dot(self.right) - self.centre.x) * self.scale + self.size as f32 / 2.0;
        let y = self.size as f32 * 0.5 - (p.dot(self.up) - self.centre.y) * self.scale;
        (x, y, p.dot(self.toward))
    }
}

/// Draws one mesh (and the ground it stands on) into a square RGB image. Rendered at double size and shrunk for smooth edges.
pub fn draw_plant(g: &Geo, size: u32) -> RgbImage {
    let big = size * 2;
    let (yaw, pitch) = (0.65f32, 0.30f32);
    let toward = Vec3::new(yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()).normalize();
    let right = Vec3::Y.cross(toward).normalize();
    let up = toward.cross(right).normalize();
    let (lo, hi) = g.bounds();
    let (mut mn, mut mx) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for p in &g.pos {
        let p = Vec3::from(*p);
        let q = Vec2::new(p.dot(right), p.dot(up));
        mn = mn.min(q);
        mx = mx.max(q);
    }
    let extent = (mx - mn).max_element().max(1e-3);
    let view = View { right, up, toward, centre: (mn + mx) * 0.5, scale: big as f32 * 0.74 / extent, size: big };
    let n = (big * big) as usize;
    let mut depth = vec![f32::MAX; n];
    let mut img = vec![[0.0f32; 3]; n];
    // Sky to haze, top to bottom.
    for y in 0..big {
        let t = y as f32 / big as f32;
        let c = [0.55 + 0.25 * t, 0.72 + 0.15 * t, 0.92 + 0.02 * t];
        for x in 0..big {
            img[(y * big + x) as usize] = c;
        }
    }
    let light = Vec3::new(-0.45, 0.8, 0.4).normalize();
    let ground_r = (hi.x.abs().max(lo.x.abs()).max(hi.z.abs()).max(lo.z.abs()) * 1.6).max(extent * 0.45);
    let ground = {
        let mut m = Geo::default();
        let ring = 24;
        for k in 0..ring {
            let (a, b) = (k as f32 / ring as f32 * std::f32::consts::TAU, (k + 1) as f32 / ring as f32 * std::f32::consts::TAU);
            m.tri(
                Vec3::ZERO,
                Vec3::new(b.cos(), 0.0, b.sin()) * ground_r,
                Vec3::new(a.cos(), 0.0, a.sin()) * ground_r,
                [0.08, 0.24, 0.05],
                [0.07, 0.2, 0.05],
                [0.07, 0.2, 0.05],
            );
        }
        m
    };
    for mesh in [&ground, g] {
        for t in mesh.idx.as_chunks::<3>().0 {
            let v: Vec<Vec3> = t.iter().map(|i| Vec3::from(mesh.pos[*i as usize])).collect();
            let face = (v[1] - v[0]).cross(v[2] - v[0]);
            if face.dot(toward) >= 0.0 {
                continue;
            }
            let pr: Vec<(f32, f32, f32)> = v.iter().map(|p| view.project(*p)).collect();
            let area = (pr[1].0 - pr[0].0) * (pr[2].1 - pr[0].1) - (pr[2].0 - pr[0].0) * (pr[1].1 - pr[0].1);
            if area.abs() < 1e-6 {
                continue;
            }
            let (x0, x1) = (
                pr.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as i32,
                pr.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().min(big as f32 - 1.0) as i32,
            );
            let (y0, y1) = (
                pr.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as i32,
                pr.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().min(big as f32 - 1.0) as i32,
            );
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((pr[1].0 - px) * (pr[2].1 - py) - (pr[2].0 - px) * (pr[1].1 - py)) / area;
                    let w1 = ((pr[2].0 - px) * (pr[0].1 - py) - (pr[0].0 - px) * (pr[2].1 - py)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * pr[0].2 + w1 * pr[1].2 + w2 * pr[2].2;
                    let idx = (y as u32 * big + x as u32) as usize;
                    if z >= depth[idx] {
                        continue;
                    }
                    depth[idx] = z;
                    let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    let n = (Vec3::from(mesh.nrm[a]) * w0 + Vec3::from(mesh.nrm[b]) * w1 + Vec3::from(mesh.nrm[c]) * w2).normalize_or_zero();
                    let lit = 0.30 + 0.80 * n.dot(light).max(0.0) + 0.30 * (n.y * 0.5 + 0.5);
                    let mut col = [0.0f32; 3];
                    for i in 0..3 {
                        col[i] = srgb((mesh.col[a][i] * w0 + mesh.col[b][i] * w1 + mesh.col[c][i] * w2) * lit);
                    }
                    img[idx] = col;
                }
            }
        }
    }
    let mut out = RgbImage::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let mut s = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = img[((y * 2 + dy) * big + x * 2 + dx) as usize];
                for i in 0..3 {
                    // Sky and ground were written display-ready; only mesh pixels went through `srgb`.
                    s[i] += p[i] * 0.25;
                }
            }
            out.put_pixel(x, y, Rgb([(s[0] * 255.0) as u8, (s[1] * 255.0) as u8, (s[2] * 255.0) as u8]));
        }
    }
    out
}

/// One tile of the sheet: a species at a height with a seed, labelled.
pub struct Tile {
    /// Which species.
    pub id: SpeciesId,
    /// Its height in metres.
    pub height: f32,
    /// The variation seed.
    pub seed: u32,
    /// The colour tint.
    pub tint: f32,
}

/// The tiles of a sheet: every species once, or `variants` plants of the one species named.
pub fn tiles(only: &[SpeciesId], variants: u32, seed: u32) -> Vec<Tile> {
    let list: Vec<SpeciesId> = if only.is_empty() { flora::SPECIES.iter().enumerate().map(|(i, _)| SpeciesId(i as u8)).collect() } else { only.to_vec() };
    let mut out = Vec::new();
    for id in list {
        let s = flora::species(id);
        let n = if only.len() == 1 { variants.max(1) } else { 1 };
        for v in 0..n {
            let f = if n == 1 { 0.5 } else { v as f32 / (n - 1) as f32 };
            out.push(Tile { id, height: s.height.0 + (s.height.1 - s.height.0) * f, seed: seed + v, tint: f });
        }
    }
    out
}

/// Draws the sheet.
pub fn sheet(tiles: &[Tile], cols: u32, tile: u32) -> RgbImage {
    let cols = cols.clamp(1, tiles.len().max(1) as u32);
    let rows = (tiles.len() as u32).div_ceil(cols);
    let label_h = 34;
    let mut canvas = RgbImage::from_pixel(cols * tile, rows * (tile + label_h), Rgb([20, 24, 20]));
    for (i, t) in tiles.iter().enumerate() {
        let (cx, cy) = (i as u32 % cols, i as u32 / cols);
        let g = shapes::build(t.id, t.height, t.seed, t.tint);
        let img = draw_plant(&g, tile);
        image::imageops::replace(&mut canvas, &img, (cx * tile) as i64, (cy * (tile + label_h)) as i64);
        let s = flora::species(t.id);
        let (x, y) = ((cx * tile + 6) as i32, (cy * (tile + label_h) + tile + 3) as i32);
        font::draw_text(&mut canvas, x, y, s.common, 2, Rgb([240, 240, 230]), None);
        font::draw_text(&mut canvas, x, y + 15, &format!("{} {:.1}M {}T", s.latin, t.height, g.tris()), 1, Rgb([170, 190, 150]), None);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_shows_every_species_and_they_look_different() {
        let t = tiles(&[], 1, 1);
        assert_eq!(t.len(), flora::SPECIES.len());
        let img = sheet(&t, 5, 96);
        assert_eq!(img.width(), 5 * 96);
        let tile_of = |i: usize| image::imageops::crop_imm(&img, (i as u32 % 5) * 96, (i as u32 / 5) * 130, 96, 96).to_image();
        // Each tile has plant pixels that are not sky or ground: a lot of distinct colours.
        for i in 0..t.len() {
            let colours: std::collections::HashSet<_> = tile_of(i).pixels().map(|p| p.0).collect();
            assert!(colours.len() > 40, "{} is nearly blank ({} colours)", flora::species(t[i].id).key, colours.len());
        }
        let a = tile_of(0);
        let b = tile_of(8);
        assert_ne!(a.as_raw(), b.as_raw());
    }

    #[test]
    fn variants_of_one_species_differ() {
        let id = flora::by_key("oak").unwrap();
        let t = tiles(&[id], 4, 3);
        assert_eq!(t.len(), 4);
        let a = shapes::build(t[0].id, t[0].height, t[0].seed, t[0].tint);
        let b = shapes::build(t[3].id, t[3].height, t[3].seed, t[3].tint);
        assert_ne!(a.pos, b.pos);
    }
}
