//! Chunk meshes: the ground and every plant of a 48 m chunk baked into three merged meshes.
//!
//! A forest cannot be thousands of scene objects (one draw call each), so a chunk becomes three draws: the **ground** (a 2 m grid whose heights,
//! normals and colours come straight from the generator, so neighbouring chunks meet exactly), the **solid** plants (trees and shrubs, which cast
//! shadows) and the **flora** (flowers and grass, which only receive them). Vertices are relative to the chunk's corner in x and z and absolute in y.
//!
//! How much of a chunk is built depends on how far it is ([`Lod`]): each kind of small plant has a cap per chunk, and a chunk with more than that keeps
//! the ones with the lowest `rank` (so thinning is even, a thinner chunk is a subset of a fuller one and nothing pops); the farthest chunks also use
//! cheaper plant models; the farthest have no flowers at all (the ground carries their colour instead). The plant models come from a [`Library`]: a handful of variants of each species, built on first use and shared by every plant.

use super::flora::{self, Kind, SpeciesId};
use super::geo::Geo;
use super::shapes;
use super::world::{ChunkId, World, CHUNK};
use glam::Vec3;
use std::sync::OnceLock;

/// Metres between ground vertices.
pub const GROUND_CELL: f64 = 2.0;
/// The flowers and grass of a chunk are kept in a `FLORA_CELLS` x `FLORA_CELLS` grid of squares, each a run of the index buffer, so the shadow pass can draw only the
/// squares near the player.
pub const FLORA_CELLS: usize = 4;
/// Metres on a side of a flora square.
pub const FLORA_CELL: f32 = (CHUNK / FLORA_CELLS as f64) as f32;
/// Shape variants of each species in the library.
const VARIANTS: u32 = 6;
/// Colour tints of each variant.
const TINTS: u32 = 3;

/// How much detail a chunk is built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lod {
    /// Everything.
    Near,
    /// Fewer flowers and grass, cheaper plant models.
    Mid,
    /// No flowers or grass, few shrubs.
    Far,
}

impl Lod {
    /// The detail for a chunk `chunks` chunk-widths from the viewer (0 is the chunk they stand in).
    pub fn for_distance(chunks: f32) -> Lod {
        if chunks < 1.7 {
            Lod::Near
        } else if chunks < 3.2 {
            Lod::Mid
        } else {
            Lod::Far
        }
    }

    /// The most plants of a kind a chunk keeps at this detail (trees are never capped: a forest's outline is its character).
    pub fn cap(self, kind: Kind) -> usize {
        match (self, kind) {
            (_, Kind::Tree) => usize::MAX,
            (Lod::Near, Kind::Shrub) => 300,
            (Lod::Near, Kind::Flower) => 700,
            (Lod::Near, Kind::Grass) => 1400,
            (Lod::Mid, Kind::Shrub) => 100,
            (Lod::Mid, Kind::Flower) => 120,
            (Lod::Mid, Kind::Grass) => 250,
            (Lod::Far, Kind::Shrub) => 20,
            (Lod::Far, Kind::Flower) => 0,
            (Lod::Far, Kind::Grass) => 0,
        }
    }
}

/// Darkens a small plant toward its root, the shade the stems and blades cast on each other and on the ground at their feet. It is what stops a distant meadow
/// looking pasted on the grass (the near shadow map only reaches a few metres, and no ambient occlusion reaches farther): free at run time, it is in the colours.
fn root_shade(geo: &mut Geo, height: f32) {
    for (c, p) in geo.col.iter_mut().zip(&geo.pos) {
        let t = (p[1] / height.max(1e-4)).clamp(0.0, 1.0);
        let k = ROOT_SHADE + (1.0 - ROOT_SHADE) * (t / ROOT_REACH).min(1.0).powf(0.7);
        *c = [c[0] * k, c[1] * k, c[2] * k];
    }
}

/// How much of its colour a plant keeps at the ground.
const ROOT_SHADE: f32 = 0.45;
/// Up to what fraction of its height the shading reaches.
const ROOT_REACH: f32 = 0.55;

/// A plant model and the height it was built at.
struct Model {
    geo: Geo,
    height: f32,
}

/// Shared plant models: `VARIANTS` shapes of each species in `TINTS` colours, near and far, each built the first time it is wanted.
pub struct Library {
    slots: Vec<OnceLock<Model>>,
}

impl Default for Library {
    fn default() -> Self {
        Library::new()
    }
}

impl Library {
    /// An empty library (models are built on demand).
    pub fn new() -> Library {
        Library { slots: (0..flora::SPECIES.len() * 2 * (VARIANTS * TINTS) as usize).map(|_| OnceLock::new()).collect() }
    }

