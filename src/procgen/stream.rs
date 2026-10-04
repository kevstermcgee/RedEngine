//! Streaming: which chunks exist around the viewer, baked on worker threads and handed to the renderer a few at a time.
//!
//! [`Streamer::update`] is called every frame with the viewer's position. It works out which chunks should be loaded (every chunk whose centre is within
//! the view distance), asks for the nearest missing ones first, collects the ones the workers have finished, and forgets those that fell behind. What it
//! returns is a delta ([`Update`]): chunks to add or replace (their new level of detail) and chunks to drop. The renderer uploads those and nothing else, so
//! walking costs a couple of chunk uploads per second, not a rebuild.
//!
//! The chunk a viewer stands in is always asked for first, and the work is spread over the workers so no frame waits on generation. With no workers
//! ([`Streamer::synchronous`]) [`Streamer::fill`] builds everything on the spot, which is what a still frame wants.
//!
//! A chunk changes level of detail as the viewer comes nearer or goes farther ([`Lod`]); a margin keeps one standing on a boundary from rebuilding the same
//! chunk back and forth.

use super::chunk::{build_chunk, Chunk, Library, Lod};
use super::world::{ChunkId, Config, World, CHUNK};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// How far to see.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Chunks whose centre is nearer than this many metres are loaded.
    pub distance: f32,
}

impl Default for View {
    fn default() -> Self {
        View { distance: 270.0 }
    }
}

/// What changed since the last update.
#[derive(Default)]
pub struct Update {
    /// New chunks, and chunks rebuilt at another level of detail (replace any chunk with the same id).
    pub added: Vec<Chunk>,
    /// Chunks to forget.
    pub removed: Vec<ChunkId>,
}

/// Counters for tests and the debug overlay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Chunks currently loaded.
    pub loaded: usize,
    /// Chunks being built right now.
    pub pending: usize,
    /// Chunks built, ever.
    pub built: u64,
    /// Chunks forgotten, ever.
    pub dropped: u64,
}

struct Job {
    id: ChunkId,
    lod: Lod,
}

struct Workers {
    jobs: Sender<Job>,
    done: Receiver<Chunk>,
}

/// The chunks around a viewer.
pub struct Streamer {
    world: Arc<World>,
    lib: Arc<Library>,
    view: View,
    loaded: HashMap<ChunkId, Lod>,
    pending: HashMap<ChunkId, Lod>,
    workers: Option<Workers>,
    stats: Stats,
}

/// Metres a chunk may sit past a level-of-detail boundary before it changes (stops flicker for a viewer standing on one).
const LOD_MARGIN: f32 = 0.3;

impl Streamer {
    /// A streamer that builds chunks on `threads` worker threads (0 builds them inside [`Streamer::update`]).
    pub fn new(cfg: Config, view: View, threads: usize) -> Streamer {
        let world = Arc::new(World::new(cfg));
        let lib = Arc::new(Library::new());
        let workers = (threads > 0).then(|| {
            let (jobs, job_rx) = channel::<Job>();
            let (done_tx, done) = channel::<Chunk>();
            let job_rx = Arc::new(Mutex::new(job_rx));
            for _ in 0..threads {
                let (world, lib, job_rx, done_tx) = (world.clone(), lib.clone(), job_rx.clone(), done_tx.clone());
                std::thread::spawn(move || loop {
                    let job = { job_rx.lock().unwrap_or_else(|e| e.into_inner()).recv() };
                    let Ok(job) = job else { break };
                    if done_tx.send(build_chunk(&world, &lib, job.id, job.lod)).is_err() {
                        break;
                    }
                });
            }
            Workers { jobs, done }
        });
        Streamer { world, lib, view, loaded: HashMap::new(), pending: HashMap::new(), workers, stats: Stats::default() }
    }

