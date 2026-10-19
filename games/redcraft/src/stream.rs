//! Streaming the world around the player: worker threads generate chunks, work out their light and mesh their sections, nearest first, and the
//! main thread hands the meshes to the renderer and forgets what is far behind.
//!
//! A chunk goes through three steps, each needing its neighbours a step behind it: generated (ring view + 2), lit (ring view + 1, once its eight
//! neighbours are generated) and meshed (ring view, once its eight neighbours are lit: a face on a border reads the light across it).

use crate::block::Block;
use crate::chunk::{ChunkData, ChunkPos, SECTIONS};
use crate::gen::Generator;
use crate::mesh::{mesh_section, Hood, SectionMesh};
use crate::world::{light_chunk, Changed, Slot, World};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

enum Job {
    Generate { pos: ChunkPos, stored: Option<Vec<u8>> },
    Light { pos: ChunkPos, hood: Box<[Arc<ChunkData>; 9]> },
    Mesh { pos: ChunkPos, sections: u8, hood: Box<[Arc<ChunkData>; 9]>, seq: u64 },
}

enum Done {
    Generated { pos: ChunkPos, data: ChunkData },
    Lit { pos: ChunkPos, light: Vec<u8> },
    Meshed { pos: ChunkPos, meshes: Vec<(usize, SectionMesh)>, seq: u64 },
}

fn run(gen: &Generator, job: Job) -> Done {
    match job {
        Job::Generate { pos, stored } => {
            let data = stored.and_then(|b| ChunkData::decode(&b)).map(|mut c| {
                c.edited = true;
                c
            });
            Done::Generated { pos, data: data.unwrap_or_else(|| gen.generate(pos)) }
        }
        Job::Light { pos, hood } => Done::Lit { pos, light: light_chunk(&hood) },
        Job::Mesh { pos, sections, hood, seq } => Done::Meshed { pos, meshes: mesh_sections(&hood, sections), seq },
    }
}

fn mesh_sections(hood: &[Arc<ChunkData>; 9], sections: u8) -> Vec<(usize, SectionMesh)> {
    let h = Hood { chunks: std::array::from_fn(|i| hood[i].as_ref()) };
    (0..SECTIONS)
        .filter(|s| sections & (1 << s) != 0)
        .map(|s| (s, if hood[4].section_empty(s) { SectionMesh::default() } else { mesh_section(&h, s) }))
        .collect()
}

/// What happened to the meshes since the renderer last looked.
#[derive(Default)]
pub struct MeshUpdates {
    /// Sections to (re)upload: an empty mesh clears the section.
    pub fresh: Vec<(ChunkPos, usize, SectionMesh)>,
    /// Chunks that are gone.
    pub gone: Vec<ChunkPos>,
}

/// Counters for the debug screen and the tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub loaded: usize,
    pub lit: usize,
    pub meshed: usize,
    pub in_flight: usize,
}

/// The world and its streaming.
pub struct Terrain {
    pub world: World,
    /// How far the world reaches, chunks.
    pub view: i32,
    jobs: Option<Sender<Job>>,
    done: Receiver<Done>,
    done_tx: Sender<Done>,
    workers: usize,
    in_flight: HashSet<(ChunkPos, u8)>,
    seq: u64,
    applied: HashMap<(ChunkPos, usize), u64>,
    pub updates: MeshUpdates,
}

impl Terrain {
    /// A terrain for `world` reaching `view` chunks, built by `workers` threads (0: everything happens inside `update`, for still pictures and tests).
    pub fn new(world: World, view: i32, workers: usize) -> Terrain {
        let (done_tx, done) = channel::<Done>();
        let jobs = if workers > 0 {
            let (tx, rx) = channel::<Job>();
            let rx = Arc::new(Mutex::new(rx));
            for k in 0..workers {
                let rx = rx.clone();
                let out = done_tx.clone();
                let gen = world.gen.clone();
                std::thread::Builder::new()
                    .name(format!("redcraft-world-{k}"))
                    .spawn(move || loop {
                        let job = match rx.lock() {
                            Ok(r) => r.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else { return };
                        if out.send(run(&gen, job)).is_err() {
                            return;
                        }
                    })
                    .expect("spawn a world worker");
            }
            Some(tx)
        } else {
            None
        };
        Terrain { world, view, jobs, done, done_tx, workers, in_flight: HashSet::new(), seq: 0, applied: HashMap::new(), updates: MeshUpdates::default() }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            loaded: self.world.chunks.len(),
            lit: self.world.chunks.values().filter(|s| s.lit).count(),
            meshed: self.world.chunks.values().filter(|s| s.meshed).count(),
            in_flight: self.in_flight.len(),
        }
    }

