//! The loaded world: chunks by position (shared with the worker threads as `Arc`s; an edit copies a chunk a worker still reads), what each still
//! needs (light, meshes), the edits the player made (kept when a chunk is unloaded, written to the save), and the questions the game asks of it:
//! which block is where, what the crosshair is on, what changes when a block is placed or broken.

use crate::block::Block;
use crate::chunk::{idx, ChunkData, ChunkPos, H, SECTION, SECTIONS, W};
use crate::gen::Generator;
use crate::light::Region;
use glam::{IVec3, Vec3};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// A loaded chunk.
pub struct Slot {
    pub data: Arc<ChunkData>,
    /// Its light is worked out.
    pub lit: bool,
    /// Sections whose mesh is out of date (bit per section).
    pub dirty: u8,
    /// Every section has been meshed once.
    pub meshed: bool,
    /// Counts every change to its blocks or light (a mesh made from an older version is out of date).
    pub version: u32,
}

/// The world in memory.
pub struct World {
    pub gen: Arc<Generator>,
    pub chunks: HashMap<ChunkPos, Slot>,
    /// Edited chunks that are not loaded now, in their save coding.
    pub stored: HashMap<ChunkPos, Vec<u8>>,
}

/// What a ray hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The block hit.
    pub pos: IVec3,
    /// The face's outward normal (the cell a placed block goes into is `pos + normal`).
    pub normal: IVec3,
    pub block: Block,
    pub distance: f32,
}

/// What changed when a block did: the sections to mesh again (chunk, section).
#[derive(Default, Debug)]
pub struct Changed {
    pub sections: HashSet<(ChunkPos, usize)>,
}

impl World {
    pub fn new(seed: u32) -> World {
        World { gen: Arc::new(Generator::new(seed)), chunks: HashMap::new(), stored: HashMap::new() }
    }

    pub fn seed(&self) -> u32 {
        self.gen.seed
    }

    /// The block at world (x, y, z): air above the world and where no chunk is loaded, bedrock below.
    pub fn block(&self, x: i32, y: i32, z: i32) -> Block {
        if y < 0 {
            return Block::BEDROCK;
        }
        if y >= H {
            return Block::AIR;
        }
        match self.chunks.get(&ChunkPos::of(x, z)) {
            Some(s) => s.data.blocks[idx(x.rem_euclid(W), y, z.rem_euclid(W))],
            None => Block::AIR,
        }
    }

    pub fn block_at(&self, p: IVec3) -> Block {
        self.block(p.x, p.y, p.z)
    }

    /// Whether the chunk holding column (x, z) is loaded and lit (the player can be there and change it).
    pub fn ready(&self, x: i32, z: i32) -> bool {
        self.chunks.get(&ChunkPos::of(x, z)).is_some_and(|s| s.lit)
    }

