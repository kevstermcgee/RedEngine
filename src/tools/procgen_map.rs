//! `procgen`: a top-down map of a generated world, with counts, so a world can be looked at and measured without a renderer.
//!
//! The ground is shaded by slope and coloured as the world would colour it; trees are discs the size of their crowns in the species' colour,
//! shrubs smaller discs, flowers dots. `--biomes` paints the kind of country instead, `--grid` draws the chunk borders. The printed summary
//! gives plant counts per species and kind of country, and how long a chunk takes to generate.

use crate::procgen::flora::{self, Kind};
use crate::procgen::{Biome, ChunkId, World, CHUNK};
use image::{Rgb, RgbImage};
use std::time::Instant;

/// What to draw.
#[derive(Debug, Clone)]
pub struct MapOpts {
    /// World position of the middle of the map.
    pub centre: (f64, f64),
    /// Side of the map in metres.
    pub size: f64,
    /// Pixels per metre.
    pub scale: f32,
    /// Draw chunk borders.
    pub grid: bool,
    /// Paint the kind of country instead of the ground colour.
    pub biomes: bool,
}

/// Counts from a map.
#[derive(Debug, Default)]
pub struct MapStats {
    /// Chunks generated.
    pub chunks: usize,
    /// Plants drawn per species id.
    pub per_species: Vec<usize>,
    /// Share of the map (0 to 1) in each kind of country, in [`BIOMES`] order.
    pub biome_share: Vec<f32>,
    /// Mean milliseconds to generate every layer of one chunk.
    pub ms_per_chunk: f32,
}

/// Every biome, in the order of [`MapStats::biome_share`].
pub const BIOMES: [Biome; 6] = [Biome::Meadow, Biome::Wildflowers, Biome::Grove, Biome::Forest, Biome::Pinewood, Biome::Glade];

fn srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

fn biome_colour(b: Biome) -> [f32; 3] {
    match b {
        Biome::Meadow => [0.30, 0.55, 0.18],
        Biome::Wildflowers => [0.85, 0.55, 0.75],
        Biome::Grove => [0.55, 0.65, 0.30],
        Biome::Forest => [0.06, 0.28, 0.08],
        Biome::Pinewood => [0.04, 0.16, 0.14],
        Biome::Glade => [0.95, 0.85, 0.35],
    }
}

fn disc(img: &mut RgbImage, cx: f32, cy: f32, r: f32, col: [u8; 3], alpha: f32) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
    let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
    for y in y0.max(0)..=y1.min(h - 1) {
        for x in x0.max(0)..=x1.min(w - 1) {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            if d <= r {
                // A slightly darker rim reads as a crown's edge.
                let k = if d > r - 0.8 && r > 2.5 { 0.72 } else { 1.0 };
                let px = img.get_pixel_mut(x as u32, y as u32);
                for i in 0..3 {
                    px.0[i] = (px.0[i] as f32 * (1.0 - alpha) + col[i] as f32 * k * alpha) as u8;
                }
            }
        }
    }
}