    fn submit(&mut self, key: (ChunkPos, u8), job: Job) {
        self.in_flight.insert(key);
        match &self.jobs {
            Some(tx) => {
                if tx.send(job).is_err() {
                    self.in_flight.remove(&key);
                }
            }
            None => {
                let gen = self.world.gen.clone();
                let _ = self.done_tx.send(run(&gen, job));
            }
        }
    }

    fn apply(&mut self, d: Done) {
        match d {
            Done::Generated { pos, data } => {
                self.in_flight.remove(&(pos, 0));
                self.world.stored.remove(&pos);
                self.world.chunks.insert(pos, Slot { data: Arc::new(data), lit: false, dirty: 0xff, meshed: false, version: 0 });
            }
            Done::Lit { pos, light } => {
                self.in_flight.remove(&(pos, 1));
                if let Some(s) = self.world.chunks.get_mut(&pos) {
                    Arc::make_mut(&mut s.data).light = light.into_boxed_slice();
                    s.lit = true;
                    s.version += 1;
                    s.dirty = 0xff;
                }
            }
            Done::Meshed { pos, meshes, seq } => {
                self.in_flight.remove(&(pos, 2));
                if !self.world.chunks.contains_key(&pos) {
                    return;
                }
                for (s, m) in meshes {
                    let last = self.applied.entry((pos, s)).or_insert(0);
                    if seq >= *last {
                        *last = seq;
                        self.updates.fresh.push((pos, s, m));
                    }
                }
                if let Some(slot) = self.world.chunks.get_mut(&pos) {
                    slot.meshed = true;
                }
            }
        }
    }

    /// Takes in what the workers finished and gives them the next jobs around `eye`; forgets what is far away.
    pub fn update(&mut self, eye: Vec3) {
        while let Ok(d) = self.done.try_recv() {
            self.apply(d);
        }
        let centre = ChunkPos::of(eye.x.floor() as i32, eye.z.floor() as i32);
        // Forget what is behind.
        let far: Vec<ChunkPos> = self.world.chunks.keys().copied().filter(|p| p.ring(centre) > self.view + 3).collect();
        for p in far {
            self.world.unload(p);
            self.applied.retain(|(c, _), _| *c != p);
            self.updates.gone.push(p);
        }
        let cap = if self.workers == 0 { usize::MAX } else { self.workers * 3 };
        let mut wanted: Vec<(i32, ChunkPos)> = Vec::new();
        let r = self.view + 2;
        for dz in -r..=r {
            for dx in -r..=r {
                let p = centre.offset(dx, dz);
                wanted.push((dx * dx + dz * dz, p));
            }
        }
        wanted.sort_by_key(|(d, p)| (*d, p.x, p.z));
        let circle = |p: ChunkPos, ring: i32| {
            let (dx, dz) = (p.x - centre.x, p.z - centre.z);
            dx * dx + dz * dz <= (ring * ring + ring)
        };
        for (_, p) in wanted {
            if self.in_flight.len() >= cap {
                break;
            }
            let ring = p.ring(centre);
            // Generated and lit in squares (a lit chunk's neighbours are all generated), meshed in a circle (its neighbours are all lit).
            match self.world.chunks.get(&p) {
                None => {
                    if ring <= self.view + 2 && !self.in_flight.contains(&(p, 0)) {
                        let stored = self.world.stored.get(&p).cloned();
                        self.submit((p, 0), Job::Generate { pos: p, stored });
                    }
                }
                Some(slot) if !slot.lit => {
                    if ring <= self.view + 1 && !self.in_flight.contains(&(p, 1)) {
                        if let Some(hood) = self.world.hood(p) {
                            self.submit((p, 1), Job::Light { pos: p, hood: Box::new(hood) });
                        }
                    }
                }
                Some(slot) => {
                    if slot.dirty != 0 && circle(p, self.view) && !self.in_flight.contains(&(p, 2)) && self.neighbours_lit(p) {
                        let sections = slot.dirty;
                        if let Some(hood) = self.world.hood(p) {
                            self.seq += 1;
                            let seq = self.seq;
                            if let Some(s) = self.world.chunks.get_mut(&p) {
                                s.dirty = 0;
                            }
                            self.submit((p, 2), Job::Mesh { pos: p, sections, hood: Box::new(hood), seq });
                        }
                    }
                }
            }
        }
        if self.workers == 0 {
            while let Ok(d) = self.done.try_recv() {
                self.apply(d);
            }
        }
    }

    fn neighbours_lit(&self, p: ChunkPos) -> bool {
        (-1..=1).all(|dz| (-1..=1).all(|dx| self.world.chunks.get(&p.offset(dx, dz)).is_some_and(|s| s.lit)))
    }