    /// (sky, block) light at a cell (full sky above the world).
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        if y >= H {
            return (15, 0);
        }
        if y < 0 {
            return (0, 0);
        }
        match self.chunks.get(&ChunkPos::of(x, z)) {
            Some(s) => {
                let l = s.data.light[idx(x.rem_euclid(W), y, z.rem_euclid(W))];
                (l >> 4, l & 15)
            }
            None => (15, 0),
        }
    }

    /// Puts `b` at (x, y, z) and works out what that changes: the light around it and the sections to mesh again. Nothing happens (and `None`
    /// comes back) outside the world or in a chunk that is not ready.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, b: Block) -> Option<Changed> {
        if !(0..H).contains(&y) || !self.ready(x, z) {
            return None;
        }
        let pos = ChunkPos::of(x, z);
        let slot = self.chunks.get_mut(&pos)?;
        let (lx, lz) = (x.rem_euclid(W), z.rem_euclid(W));
        if slot.data.get(lx, y, lz) == b {
            return Some(Changed::default());
        }
        let data = Arc::make_mut(&mut slot.data);
        data.set(lx, y, lz, b);
        data.edited = true;
        slot.version += 1;
        let mut changed = Changed::default();
        mark_around(&mut changed.sections, x, y, z);
        self.relight(x, z, &mut changed);
        for (p, s) in &changed.sections {
            if let Some(slot) = self.chunks.get_mut(p) {
                slot.dirty |= 1 << s;
            }
        }
        Some(changed)
    }

    /// Puts many blocks at once (water spreading): one relight over the box around them all. Cells outside the world or in chunks that are
    /// not ready are skipped.
    pub fn set_many(&mut self, cells: &[(IVec3, Block)]) -> Changed {
        let mut changed = Changed::default();
        let (mut lo, mut hi) = (IVec3::splat(i32::MAX), IVec3::splat(i32::MIN));
        for &(p, b) in cells {
            if !(0..H).contains(&p.y) || !self.ready(p.x, p.z) {
                continue;
            }
            let Some(slot) = self.chunks.get_mut(&ChunkPos::of(p.x, p.z)) else { continue };
            let (lx, lz) = (p.x.rem_euclid(W), p.z.rem_euclid(W));
            if slot.data.get(lx, p.y, lz) == b {
                continue;
            }
            let data = Arc::make_mut(&mut slot.data);
            data.set(lx, p.y, lz, b);
            data.edited = true;
            slot.version += 1;
            mark_around(&mut changed.sections, p.x, p.y, p.z);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        if lo.x <= hi.x {
            self.relight_box(lo.x, lo.z, hi.x, hi.z, &mut changed);
        }
        for (p, s) in &changed.sections {
            if let Some(slot) = self.chunks.get_mut(p) {
                slot.dirty |= 1 << s;
            }
        }
        changed
    }

    /// Works the light out again in the 31 columns around (x, z), all heights, holding the light just outside fixed (exact: see `light`).
    fn relight(&mut self, x: i32, z: i32, changed: &mut Changed) {
        self.relight_box(x, z, x, z, changed);
    }

    /// The same, for every column within 15 of the box (x0..=x1, z0..=z1).
    fn relight_box(&mut self, xa: i32, za: i32, xb: i32, zb: i32, changed: &mut Changed) {
        const R: i32 = 15;
        let (x0, z0) = (xa - R, za - R);
        let (n, nz) = (xb - xa + 2 * R + 1, zb - za + 2 * R + 1);
        let region = Region::compute(
            x0,
            z0,
            n,
            nz,
            |x, y, z| if self.chunks.contains_key(&ChunkPos::of(x, z)) { self.block(x, y, z) } else { Block::STONE },
            |x, y, z| self.light(x, y, z),
        );
        // Copy back what changed, chunk by chunk.
        let mut touched: Vec<ChunkPos> = Vec::new();
        for cz in ChunkPos::of(x0, z0).z..=ChunkPos::of(x0, z0 + nz - 1).z {
            for cx in ChunkPos::of(x0, z0).x..=ChunkPos::of(x0 + n - 1, z0).x {
                touched.push(ChunkPos::new(cx, cz));
            }
        }
        for p in touched {
            let Some(slot) = self.chunks.get_mut(&p) else { continue };
            if !slot.lit {
                continue;
            }
            let (ox, oz) = p.origin();
            let (ax, bx) = (x0.max(ox), (x0 + n).min(ox + W));
            let (az, bz) = (z0.max(oz), (z0 + nz).min(oz + W));
            let mut diff: Vec<(i32, i32, i32, u8)> = Vec::new();
            for y in 0..H {
                for wz in az..bz {
                    for wx in ax..bx {
                        let new = region.light(wx, y, wz);
                        let i = idx(wx - ox, y, wz - oz);
                        if slot.data.light[i] != new {
                            diff.push((wx, y, wz, new));
                        }
                    }
                }
            }
            if diff.is_empty() {
                continue;
            }
            let data = Arc::make_mut(&mut slot.data);
            for &(wx, y, wz, l) in &diff {
                data.light[idx(wx - ox, y, wz - oz)] = l;
            }
            slot.version += 1;
            for &(wx, y, wz, _) in &diff {
                mark_around(&mut changed.sections, wx, y, wz);
            }
        }
    }

    /// The first block the ray from `from` along `dir` (unit) meets within `reach` metres that can be aimed at (not air, not a liquid).
    pub fn raycast(&self, from: Vec3, dir: Vec3, reach: f32) -> Option<Hit> {
        let mut cell = from.floor().as_ivec3();
        let step = IVec3::new(dir.x.signum() as i32, dir.y.signum() as i32, dir.z.signum() as i32);
        let next = |c: i32, s: i32, o: f32, d: f32| {
            if d.abs() < 1e-9 {
                f32::INFINITY
            } else {
                let edge = if s > 0 { c as f32 + 1.0 } else { c as f32 };
                (edge - o) / d
            }
        };
        let mut t_max = Vec3::new(next(cell.x, step.x, from.x, dir.x), next(cell.y, step.y, from.y, dir.y), next(cell.z, step.z, from.z, dir.z));
        let t_delta = Vec3::new(
            if dir.x != 0.0 { 1.0 / dir.x.abs() } else { f32::INFINITY },
            if dir.y != 0.0 { 1.0 / dir.y.abs() } else { f32::INFINITY },
            if dir.z != 0.0 { 1.0 / dir.z.abs() } else { f32::INFINITY },
        );
        let mut normal = IVec3::ZERO;
        let mut t = 0.0;
        while t <= reach {
            let b = self.block_at(cell);
            if !b.is_air() && !b.is_liquid() {
                let (lo, hi) = selection_box(b);
                if let Some(d) = ray_box(from, dir, cell.as_vec3() + lo, cell.as_vec3() + hi) {
                    if d.0 <= reach {
                        let n = if lo == Vec3::ZERO && hi == Vec3::ONE { normal } else { d.1 };
                        return Some(Hit { pos: cell, normal: n, block: b, distance: d.0 });
                    }
                }
            }
            if t_max.x < t_max.y && t_max.x < t_max.z {
                cell.x += step.x;
                t = t_max.x;
                t_max.x += t_delta.x;
                normal = IVec3::new(-step.x, 0, 0);
            } else if t_max.y < t_max.z {
                cell.y += step.y;
                t = t_max.y;
                t_max.y += t_delta.y;
                normal = IVec3::new(0, -step.y, 0);
            } else {
                cell.z += step.z;
                t = t_max.z;
                t_max.z += t_delta.z;
                normal = IVec3::new(0, 0, -step.z);
            }
        }
        None
    }

    /// The y of the highest block a player can stand on in column (x, z), or `None` when nothing is loaded there.
    pub fn ground(&self, x: i32, z: i32) -> Option<i32> {
        let slot = self.chunks.get(&ChunkPos::of(x, z))?;
        let (lx, lz) = (x.rem_euclid(W), z.rem_euclid(W));
        (0..H).rev().find(|&y| slot.data.get(lx, y, lz).solid())
    }

    /// Every edited chunk, loaded or not, in its save coding.
    pub fn edited_chunks(&self) -> Vec<(ChunkPos, Vec<u8>)> {
        let mut v: Vec<(ChunkPos, Vec<u8>)> = self.stored.iter().map(|(p, b)| (*p, b.clone())).collect();
        for (p, s) in &self.chunks {
            if s.data.edited {
                v.push((*p, s.data.encode()));
            }
        }
        v.sort_by_key(|(p, _)| *p);
        v
    }

    /// Forgets a chunk, keeping its edits.
    pub fn unload(&mut self, p: ChunkPos) {
        if let Some(s) = self.chunks.remove(&p) {
            if s.data.edited {
                self.stored.insert(p, s.data.encode());
            }
        }
    }

    /// The chunk and its eight neighbours (for light or meshes), when all are loaded.
    pub fn hood(&self, p: ChunkPos) -> Option<[Arc<ChunkData>; 9]> {
        let mut v: Vec<Arc<ChunkData>> = Vec::with_capacity(9);
        for dz in -1..=1 {
            for dx in -1..=1 {
                v.push(self.chunks.get(&p.offset(dx, dz))?.data.clone());
            }
        }
        v.try_into().ok()
    }
}