    fn model(&self, id: SpeciesId, seed: u32, tint: f32, far: bool) -> &Model {
        let v = seed % VARIANTS;
        let t = ((tint.clamp(0.0, 0.9999) * TINTS as f32) as u32).min(TINTS - 1);
        let slot = ((id.0 as usize * 2 + far as usize) * VARIANTS as usize + v as usize) * TINTS as usize + t as usize;
        self.slots[slot].get_or_init(|| {
            let s = flora::species(id);
            let height = (s.height.0 + s.height.1) * 0.5;
            let tint = (t as f32 + 0.5) / TINTS as f32;
            let mut geo = shapes::build_lod(id, height, v.wrapping_mul(7919).wrapping_add(id.0 as u32 * 104_729), tint, far);
            if matches!(s.kind, Kind::Flower | Kind::Grass) {
                root_shade(&mut geo, height);
            }
            Model { geo, height }
        })
    }
}

/// A baked chunk.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Which chunk.
    pub id: ChunkId,
    /// The detail it was built with.
    pub lod: Lod,
    /// The ground.
    pub ground: Geo,
    /// Trees and shrubs (they cast shadows).
    pub solid: Geo,
    /// Flowers and grass, grouped by square: see [`Chunk::flora_cells`].
    pub flora: Geo,
    /// How many indices of `solid` belong to the trees (they come first, then the shrubs): the far shadow map draws only those.
    pub solid_trees: usize,
    /// For each flora square (row by row), the start and length of its run in `flora.idx`.
    pub flora_cells: Vec<(u32, u32)>,
    /// How many plants of each kind went in: trees, shrubs, flowers, grass.
    pub counts: [usize; 4],
    /// The lowest and highest point of everything in the chunk, relative to its corner in x and z.
    pub bounds: (Vec3, Vec3),
}

impl Chunk {
    /// Triangles in the whole chunk.
    pub fn tris(&self) -> usize {
        self.ground.tris() + self.solid.tris() + self.flora.tris()
    }

    /// Vertices in the whole chunk.
    pub fn verts(&self) -> usize {
        self.ground.pos.len() + self.solid.pos.len() + self.flora.pos.len()
    }
}

/// The ground of a chunk: a grid of `GROUND_CELL` squares, vertices relative to the chunk's corner in x and z.
pub fn ground(world: &World, id: ChunkId) -> Geo {
    let (x0, z0) = id.origin();
    let n = (CHUNK / GROUND_CELL).round() as usize;
    let mut g = Geo::default();
    for j in 0..=n {
        for i in 0..=n {
            let (lx, lz) = (i as f64 * GROUND_CELL, j as f64 * GROUND_CELL);
            let (wx, wz) = (x0 + lx, z0 + lz);
            g.pos.push([lx as f32, world.height(wx, wz), lz as f32]);
            g.nrm.push(world.normal(wx, wz));
            g.col.push(world.ground_color(wx, wz));
            g.sway.push(0.0);
        }
    }
    let at = |i: usize, j: usize| (j * (n + 1) + i) as u32;
    for j in 0..n {
        for i in 0..n {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
            // Alternate the diagonal so the facets do not all lean one way.
            if (i + j) % 2 == 0 {
                g.idx.extend_from_slice(&[a, c, b, b, c, d]);
            } else {
                g.idx.extend_from_slice(&[a, c, d, a, d, b]);
            }
        }
    }
    g
}