    /// Runs until every chunk within view of `eye` is meshed (a still picture, the first frame of a game, a test). Gives up after `secs`.
    pub fn settle(&mut self, eye: Vec3, secs: f32) {
        let start = std::time::Instant::now();
        let centre = ChunkPos::of(eye.x.floor() as i32, eye.z.floor() as i32);
        loop {
            self.update(eye);
            let done = (-self.view..=self.view).all(|dz| {
                (-self.view..=self.view).all(|dx| {
                    let p = centre.offset(dx, dz);
                    dx * dx + dz * dz > self.view * self.view + self.view || self.world.chunks.get(&p).is_some_and(|s| s.meshed && s.dirty == 0)
                })
            });
            if done && self.in_flight.is_empty() {
                return;
            }
            if start.elapsed().as_secs_f32() > secs {
                eprintln!("redcraft: the world around {eye} was not ready after {secs} s ({:?})", self.stats());
                return;
            }
            if self.workers > 0 {
                if let Ok(d) = self.done.recv_timeout(std::time::Duration::from_millis(20)) {
                    self.apply(d);
                }
            }
        }
    }

    /// Whether every chunk within `r` of `eye` is lit (the player may stand there).
    pub fn ready_around(&self, eye: Vec3, r: i32) -> bool {
        let centre = ChunkPos::of(eye.x.floor() as i32, eye.z.floor() as i32);
        (-r..=r).all(|dz| (-r..=r).all(|dx| self.world.chunks.get(&centre.offset(dx, dz)).is_some_and(|s| s.lit && s.meshed)))
    }

    /// Changes a block; the sections it shows in are meshed at once (the rest of what its light changed follows from the workers).
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, b: Block) -> Option<Changed> {
        let changed = self.world.set_block(x, y, z, b)?;
        // The cell itself and what touches it: now, so the player never sees a hole where a block was put.
        let mut now: HashSet<(ChunkPos, usize)> = HashSet::new();
        for (dx, dy, dz) in [(0, 0, 0), (1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            if (0..crate::chunk::H).contains(&ny) {
                now.insert((ChunkPos::of(nx, nz), (ny / crate::chunk::SECTION) as usize));
            }
        }
        for (p, s) in now {
            if !changed.sections.contains(&(p, s)) {
                continue;
            }
            if let Some(hood) = self.world.hood(p) {
                self.seq += 1;
                let meshes = mesh_sections(&hood, 1 << s);
                if let Some(slot) = self.world.chunks.get_mut(&p) {
                    slot.dirty &= !(1 << s);
                }
                for (s, m) in meshes {
                    self.applied.insert((p, s), self.seq);
                    self.updates.fresh.push((p, s, m));
                }
            }
        }
        Some(changed)
    }

    /// Changes many blocks at once (water spreading); the workers mesh what changed.
    pub fn set_many(&mut self, cells: &[(glam::IVec3, Block)]) -> Changed {
        self.world.set_many(cells)
    }

    /// Takes the meshes made since the last call.
    pub fn take_updates(&mut self) -> MeshUpdates {
        std::mem::take(&mut self.updates)
    }

    /// The world column's chunk origin for a position (for the renderer).
    pub fn section_origin(p: ChunkPos, s: usize) -> Vec3 {
        let (x, z) = p.origin();
        Vec3::new(x as f32, (s as i32 * crate::chunk::SECTION) as f32, z as f32)
    }
}

impl Drop for Terrain {
    fn drop(&mut self) {
        // Closing the job channel ends the workers.
        self.jobs = None;
    }
}