/// The box a ray aims at in a block's cell (plants and torches are smaller than their cell).
pub fn selection_box(b: Block) -> (Vec3, Vec3) {
    match b.info().shape {
        crate::block::Shape::Plant(_) => (Vec3::new(0.2, 0.0, 0.2), Vec3::new(0.8, 0.7, 0.8)),
        crate::block::Shape::Torch(None) => (Vec3::new(0.38, 0.0, 0.38), Vec3::new(0.62, 0.65, 0.62)),
        crate::block::Shape::Torch(Some(_)) => (Vec3::new(0.3, 0.15, 0.3), Vec3::new(0.7, 0.85, 0.7)),
        _ => (Vec3::ZERO, Vec3::ONE),
    }
}

/// Where a ray enters a box (distance) and the face it enters through.
fn ray_box(o: Vec3, d: Vec3, lo: Vec3, hi: Vec3) -> Option<(f32, IVec3)> {
    let inv = Vec3::ONE / d;
    let t1 = (lo - o) * inv;
    let t2 = (hi - o) * inv;
    let tmin = t1.min(t2);
    let tmax = t1.max(t2);
    let near = tmin.max_element();
    let far = tmax.min_element();
    if far < near.max(0.0) {
        return None;
    }
    let n = if near == tmin.x {
        IVec3::new(-d.x.signum() as i32, 0, 0)
    } else if near == tmin.y {
        IVec3::new(0, -d.y.signum() as i32, 0)
    } else {
        IVec3::new(0, 0, -d.z.signum() as i32)
    };
    Some((near.max(0.0), n))
}