/// Bakes a chunk at a level of detail.
pub fn build_chunk(world: &World, lib: &Library, id: ChunkId, lod: Lod) -> Chunk {
    let (x0, z0) = id.origin();
    let mut out = Chunk {
        id,
        lod,
        ground: ground(world, id),
        solid: Geo::default(),
        flora: Geo::default(),
        solid_trees: 0,
        flora_cells: Vec::new(),
        counts: [0; 4],
        bounds: (Vec3::ZERO, Vec3::ZERO),
    };
    let mut cells: Vec<Geo> = (0..FLORA_CELLS * FLORA_CELLS).map(|_| Geo::default()).collect();
    for (slot, kind) in [Kind::Tree, Kind::Shrub, Kind::Flower, Kind::Grass].into_iter().enumerate() {
        let cap = lod.cap(kind);
        if cap == 0 {
            continue;
        }
        let plants = world.plants(id, kind);
        // Keep the lowest-ranked `cap` of them: the same share everywhere in the chunk.
        let keep = if plants.len() > cap { cap as f32 / plants.len() as f32 } else { 1.0 };
        for p in plants {
            if p.rank >= keep {
                continue;
            }
            let m = lib.model(p.species, p.seed, p.tint, lod != Lod::Near);
            let at = Vec3::new((p.x - x0) as f32, p.y, (p.z - z0) as f32);
            if matches!(kind, Kind::Tree | Kind::Shrub) {
                out.solid.add(&m.geo, at, p.yaw, p.height / m.height);
            } else {
                let cell = |v: f32| ((v / FLORA_CELL) as usize).min(FLORA_CELLS - 1);
                cells[cell(at.z) * FLORA_CELLS + cell(at.x)].add(&m.geo, at, p.yaw, p.height / m.height);
            }
            out.counts[slot] += 1;
        }
        if kind == Kind::Tree {
            out.solid_trees = out.solid.idx.len();
        }
    }
    for cell in &cells {
        let start = out.flora.idx.len() as u32;
        out.flora.extend(cell);
        out.flora_cells.push((start, out.flora.idx.len() as u32 - start));
    }
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for g in [&out.ground, &out.solid, &out.flora] {
        if !g.pos.is_empty() {
            let (a, b) = g.bounds();
            lo = lo.min(a);
            hi = hi.max(b);
        }
    }
    out.bounds = (lo, hi);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procgen::world::Config;

    fn world() -> World {
        World::new(Config { seed: 7, ..Config::default() })
    }

    #[test]
    fn the_ground_is_a_complete_grid_that_follows_the_generator() {
        let w = world();
        let id = ChunkId { x: 3, z: -2 };
        let g = ground(&w, id);
        let n = 24;
        assert_eq!(g.pos.len(), 25 * 25);
        assert_eq!(g.tris(), n * n * 2);
        let (x0, z0) = id.origin();
        for k in [0usize, 12, 300, 624] {
            let p = g.pos[k];
            assert_eq!(p[1], w.height(x0 + p[0] as f64, z0 + p[2] as f64));
        }
        // Every triangle faces up (counter-clockwise from above).
        for t in g.idx.as_chunks::<3>().0 {
            let (a, b, c) = (Vec3::from(g.pos[t[0] as usize]), Vec3::from(g.pos[t[1] as usize]), Vec3::from(g.pos[t[2] as usize]));
            assert!((b - a).cross(c - a).y > 0.0, "a ground triangle faces down");
        }
    }

    #[test]
    fn neighbouring_chunks_meet_exactly() {
        let w = world();
        let (a, b) = (ground(&w, ChunkId { x: 0, z: 0 }), ground(&w, ChunkId { x: 1, z: 0 }));
        for j in 0..=24 {
            let right_edge = a.pos[j * 25 + 24];
            let left_edge = b.pos[j * 25];
            assert_eq!(right_edge[1], left_edge[1], "heights differ along the seam at row {j}");
            assert_eq!(a.nrm[j * 25 + 24], b.nrm[j * 25], "normals differ along the seam at row {j}");
            assert_eq!(a.col[j * 25 + 24], b.col[j * 25], "colours differ along the seam at row {j}");
        }
    }

    #[test]
    fn lower_detail_is_always_cheaper_and_never_adds_plants() {
        let w = world();
        let lib = Library::new();
        for id in [ChunkId { x: 2, z: 2 }, ChunkId { x: -5, z: 7 }, ChunkId { x: 12, z: -3 }] {
            let (n, m, f) = (build_chunk(&w, &lib, id, Lod::Near), build_chunk(&w, &lib, id, Lod::Mid), build_chunk(&w, &lib, id, Lod::Far));
            assert!(n.tris() >= m.tris() && m.tris() >= f.tris(), "{id:?}: {} {} {}", n.tris(), m.tris(), f.tris());
            for k in 0..4 {
                assert!(n.counts[k] >= m.counts[k] && m.counts[k] >= f.counts[k]);
            }
            assert_eq!(n.counts[0], f.counts[0], "every tree stays, however far");
        }
    }

    #[test]
    fn a_chunk_is_valid_deterministic_and_inside_its_bounds() {
        let w = world();
        let lib = Library::new();
        let id = ChunkId { x: 4, z: 4 };
        let c = build_chunk(&w, &lib, id, Lod::Near);
        let again = build_chunk(&w, &lib, id, Lod::Near);
        assert_eq!(c.solid.pos, again.solid.pos);
        assert_eq!(c.flora.idx, again.flora.idx);
        for g in [&c.ground, &c.solid, &c.flora] {
            assert!(g.idx.iter().all(|i| (*i as usize) < g.pos.len()));
            assert_eq!(g.sway.len(), g.pos.len());
            assert!(g.pos.iter().flatten().all(|v| v.is_finite()));
        }
        let (lo, hi) = c.bounds;
        assert!(lo.x >= -12.0 && hi.x <= CHUNK as f32 + 12.0 && lo.z >= -12.0 && hi.z <= CHUNK as f32 + 12.0, "{lo:?} {hi:?}");
    }

    #[test]
    fn chunks_stay_inside_a_triangle_and_memory_budget() {
        let w = world();
        let lib = Library::new();
        let (mut worst_tris, mut worst_verts) = (0, 0);
        let mut total = 0;
        let mut n = 0;
        for cz in -4..4 {
            for cx in -4..4 {
                let c = build_chunk(&w, &lib, ChunkId { x: cx * 3, z: cz * 3 }, Lod::Near);
                worst_tris = worst_tris.max(c.tris());
                worst_verts = worst_verts.max(c.verts());
                total += c.tris();
                n += 1;
            }
        }
        assert!(worst_tris < 110_000, "the heaviest near chunk has {worst_tris} triangles");
        assert!(worst_verts * 40 < 24_000_000, "the heaviest near chunk needs {} MB of vertices", worst_verts * 40 / 1_000_000);
        assert!(total / n > 8_000, "chunks are suspiciously empty");
    }

    #[test]
    fn plants_are_scaled_to_their_own_height() {
        let w = world();
        let lib = Library::new();
        let id = ChunkId { x: 1, z: 6 };
        let c = build_chunk(&w, &lib, id, Lod::Near);
        let tallest_tree = w.plants(id, Kind::Tree).iter().map(|p| p.y + p.height).fold(0.0f32, f32::max);
        if tallest_tree > 0.0 {
            assert!(c.solid.bounds().1.y > tallest_tree * 0.7);
        }
    }

    #[test]
    fn flora_squares_and_the_tree_run_partition_the_index_buffers_exactly() {
        let w = world();
        let lib = Library::new();
        let c = build_chunk(&w, &lib, ChunkId { x: 4, z: 4 }, Lod::Near);
        assert_eq!(c.flora_cells.len(), FLORA_CELLS * FLORA_CELLS);
        let mut next = 0;
        for &(start, count) in &c.flora_cells {
            assert_eq!(start, next, "the squares follow one another with no gap");
            assert_eq!(count % 3, 0, "whole triangles");
            next += count;
        }
        assert_eq!(next as usize, c.flora.idx.len());
        assert!(c.solid_trees <= c.solid.idx.len() && c.solid_trees.is_multiple_of(3));
        assert!(c.solid_trees > 0, "this chunk has trees");
        // Every flora triangle sits in the square its run says, give or take a plant's width.
        for (k, &(start, count)) in c.flora_cells.iter().enumerate() {
            let (cx, cz) = ((k % FLORA_CELLS) as f32 * FLORA_CELL, (k / FLORA_CELLS) as f32 * FLORA_CELL);
            for &i in &c.flora.idx[start as usize..(start + count) as usize] {
                let p = c.flora.pos[i as usize];
                assert!(p[0] > cx - 3.0 && p[0] < cx + FLORA_CELL + 3.0 && p[2] > cz - 3.0 && p[2] < cz + FLORA_CELL + 3.0, "square {k}: {p:?}");
            }
        }
    }

    #[test]
    fn small_plants_are_darker_at_the_root_than_at_the_tip() {
        let lib = Library::new();
        let grass = flora::SPECIES.iter().position(|s| s.kind == Kind::Grass).expect("a grass species");
        let m = lib.model(SpeciesId(grass as u8), 1, 0.5, false);
        let (mut low, mut high) = ((0.0f32, 0), (0.0f32, 0));
        for (c, p) in m.geo.col.iter().zip(&m.geo.pos) {
            let lum = c[0] + c[1] + c[2];
            if p[1] < m.height * 0.1 {
                low = (low.0 + lum, low.1 + 1);
            } else if p[1] > m.height * 0.7 {
                high = (high.0 + lum, high.1 + 1);
            }
        }
        assert!(low.1 > 0 && high.1 > 0);
        assert!(low.0 / (low.1 as f32) < 0.8 * high.0 / (high.1 as f32), "root {} tip {}", low.0 / low.1 as f32, high.0 / high.1 as f32);
    }

    #[test]
    fn lod_follows_distance() {
        assert_eq!(Lod::for_distance(0.0), Lod::Near);
        assert_eq!(Lod::for_distance(2.0), Lod::Mid);
        assert_eq!(Lod::for_distance(5.0), Lod::Far);
    }
}