/// Draws the map and counts what is on it.
pub fn map(world: &World, o: &MapOpts) -> (RgbImage, MapStats) {
    let px = (o.size * o.scale as f64).round().max(16.0) as u32;
    let mut img = RgbImage::new(px, px);
    let (x0, z0) = (o.centre.0 - o.size / 2.0, o.centre.1 - o.size / 2.0);
    let per_px = o.size / px as f64;
    // Ground, sampled on blocks of pixels (the fields are smooth) and lit from the north-west.
    let block = 3u32;
    let light = {
        let l = [-0.5f32, 0.75, -0.45];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    let mut share = [0usize; 6];
    let mut cells = 0usize;
    for by in (0..px).step_by(block as usize) {
        for bx in (0..px).step_by(block as usize) {
            let (wx, wz) = (x0 + (bx as f64 + block as f64 / 2.0) * per_px, z0 + (by as f64 + block as f64 / 2.0) * per_px);
            let n = world.normal(wx, wz);
            let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
            let shade = 0.45 + 0.75 * lit;
            let biome = world.biome(wx, wz);
            let bi = BIOMES.iter().position(|b| *b == biome).unwrap_or(0);
            share[bi] += 1;
            cells += 1;
            let base = if o.biomes { biome_colour(biome) } else { world.ground_color(wx, wz) };
            let c = [srgb(base[0] * shade), srgb(base[1] * shade), srgb(base[2] * shade)];
            for y in by..(by + block).min(px) {
                for x in bx..(bx + block).min(px) {
                    img.put_pixel(x, y, Rgb(c));
                }
            }
        }
    }
    // Plants, from the chunks that touch the map.
    let (c0, c1) = (ChunkId::at(x0, z0), ChunkId::at(x0 + o.size, z0 + o.size));
    let mut stats = MapStats { per_species: vec![0; flora::SPECIES.len()], ..Default::default() };
    let to_px = |x: f64, z: f64| (((x - x0) / per_px) as f32, ((z - z0) / per_px) as f32);
    let started = Instant::now();
    let mut layers: Vec<(ChunkId, Kind, Vec<crate::procgen::Plant>)> = Vec::new();
    for cz in c0.z..=c1.z {
        for cx in c0.x..=c1.x {
            let id = ChunkId { x: cx, z: cz };
            stats.chunks += 1;
            for kind in [Kind::Grass, Kind::Flower, Kind::Shrub, Kind::Tree] {
                layers.push((id, kind, world.plants(id, kind)));
            }
        }
    }
    stats.ms_per_chunk = started.elapsed().as_secs_f32() * 1000.0 / stats.chunks.max(1) as f32;
    for kind in [Kind::Grass, Kind::Flower, Kind::Shrub, Kind::Tree] {
        for (_, k, plants) in &layers {
            if *k != kind {
                continue;
            }
            for p in plants {
                stats.per_species[p.species.0 as usize] += 1;
                if o.biomes {
                    continue;
                }
                let s = flora::species(p.species);
                let pal = s.palette();
                let col = pal[((p.tint * pal.len() as f32) as usize).min(pal.len() - 1)];
                let rgb = [srgb(col[0]), srgb(col[1]), srgb(col[2])];
                let (x, y) = to_px(p.x, p.z);
                match kind {
                    Kind::Grass => {}
                    Kind::Flower => disc(&mut img, x, y, (o.scale * 0.22).max(0.9), rgb, 1.0),
                    Kind::Shrub => disc(&mut img, x, y, (p.height * s.spread * o.scale).max(1.0), rgb, 0.9),
                    Kind::Tree => disc(&mut img, x, y, (p.height * s.spread * o.scale).max(1.5), rgb, 0.92),
                }
            }
        }
    }
    if o.grid {
        let mut cx = (x0 / CHUNK).ceil() * CHUNK;
        while cx < x0 + o.size {
            let x = ((cx - x0) / per_px) as u32;
            for y in 0..px {
                img.put_pixel(x.min(px - 1), y, Rgb([255, 255, 255]));
            }
            cx += CHUNK;
        }
        let mut cz = (z0 / CHUNK).ceil() * CHUNK;
        while cz < z0 + o.size {
            let y = ((cz - z0) / per_px) as u32;
            for x in 0..px {
                img.put_pixel(x, y.min(px - 1), Rgb([255, 255, 255]));
            }
            cz += CHUNK;
        }
    }
    stats.biome_share = share.iter().map(|n| *n as f32 / cells.max(1) as f32).collect();
    (img, stats)
}

/// The printed summary of a map: country shares, then plants per species with their Latin names.
pub fn report(stats: &MapStats) -> String {
    let mut out = String::new();
    let country: Vec<String> =
        BIOMES.iter().zip(&stats.biome_share).filter(|(_, s)| **s > 0.0).map(|(b, s)| format!("{} {:.0}%", b.name(), s * 100.0)).collect();
    out.push_str(&format!("country: {}\n", country.join(", ")));
    for kind in [Kind::Tree, Kind::Shrub, Kind::Flower, Kind::Grass] {
        let total: usize = flora::of_kind(kind).map(|(id, _)| stats.per_species[id.0 as usize]).sum();
        out.push_str(&format!("{kind:?}s: {total}\n"));
        for (id, s) in flora::of_kind(kind) {
            let n = stats.per_species[id.0 as usize];
            if n > 0 {
                out.push_str(&format!("  {:<18} {:<26} {n}\n", s.common, s.latin));
            }
        }
    }
    out.push_str(&format!("{} chunks, {:.1} ms to generate one\n", stats.chunks, stats.ms_per_chunk));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procgen::Config;

    #[test]
    fn a_map_draws_ground_and_plants_and_counts_them() {
        let w = World::new(Config { seed: 3, ..Config::default() });
        let o = MapOpts { centre: (0.0, 0.0), size: 96.0, scale: 3.0, grid: true, biomes: false };
        let (img, stats) = map(&w, &o);
        assert_eq!((img.width(), img.height()), (288, 288));
        assert!(stats.chunks >= 4 && stats.per_species.iter().sum::<usize>() > 500);
        assert!((stats.biome_share.iter().sum::<f32>() - 1.0).abs() < 1e-3);
        let colours: std::collections::HashSet<_> = img.pixels().map(|p| p.0).collect();
        assert!(colours.len() > 60, "{} colours", colours.len());
        assert!(report(&stats).contains("English oak") || report(&stats).contains("Poa pratensis"));
    }
}