/// Marks the sections a change at (x, y, z) shows in: its own, and its neighbours' where it sits on their edge (their faces and corners read it).
fn mark_around(set: &mut HashSet<(ChunkPos, usize)>, x: i32, y: i32, z: i32) {
    let edge = |v: i32, size: i32| {
        let r = v.rem_euclid(size);
        (if r == 0 { -1 } else { 0 }, if r == size - 1 { 1 } else { 0 })
    };
    let (x_lo, x_hi) = edge(x, W);
    let (z_lo, z_hi) = edge(z, W);
    let (y_lo, y_hi) = edge(y, SECTION);
    for dz in z_lo..=z_hi {
        for dy in y_lo..=y_hi {
            for dx in x_lo..=x_hi {
                let ny = y + dy;
                if !(0..H).contains(&ny) {
                    continue;
                }
                let s = (ny / SECTION) as usize;
                if s < SECTIONS {
                    set.insert((ChunkPos::of(x + dx, z + dz), s));
                }
            }
        }
    }
}

/// A chunk's light worked out from it and its eight neighbours (`hood` in [`World::hood`] order).
pub fn light_chunk(hood: &[Arc<ChunkData>; 9]) -> Vec<u8> {
    let block = |x: i32, y: i32, z: i32| {
        let (cx, cz) = (x.div_euclid(W), z.div_euclid(W));
        if !(-1..=1).contains(&cx) || !(-1..=1).contains(&cz) {
            return Block::STONE;
        }
        hood[((cx + 1) + 3 * (cz + 1)) as usize].get(x.rem_euclid(W), y, z.rem_euclid(W))
    };
    // Light from 15 or more cells away cannot reach the middle chunk: the box needs only that much around it.
    let r = Region::compute(-15, -15, W + 30, W + 30, block, |_, _, _| (0, 0));
    let mut out = vec![0u8; crate::chunk::CELLS];
    for y in 0..H {
        for z in 0..W {
            for x in 0..W {
                out[idx(x, y, z)] = r.light(x, y, z);
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A world with chunks within `r` of the origin generated and lit (no workers).
    pub fn loaded(seed: u32, r: i32) -> World {
        let mut w = World::new(seed);
        for cz in -r - 1..=r + 1 {
            for cx in -r - 1..=r + 1 {
                let p = ChunkPos::new(cx, cz);
                w.chunks.insert(p, Slot { data: Arc::new(w.gen.generate(p)), lit: false, dirty: 0xff, meshed: false, version: 0 });
            }
        }
        for cz in -r..=r {
            for cx in -r..=r {
                let p = ChunkPos::new(cx, cz);
                let light = light_chunk(&w.hood(p).unwrap());
                let s = w.chunks.get_mut(&p).unwrap();
                Arc::make_mut(&mut s.data).light = light.into_boxed_slice();
                s.lit = true;
            }
        }
        w
    }

    #[test]
    fn the_ray_finds_the_ground_and_the_face_it_came_through() {
        let w = loaded(3, 1);
        let x = 8;
        let z = 8;
        let top = w.ground(x, z).unwrap();
        let hit = w.raycast(Vec3::new(x as f32 + 0.5, top as f32 + 3.0, z as f32 + 0.5), Vec3::NEG_Y, 10.0).unwrap();
        assert!(hit.pos.y >= top, "{hit:?} top {top}");
        assert_eq!(hit.normal, IVec3::Y);
        assert!(w.raycast(Vec3::new(0.5, 200.0, 0.5), Vec3::Y, 10.0).is_none());
        // Sideways into a wall: the face toward the ray.
        let mut w = w;
        w.set_block(x, top + 6, z + 2, Block::STONE).unwrap();
        let h = w.raycast(Vec3::new(x as f32 + 0.5, top as f32 + 6.5, z as f32 - 1.5), Vec3::Z, 10.0).unwrap();
        assert_eq!((h.pos, h.normal), (IVec3::new(x, top + 6, z + 2), IVec3::NEG_Z), "{h:?}");
        assert!((h.distance - 3.5).abs() < 1e-4);
        // A flower is aimed at by its own small box.
        w.set_block(x, top + 1, z, Block::POPPY).unwrap();
        let near_edge = w.raycast(Vec3::new(x as f32 + 0.05, top as f32 + 3.0, z as f32 + 0.05), Vec3::NEG_Y, 10.0).unwrap();
        assert_ne!(near_edge.block, Block::POPPY, "the corner of the flower's cell is not the flower");
    }

    #[test]
    fn digging_down_lets_the_sky_in_and_a_torch_lights_a_pit() {
        // Seed 1 starts on land at the origin.
        let mut w = loaded(1, 1);
        // A dry column with open sky over it.
        let (x, z) = (0..16)
            .flat_map(|z| (0..16).map(move |x| (x, z)))
            .find(|&(x, z)| {
                let top = w.ground(x, z).unwrap();
                w.block(x, top + 1, z).is_air() && w.light(x, top + 1, z).0 == 15 && w.block(x, top, z).cull()
            })
            .expect("a dry column");
        let top = w.ground(x, z).unwrap();
        let deep = top - 6;
        assert!(w.light(x, deep, z).0 < 15);
        for y in deep..=top {
            w.set_block(x, y, z, Block::AIR).unwrap();
        }
        assert_eq!(w.light(x, deep, z).0, 15, "an open shaft is lit by the sky all the way down");
        // Cover it and the shaft goes dark but for what leaks in sideways.
        w.set_block(x, top + 1, z, Block::STONE).unwrap();
        assert!(w.light(x, deep, z).0 < 15);
        let changed = w.set_block(x, deep, z, Block::TORCH).unwrap();
        assert_eq!(w.light(x, deep, z).1, 14);
        assert_eq!(w.light(x, deep + 3, z).1, 11);
        assert!(!changed.sections.is_empty());
        w.set_block(x, deep, z, Block::AIR).unwrap();
        assert_eq!(w.light(x, deep + 3, z).1, 0, "taking the torch away takes its light");
    }

    #[test]
    fn a_change_on_a_chunk_edge_marks_the_neighbours_sections_too() {
        let mut set = HashSet::new();
        mark_around(&mut set, 16, 32, 5);
        assert!(set.contains(&(ChunkPos::new(1, 0), 2)));
        assert!(set.contains(&(ChunkPos::new(0, 0), 2)), "x 16 is the edge of chunk 1: chunk 0 reads it");
        assert!(set.contains(&(ChunkPos::new(1, 0), 1)), "y 32 is a section's bottom: the section below reads it");
        assert_eq!(set.len(), 4);
    }

    #[test]
    fn a_chunks_light_agrees_with_its_neighbours_at_the_border() {
        let w = loaded(9, 1);
        let mut differences = 0;
        for y in 40..100 {
            for z in 0..W {
                let a = w.light(15, y, z);
                let b = w.light(16, y, z);
                let block_a = w.block(15, y, z);
                let block_b = w.block(16, y, z);
                if !block_a.solid() && !block_b.solid() && (a.0 as i32 - b.0 as i32).abs() > 2 {
                    differences += 1;
                }
            }
        }
        assert_eq!(differences, 0, "open cells either side of a border are within a step or two of each other");
    }
}