    /// A streamer with no worker threads: [`Streamer::update`] builds a few chunks per call, [`Streamer::fill`] builds all.
    pub fn synchronous(cfg: Config, view: View) -> Streamer {
        Streamer::new(cfg, view, 0)
    }

    /// Changes how far the world reaches (chunks beyond it are dropped on the next update).
    pub fn set_view(&mut self, view: View) {
        self.view = view;
    }

    /// The generator.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// The model library (shared with the workers).
    pub fn library(&self) -> &Library {
        &self.lib
    }

    /// Current counters.
    pub fn stats(&self) -> Stats {
        Stats { loaded: self.loaded.len(), pending: self.pending.len(), ..self.stats }
    }

    /// The chunks that should exist for a viewer at `eye`, with the detail each should have, nearest first.
    pub fn wanted(&self, eye: (f64, f64)) -> Vec<(ChunkId, Lod, f32)> {
        self.wanted_many(&[eye])
    }

    /// [`Streamer::wanted`] for several viewers at once (split-screen): every chunk any of them needs, at the detail the *nearest* viewer calls for.
    pub fn wanted_many(&self, eyes: &[(f64, f64)]) -> Vec<(ChunkId, Lod, f32)> {
        let reach = (self.view.distance as f64 / CHUNK).ceil() as i32 + 1;
        let mut best: HashMap<ChunkId, f32> = HashMap::new();
        for &eye in eyes {
            let here = ChunkId::at(eye.0, eye.1);
            for dz in -reach..=reach {
                for dx in -reach..=reach {
                    let id = ChunkId { x: here.x + dx, z: here.z + dz };
                    let (cx, cz) = id.centre();
                    let d = ((cx - eye.0).powi(2) + (cz - eye.1).powi(2)).sqrt();
                    if d > self.view.distance as f64 {
                        continue;
                    }
                    let chunks = (d / CHUNK) as f32;
                    best.entry(id).and_modify(|b| *b = b.min(chunks)).or_insert(chunks);
                }
            }
        }
        let mut out: Vec<(ChunkId, Lod, f32)> = best
            .into_iter()
            .map(|(id, chunks)| {
                let lod = match self.loaded.get(&id).or(self.pending.get(&id)) {
                    // Keep what is there unless the viewer is clearly in another band.
                    Some(&have) if Lod::for_distance(chunks - LOD_MARGIN) <= have && have <= Lod::for_distance(chunks + LOD_MARGIN) => have,
                    _ => Lod::for_distance(chunks),
                };
                (id, lod, chunks)
            })
            .collect();
        out.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
        out
    }

    /// Advances the stream: asks for what is missing (at most `max_new` chunks this call), collects what is finished and forgets what is behind.
    pub fn update_with(&mut self, eye: (f64, f64), max_new: usize) -> Update {
        self.update_many(&[eye], max_new)
    }

    /// [`Streamer::update_with`] for several viewers at once.
    pub fn update_many(&mut self, eyes: &[(f64, f64)], max_new: usize) -> Update {
        let wanted = self.wanted_many(eyes);
        let want: HashSet<ChunkId> = wanted.iter().map(|w| w.0).collect();
        let mut update = Update::default();
        // Forget what is out of range, and abandon what is no longer wanted (a finished chunk nobody wants is dropped on arrival).
        let gone: Vec<ChunkId> = self.loaded.keys().filter(|id| !want.contains(id)).copied().collect();
        for id in gone {
            self.loaded.remove(&id);
            self.stats.dropped += 1;
            update.removed.push(id);
        }
        self.pending.retain(|id, _| want.contains(id));
        // Collect finished work.
        if let Some(w) = &self.workers {
            while let Ok(chunk) = w.done.try_recv() {
                if self.pending.get(&chunk.id) == Some(&chunk.lod) {
                    self.pending.remove(&chunk.id);
                    self.loaded.insert(chunk.id, chunk.lod);
                    self.stats.built += 1;
                    update.added.push(chunk);
                }
            }
        }
        // Ask for the nearest chunks that are missing or at the wrong detail.
        let mut asked = 0;
        for (id, lod, _) in &wanted {
            if asked >= max_new {
                break;
            }
            if self.loaded.get(id) == Some(lod) || self.pending.get(id) == Some(lod) {
                continue;
            }
            asked += 1;
            match &self.workers {
                Some(w) => {
                    self.pending.insert(*id, *lod);
                    let _ = w.jobs.send(Job { id: *id, lod: *lod });
                }
                None => {
                    self.loaded.insert(*id, *lod);
                    self.stats.built += 1;
                    update.added.push(build_chunk(&self.world, &self.lib, *id, *lod));
                }
            }
        }
        update
    }