/// How many worker threads a live game uses.
pub fn live_workers() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).clamp(1, 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settling_meshes_every_chunk_in_view_and_walking_away_unloads() {
        let mut t = Terrain::new(World::new(1), 2, 2);
        t.settle(Vec3::new(8.0, 80.0, 8.0), 60.0);
        let s = t.stats();
        assert!(s.meshed >= 13, "{s:?}");
        let u = t.take_updates();
        assert!(u.fresh.iter().any(|(_, _, m)| !m.is_empty()));
        t.update(Vec3::new(8.0 + 16.0 * 10.0, 80.0, 8.0));
        assert!(!t.take_updates().gone.is_empty(), "chunks far behind are let go");
    }

    #[test]
    fn an_edit_is_meshed_at_once_and_survives_an_unload() {
        let mut t = Terrain::new(World::new(4), 1, 0);
        t.settle(Vec3::new(8.0, 80.0, 8.0), 60.0);
        t.take_updates();
        let top = t.world.ground(8, 8).unwrap();
        t.set_block(8, top + 1, 8, Block::GLASS).unwrap();
        let u = t.take_updates();
        assert!(u.fresh.iter().any(|(_, _, m)| !m.glass.indices.is_empty()), "the glass is in a mesh straight away");
        // Away and back: the edit is kept.
        t.update(Vec3::new(8.0 + 16.0 * 20.0, 80.0, 8.0));
        assert!(!t.world.chunks.contains_key(&ChunkPos::new(0, 0)));
        assert!(t.world.stored.contains_key(&ChunkPos::new(0, 0)));
        t.settle(Vec3::new(8.0, 80.0, 8.0), 60.0);
        assert_eq!(t.world.block(8, top + 1, 8), Block::GLASS);
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// `cargo test -p redcraft --release timing -- --ignored --nocapture`: what a chunk costs to make.
    #[test]
    #[ignore]
    fn what_a_chunk_costs() {
        let world = World::new(7);
        let t = std::time::Instant::now();
        let n = 9;
        let chunks: Vec<ChunkData> = (0..n).map(|i| world.gen.generate(ChunkPos::new(i % 3 - 1, i / 3 - 1))).collect();
        let gen = t.elapsed().as_secs_f32() / n as f32;
        let hood: [Arc<ChunkData>; 9] = std::array::from_fn(|i| Arc::new(chunks[i].clone()));
        let t = std::time::Instant::now();
        let light = light_chunk(&hood);
        let lit = t.elapsed().as_secs_f32();
        let mut lit_hood = hood.clone();
        for c in lit_hood.iter_mut() {
            Arc::make_mut(c).light = light.clone().into_boxed_slice();
        }
        let t = std::time::Instant::now();
        let meshes = mesh_sections(&lit_hood, 0xff);
        let meshed = t.elapsed().as_secs_f32();
        let tris: usize = meshes.iter().map(|(_, m)| m.triangles()).sum();
        println!("generate {:.1} ms, light {:.1} ms, mesh {:.1} ms ({} triangles)", gen * 1000.0, lit * 1000.0, meshed * 1000.0, tris);
    }
}

#[cfg(test)]
mod settle_probe {
    use super::*;

    #[test]
    #[ignore]
    fn settle_probe() {
        let mut t = Terrain::new(World::new(1), 6, 3);
        let s = std::time::Instant::now();
        t.settle(Vec3::new(0.0, 61.6, 0.0), 30.0);
        println!("{:.1}s {:?}", s.elapsed().as_secs_f32(), t.stats());
        let centre = ChunkPos::of(0, 0);
        for (p, slot) in &t.world.chunks {
            let (dx, dz) = (p.x - centre.x, p.z - centre.z);
            if dx * dx + dz * dz <= 42 && !(slot.meshed && slot.dirty == 0) {
                println!("not done: {p:?} lit {} meshed {} dirty {:#x}", slot.lit, slot.meshed, slot.dirty);
            }
        }
        println!("in flight {:?}", t.in_flight);
    }
}

#[cfg(test)]
mod breakdown {
    use super::*;
    use crate::block::Shape;

    /// `cargo test -p redcraft breakdown -- --ignored --nocapture`: where a chunk's triangles go.
    #[test]
    #[ignore]
    fn triangles_by_kind() {
        let mut t = Terrain::new(World::new(1), 1, 0);
        t.settle(Vec3::new(8.0, 80.0, 8.0), 60.0);
        let hood = t.world.hood(ChunkPos::new(0, 0)).unwrap();
        let all: usize = mesh_sections(&hood, 0xff).iter().map(|(_, m)| m.triangles()).sum();
        // The same chunk with its plants and leaves taken out, one kind at a time.
        let without = |pred: &dyn Fn(Block) -> bool| {
            let mut h = hood.clone();
            let c = Arc::make_mut(&mut h[4]);
            for b in c.blocks.iter_mut() {
                if pred(*b) {
                    *b = Block::AIR;
                }
            }
            mesh_sections(&h, 0xff).iter().map(|(_, m)| m.triangles()).sum::<usize>()
        };
        let no_plants = without(&|b| matches!(b.info().shape, Shape::Plant(_)));
        let no_leaves = without(&|b| matches!(b, Block::OAK_LEAVES | Block::SPRUCE_LEAVES | Block::BIRCH_LEAVES));
        let counts: std::collections::BTreeMap<&str, usize> = hood[4].blocks.iter().fold(Default::default(), |mut m, b| {
            *m.entry(b.info().name).or_default() += 1;
            m
        });
        println!("all {all}, plants {}, leaves {}; {counts:?}", all - no_plants, all as i64 - no_leaves as i64);
    }
}