    /// [`Streamer::update_with`] asking for up to four new chunks per call.
    pub fn update(&mut self, eye: (f64, f64)) -> Update {
        self.update_with(eye, 4)
    }

    /// Builds everything wanted around `eye` right now (a still frame, a test). With workers it waits for them.
    pub fn fill(&mut self, eye: (f64, f64)) -> Update {
        self.fill_many(&[eye])
    }

    /// [`Streamer::fill`] for several viewers at once.
    pub fn fill_many(&mut self, eyes: &[(f64, f64)]) -> Update {
        let mut all = Update::default();
        for _ in 0..100_000 {
            let u = self.update_many(eyes, usize::MAX);
            all.added.extend(u.added);
            all.removed.extend(u.removed);
            if self.pending.is_empty() && self.wanted_many(eyes).iter().all(|(id, lod, _)| self.loaded.get(id) == Some(lod)) {
                return all;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config { seed: 7, ..Config::default() }
    }

    fn small() -> View {
        View { distance: 150.0 }
    }

    #[test]
    fn fill_builds_the_disc_of_chunks_with_detail_by_distance() {
        let mut s = Streamer::synchronous(cfg(), small());
        let eye = (24.0, 24.0);
        let u = s.fill(eye);
        let want = s.wanted(eye);
        assert_eq!(u.added.len(), want.len());
        assert!(u.added.len() > 20 && u.added.len() < 60, "{} chunks", u.added.len());
        let here = u.added.iter().find(|c| c.id == ChunkId { x: 0, z: 0 }).expect("the chunk underfoot");
        assert_eq!(here.lod, Lod::Near);
        assert!(u.added.iter().any(|c| c.lod == Lod::Mid), "the middle ring is cheaper");
        for c in &u.added {
            let (cx, cz) = c.id.centre();
            assert!(((cx - eye.0).powi(2) + (cz - eye.1).powi(2)).sqrt() <= 150.0);
        }
    }

    #[test]
    fn several_viewers_keep_the_ground_of_all_of_them() {
        let mut s = Streamer::synchronous(cfg(), small());
        let (a, b) = ((24.0, 24.0), (2000.0, -900.0));
        let both = s.fill_many(&[a, b]);
        let mut alone = Streamer::synchronous(cfg(), small());
        let one = alone.fill(a).added.len() + Streamer::synchronous(cfg(), small()).fill(b).added.len();
        assert_eq!(both.added.len(), one, "the two discs, no more and no fewer");
        assert!(both.added.iter().any(|c| c.id == ChunkId { x: 0, z: 0 }) && both.added.iter().any(|c| c.id == ChunkId::at(2000.0, -900.0)));
        // Standing still, nothing is rebuilt however many are watching.
        let built = s.stats().built;
        s.fill_many(&[a, b]);
        s.fill_many(&[b, a]);
        assert_eq!(s.stats().built, built);
        // One walks away: only their ground changes hands.
        let u = s.fill_many(&[a, (2000.0 + 480.0, -900.0)]);
        assert!(u.removed.iter().all(|id| id.x > 30), "the first viewer's ground stays: {:?}", u.removed);
    }

    #[test]
    fn the_nearest_chunk_comes_first() {
        let mut s = Streamer::synchronous(cfg(), small());
        let u = s.update_with((24.0, 24.0), 1);
        assert_eq!(u.added.len(), 1);
        assert_eq!(u.added[0].id, ChunkId { x: 0, z: 0 });
    }

    #[test]
    fn walking_east_drops_the_west_and_adds_the_east_and_nothing_else_is_rebuilt() {
        let mut s = Streamer::synchronous(cfg(), small());
        s.fill((24.0, 24.0));
        let before = s.stats().built;
        let u = s.fill((24.0 + 48.0, 24.0));
        assert!(!u.removed.is_empty() && !u.added.is_empty());
        assert!(u.removed.iter().all(|id| id.x <= 1), "dropped something that is still near: {:?}", u.removed);
        // Chunks that stayed in the same band are untouched: far fewer rebuilds than a full reload.
        let all = s.wanted((72.0, 24.0)).len() as u64;
        assert!(s.stats().built - before < all * 3 / 4, "rebuilt {} of {all}", s.stats().built - before);
        assert_eq!(s.stats().loaded as u64, all);
    }

    #[test]
    fn standing_on_a_detail_boundary_does_not_rebuild_back_and_forth() {
        let mut s = Streamer::synchronous(cfg(), small());
        s.fill((24.0, 24.0));
        let built = s.stats().built;
        // Wobble one metre either way: nothing should change.
        for k in 0..20 {
            s.fill((24.0 + if k % 2 == 0 { 0.7 } else { -0.7 }, 24.0));
        }
        assert!(s.stats().built - built <= 2, "{} rebuilds from wobbling", s.stats().built - built);
    }

    #[test]
    fn worker_threads_deliver_the_same_chunks_as_building_in_place() {
        let mut threaded = Streamer::new(cfg(), small(), 3);
        let mut inline = Streamer::synchronous(cfg(), small());
        let eye = (-300.0, 120.0);
        let (a, b) = (threaded.fill(eye), inline.fill(eye));
        assert_eq!(a.added.len(), b.added.len());
        for c in &b.added {
            let t = a.added.iter().find(|t| t.id == c.id).expect("the same chunk");
            assert_eq!(t.lod, c.lod);
            assert_eq!(t.solid.pos, c.solid.pos);
            assert_eq!(t.ground.pos, c.ground.pos);
        }
        assert_eq!(threaded.stats().pending, 0);
    }

    #[test]
    fn a_few_chunks_per_update_keeps_frames_short() {
        let mut s = Streamer::synchronous(cfg(), View { distance: 270.0 });
        let u = s.update((0.0, 0.0));
        assert!(u.added.len() <= 4);
        assert!(s.stats().loaded <= 4);
    }

    #[test]
    fn the_whole_view_is_a_sensible_cost() {
        let mut s = Streamer::synchronous(cfg(), View::default());
        let u = s.fill((1000.0, -2000.0));
        let (tris, verts): (usize, usize) = u.added.iter().map(|c| (c.tris(), c.verts())).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        eprintln!("{} chunks, {tris} triangles, {verts} vertices ({} MB)", u.added.len(), verts * 40 / 1_000_000);
        for lod in [Lod::Near, Lod::Mid, Lod::Far] {
            let cs: Vec<_> = u.added.iter().filter(|c| c.lod == lod).collect();
            let (g, so, fl): (usize, usize, usize) = cs.iter().fold((0, 0, 0), |a, c| (a.0 + c.ground.tris(), a.1 + c.solid.tris(), a.2 + c.flora.tris()));
            eprintln!("  {lod:?}: {} chunks, ground {g} solid {so} flora {fl}", cs.len());
        }
        assert!(tris < 1_900_000, "{tris} triangles in view");
        assert!(verts * 40 < 130_000_000, "{} MB of vertices", verts * 40 / 1_000_000);
    }
}
