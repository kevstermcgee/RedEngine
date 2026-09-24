//! Reachability: "where can the player actually walk?"
//!
//! A flood-fill over `(x, z, foot height)` states that asks the *engine's own* questions at each
//! step — [`collider_blocks_at`] for walls/furniture at the current foot height and
//! [`ground_height_at`] for what's underfoot (floors, stair ramps, box tops, drops). Because the
//! rules are the game's, the result is ground truth for questions like "can I get upstairs?" or
//! "is this bedroom sealed off?", not a geometric guess.
//!
//! What it models: walking (no jumping/crouching), stepping up <= `GROUND_SNAP_EPS` (stairs),
//! falling off edges. What it doesn't: jumping onto low furniture (a deliberately conservative
//! choice — a map that relies on jumping to reach a floor is broken for most players anyway).

use super::world::{MapWorld, Zone};
use crate::player::PLAYER_RADIUS;
use crate::viewer::{collider_blocks_at, ground_height_at, Collider2D};
use glam::Vec2;
use std::collections::{HashMap, HashSet, VecDeque};

/// Height quantization for de-duplicating states (0.05m buckets).
const Y_BUCKET: f32 = 20.0;
/// A fall bigger than this counts as a drop hazard.
pub const DROP_THRESHOLD: f32 = 0.6;
const MAX_STATES: usize = 6_000_000;

/// A place where the player can step off an edge and fall (`from_y` to `to_y`).
#[derive(Debug, Clone, Copy)]
pub struct DropEvent {
    pub pos: Vec2,
    pub from_y: f32,
    pub to_y: f32,
}

/// Reachability result: per-cell standable heights, drops, perimeter leaks and derived queries.
pub struct Reach {
    pub cell: f32,
    pub min: Vec2,
    pub nx: usize,
    pub nz: usize,
    /// Per grid cell: the distinct foot heights the player can stand at there.
    pub levels: Vec<Vec<f32>>,
    pub drops: Vec<DropEvent>,
    /// Reachable cells that touch the analysis border — the player can walk off the map here.
    pub leaks: Vec<Vec2>,
    pub start: Vec2,
    pub start_y: f32,
    /// True if the requested start was inside a collider and had to be nudged (or failed).
    pub start_adjusted: bool,
    pub start_ok: bool,
    pub truncated: bool,
}

/// Uniform-grid index over colliders so a position test only looks at nearby ones.
struct ColliderGrid<'a> {
    colliders: &'a [Collider2D],
    min: Vec2,
    bin: f32,
    nx: usize,
    nz: usize,
    bins: Vec<Vec<u32>>,
}

impl<'a> ColliderGrid<'a> {
    fn new(colliders: &'a [Collider2D], min: Vec2, max: Vec2) -> Self {
        let bin = 1.0;
        let nx = (((max.x - min.x) / bin).ceil() as usize + 1).max(1);
        let nz = (((max.y - min.y) / bin).ceil() as usize + 1).max(1);
        let mut bins = vec![Vec::new(); nx * nz];
        for (i, c) in colliders.iter().enumerate() {
            let lo = c.min - Vec2::splat(PLAYER_RADIUS);
            let hi = c.max + Vec2::splat(PLAYER_RADIUS);
            let (x0, x1) = (((lo.x - min.x) / bin).floor().max(0.0) as usize, ((hi.x - min.x) / bin).floor().max(0.0) as usize);
            let (z0, z1) = (((lo.y - min.y) / bin).floor().max(0.0) as usize, ((hi.y - min.y) / bin).floor().max(0.0) as usize);
            for z in z0..=z1.min(nz - 1) {
                for x in x0..=x1.min(nx - 1) {
                    bins[z * nx + x].push(i as u32);
                }
            }
        }
        ColliderGrid { colliders, min, bin, nx, nz, bins }
    }

    /// Would a player circle centered at `p` with feet at `foot_y` overlap a blocking collider?
    fn blocked(&self, p: Vec2, foot_y: f32) -> bool {
        let x = ((p.x - self.min.x) / self.bin).floor();
        let z = ((p.y - self.min.y) / self.bin).floor();
        if x < 0.0 || z < 0.0 || x as usize >= self.nx || z as usize >= self.nz {
            return false;
        }
        let r2 = PLAYER_RADIUS * PLAYER_RADIUS - 1e-5;
        for &i in &self.bins[z as usize * self.nx + x as usize] {
            let c = &self.colliders[i as usize];
            if !collider_blocks_at(c, foot_y) {
                continue;
            }
            let d = p - p.clamp(c.min, c.max);
            if d.length_squared() < r2 {
                return true;
            }
        }
        false
    }
}

/// Inputs to reachability: grid resolution, start point, margin.
pub struct ReachParams {
    /// Grid resolution in meters. 0.1 is fast; 0.05 resolves tight gaps more faithfully.
    pub cell: f32,
    /// Where to start (XZ). Default: the scene camera / player spawn.
    pub start: Option<Vec2>,
    /// Extra room around the map's solid bounds to flood into (the perimeter-leak test).
    pub margin: f32,
}

impl Default for ReachParams {
    fn default() -> Self {
        ReachParams { cell: 0.1, start: None, margin: 3.0 }
    }
}

/// Flood-fills the walkable grid from the start using the game's real per-tick movement functions.
pub fn compute(world: &MapWorld, params: &ReachParams) -> Reach {
    let (smin, smax) = world.solid_bounds();
    let mut bmin = smin - Vec2::splat(params.margin);
    let mut bmax = smax + Vec2::splat(params.margin);
    let start = params.start.unwrap_or(world.spawn);
    bmin = bmin.min(start - Vec2::splat(1.0));
    bmax = bmax.max(start + Vec2::splat(1.0));
    let cell = params.cell.max(0.02);
    let nx = ((bmax.x - bmin.x) / cell).ceil() as usize + 1;
    let nz = ((bmax.y - bmin.y) / cell).ceil() as usize + 1;
    let grid = ColliderGrid::new(&world.colliders, bmin, bmax);
    let center = |ix: usize, iz: usize| Vec2::new(bmin.x + ix as f32 * cell, bmin.y + iz as f32 * cell);

    let mut reach = Reach {
        cell,
        min: bmin,
        nx,
        nz,
        levels: vec![Vec::new(); nx * nz],
        drops: Vec::new(),
        leaks: Vec::new(),
        start,
        start_y: 0.0,
        start_adjusted: false,
        start_ok: true,
        truncated: false,
    };

    // Starting state: the spawn, or the nearest unblocked cell within a meter of it.
    let sx = ((start.x - bmin.x) / cell).round() as isize;
    let sz = ((start.y - bmin.y) / cell).round() as isize;
    let y0 = ground_height_at(&world.ground, start, 0.0);
    let mut seed: Option<(usize, usize, f32)> = None;
    let search = (1.0 / cell).ceil() as isize;
    'find: for ring in 0..=search {
        for dz in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dz.abs()) != ring {
                    continue;
                }
                let (ix, iz) = (sx + dx, sz + dz);
                if ix < 0 || iz < 0 || ix as usize >= nx || iz as usize >= nz {
                    continue;
                }
                let p = center(ix as usize, iz as usize);
                let y = ground_height_at(&world.ground, p, y0);
                if !grid.blocked(p, y) {
                    seed = Some((ix as usize, iz as usize, y));
                    reach.start_adjusted = ring > 0;
                    break 'find;
                }
            }
        }
    }
    let Some((ix0, iz0, y_seed)) = seed else {
        reach.start_ok = false;
        return reach;
    };
    reach.start_y = y_seed;

    let key = |ix: usize, iz: usize, y: f32| (ix as u32, iz as u32, (y * Y_BUCKET).round() as i32);
    let mut seen: HashSet<(u32, u32, i32)> = HashSet::new();
    let mut queue: VecDeque<(usize, usize, f32)> = VecDeque::new();
    seen.insert(key(ix0, iz0, y_seed));
    queue.push_back((ix0, iz0, y_seed));
    reach.levels[iz0 * nx + ix0].push(y_seed);

    const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
    let mut drop_seen: HashSet<(u32, u32)> = HashSet::new();
    while let Some((ix, iz, y)) = queue.pop_front() {
        if seen.len() > MAX_STATES {
            reach.truncated = true;
            break;
        }
        for (dx, dz) in DIRS {
            let (jx, jz) = (ix as isize + dx, iz as isize + dz);
            if jx < 0 || jz < 0 || jx as usize >= nx || jz as usize >= nz {
                continue;
            }
            let (jx, jz) = (jx as usize, jz as usize);
            let p = center(jx, jz);
            if grid.blocked(p, y) {
                continue;
            }
            let ny = ground_height_at(&world.ground, p, y);
            let k = key(jx, jz, ny);
            if !seen.insert(k) {
                continue;
            }
            if y - ny > DROP_THRESHOLD && drop_seen.insert((jx as u32 / 10, jz as u32 / 10)) {
                reach.drops.push(DropEvent { pos: p, from_y: y, to_y: ny });
            }
            reach.levels[jz * nx + jx].push(ny);
            if jx == 0 || jz == 0 || jx + 1 == nx || jz + 1 == nz {
                reach.leaks.push(p);
            }
            queue.push_back((jx, jz, ny));
        }
    }
    reach
}

/// Free floor the player can reach *locally* from `center` (within `radius`), standing at foot
/// height `y` — used to check that a doorway is really usable: both sides must offer somewhere to
/// stand that connects to the opening without leaving the neighbourhood (a chair or shelf parked
/// right in front of a door defeats it even if the room is reachable some other way).
pub fn local_reach(world: &MapWorld, center: Vec2, y: f32, radius: f32, cell: f32) -> Vec<Vec2> {
    let grid = ColliderGrid::new(&world.colliders, center - Vec2::splat(radius + 1.0), center + Vec2::splat(radius + 1.0));
    let n = (radius / cell).ceil() as i32;
    let at = |ix: i32, iz: i32| center + Vec2::new(ix as f32 * cell, iz as f32 * cell);
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    if grid.blocked(center, y) {
        return Vec::new();
    }
    seen.insert((0, 0));
    queue.push_back((0, 0));
    let mut out = vec![center];
    while let Some((ix, iz)) = queue.pop_front() {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let (jx, jz) = (ix + dx, iz + dz);
            if jx.abs() > n || jz.abs() > n || (jx * jx + jz * jz) as f32 * cell * cell > radius * radius {
                continue;
            }
            if !seen.insert((jx, jz)) {
                continue;
            }
            let p = at(jx, jz);
            if grid.blocked(p, y) {
                continue;
            }
            out.push(p);
            queue.push_back((jx, jz));
        }
    }
    out
}

impl Reach {
    fn idx(&self, p: Vec2) -> Option<usize> {
        let ix = ((p.x - self.min.x) / self.cell).round();
        let iz = ((p.y - self.min.y) / self.cell).round();
        if ix < 0.0 || iz < 0.0 || ix as usize >= self.nx || iz as usize >= self.nz {
            return None;
        }
        Some(iz as usize * self.nx + ix as usize)
    }

    /// Every foot height the player can stand at over `p` (empty = unreachable there).
    pub fn levels_at(&self, p: Vec2) -> Vec<f32> {
        self.idx(p).map(|i| self.levels[i].clone()).unwrap_or_default()
    }

    /// World XZ of a grid cell's center.
    pub fn cell_center(&self, i: usize) -> Vec2 {
        Vec2::new(self.min.x + (i % self.nx) as f32 * self.cell, self.min.y + (i / self.nx) as f32 * self.cell)
    }

    /// True if the player can stand within `tol` of height `y` at `p`.
    pub fn reachable(&self, p: Vec2, y: f32, tol: f32) -> bool {
        self.idx(p).is_some_and(|i| self.levels[i].iter().any(|l| (l - y).abs() <= tol))
    }

    /// Reachable and blocked-free area (m^2) in `[min, max]` at floor height `y`, plus the total
    /// standable area there (cells the player circle *could* occupy if nothing sealed them off).
    pub fn area_in(&self, world: &MapWorld, min: Vec2, max: Vec2, y: f32, tol: f32) -> (f32, f32) {
        let grid = ColliderGrid::new(&world.colliders, self.min, self.min + Vec2::new(self.nx as f32, self.nz as f32) * self.cell);
        let (mut reach_n, mut free_n) = (0usize, 0usize);
        let x0 = (((min.x - self.min.x) / self.cell).ceil().max(0.0)) as usize;
        let x1 = (((max.x - self.min.x) / self.cell).floor().max(0.0) as usize).min(self.nx - 1);
        let z0 = (((min.y - self.min.y) / self.cell).ceil().max(0.0)) as usize;
        let z1 = (((max.y - self.min.y) / self.cell).floor().max(0.0) as usize).min(self.nz - 1);
        for iz in z0..=z1 {
            for ix in x0..=x1 {
                let i = iz * self.nx + ix;
                let p = self.cell_center(i);
                if self.levels[i].iter().any(|l| (l - y).abs() <= tol) {
                    reach_n += 1;
                    free_n += 1;
                } else if !grid.blocked(p, y) {
                    free_n += 1;
                }
            }
        }
        let a = self.cell * self.cell;
        (reach_n as f32 * a, free_n as f32 * a)
    }

    /// Distinct floors the player can stand on: `(height, reachable area m^2)`, ignoring the
    /// thin slivers a stair ramp contributes at every intermediate height.
    pub fn floors(&self) -> Vec<(f32, f32)> {
        let mut buckets: HashMap<i32, usize> = HashMap::new();
        for lv in &self.levels {
            for y in lv {
                *buckets.entry((y * 10.0).round() as i32).or_default() += 1;
            }
        }
        let a = self.cell * self.cell;
        let mut out: Vec<(f32, f32)> =
            buckets.into_iter().map(|(k, n)| (k as f32 / 10.0, n as f32 * a)).filter(|(_, area)| *area >= 2.0).collect();
        out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        out
    }

    /// Total reachable floor area in square metres.
    pub fn total_area(&self) -> f32 {
        self.levels.iter().map(|l| l.len()).sum::<usize>() as f32 * self.cell * self.cell
    }

    /// Groups drop events that are near each other into single hazards.
    pub fn drop_clusters(&self) -> Vec<(Vec2, f32, f32, usize)> {
        let mut clusters: Vec<(Vec2, f32, f32, usize)> = Vec::new();
        for d in &self.drops {
            if let Some(c) = clusters.iter_mut().find(|c| (c.0 - d.pos).length() < 2.5 && (c.1 - d.from_y).abs() < 0.5) {
                let n = c.3 as f32;
                c.0 = (c.0 * n + d.pos) / (n + 1.0);
                c.3 += 1;
            } else {
                clusters.push((d.pos, d.from_y, d.to_y, 1));
            }
        }
        clusters
    }

    /// Which zone (if any) contains `p` at foot height `y`. Points a hair outside every zone (the
    /// cells inside a doorway, where the wall's thickness separates two rooms' rects) count as
    /// belonging to the nearest zone within `WALL_TOLERANCE`, so doorways connect zones.
    pub fn zone_of<'a>(zones: &'a [Zone], p: Vec2, y: f32) -> Option<&'a Zone> {
        const WALL_TOLERANCE: f32 = 0.2;
        let on_floor = |z: &&Zone| (z.y - y).abs() <= 0.6;
        zones
            .iter()
            .filter(on_floor)
            .find(|z| z.contains(p))
            .or_else(|| {
                zones.iter().filter(on_floor).find(|z| {
                    p.x >= z.min.x - WALL_TOLERANCE && p.x <= z.max.x + WALL_TOLERANCE && p.y >= z.min.y - WALL_TOLERANCE && p.y <= z.max.y + WALL_TOLERANCE
                })
            })
    }

    /// Doorway/opening connections between zones: `(zone A, zone B, midpoint, width)`.
    pub fn passages(&self, zones: &[Zone]) -> Vec<(String, String, Vec2, f32)> {
        let mut acc: HashMap<(String, String), (Vec2, usize)> = HashMap::new();
        let nx = self.nx;
        for i in 0..self.levels.len() {
            let (ix, iz) = (i % nx, i / nx);
            let p = self.cell_center(i);
            for &y in &self.levels[i] {
                let Some(za) = Self::zone_of(zones, p, y) else { continue };
                for (dx, dz) in [(1usize, 0usize), (0, 1)] {
                    if ix + dx >= nx || iz + dz >= self.nz {
                        continue;
                    }
                    let j = (iz + dz) * nx + ix + dx;
                    let q = self.cell_center(j);
                    for &y2 in &self.levels[j] {
                        if (y2 - y).abs() > 0.15 {
                            continue;
                        }
                        let Some(zb) = Self::zone_of(zones, q, y2) else { continue };
                        if za.id == zb.id {
                            continue;
                        }
                        let (a, b) = if za.id < zb.id { (za.id.clone(), zb.id.clone()) } else { (zb.id.clone(), za.id.clone()) };
                        let e = acc.entry((a, b)).or_insert((Vec2::ZERO, 0));
                        e.0 += (p + q) * 0.5;
                        e.1 += 1;
                    }
                }
            }
        }
        let mut out: Vec<(String, String, Vec2, f32)> =
            // The crossing cells are player *centers*, which stay a radius clear of the door frame on
            // each side, so the physical opening is that free span plus the player's diameter.
            acc.into_iter().map(|((a, b), (sum, n))| (a, b, sum / n as f32, n as f32 * self.cell + 2.0 * PLAYER_RADIUS)).collect();
        out.sort_by(|a, b| (a.0.as_str(), a.1.as_str()).cmp(&(b.0.as_str(), b.1.as_str())));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn world(json: &str) -> MapWorld {
        MapWorld::from_text(json, Path::new("t.json")).unwrap()
    }

    #[test]
    fn open_yard_floods_to_the_border() {
        let w = world(r##"{"camera":{"position":[0,1.7,0]},"objects":[
            {"id":"b","type":"box","size":[1,2,1],"position":[5,1,5]}]}"##);
        let r = compute(&w, &ReachParams { cell: 0.25, ..Default::default() });
        assert!(r.start_ok);
        assert!(!r.leaks.is_empty(), "an unfenced yard must report a perimeter leak");
    }

    #[test]
    fn sealed_room_reaches_only_its_interior() {
        // Four walls around the spawn: the flood must stay inside and never reach the border.
        let w = world(r##"{"camera":{"position":[0,1.7,0]},"objects":[
            {"id":"n","type":"wall","from":[-3,-3],"to":[3,-3]},
            {"id":"s","type":"wall","from":[-3,3],"to":[3,3]},
            {"id":"e","type":"wall","from":[3,-3],"to":[3,3]},
            {"id":"w","type":"wall","from":[-3,-3],"to":[-3,3]}]}"##);
        let r = compute(&w, &ReachParams { cell: 0.1, ..Default::default() });
        assert!(r.leaks.is_empty(), "sealed room must not leak");
        let area = r.total_area();
        assert!(area > 20.0 && area < 36.0, "interior is ~5.4x5.4 minus the player radius margin, got {area}");
    }

    #[test]
    fn narrow_doorway_blocks_and_wide_one_passes() {
        for (width, expect_leak) in [(0.6, false), (1.0, true)] {
            let json = format!(
                r##"{{"camera":{{"position":[0,1.7,0]}},"objects":[
                {{"id":"n","type":"wall","from":[-3,-3],"to":[3,-3]}},
                {{"id":"s","type":"wall","from":[-3,3],"to":[3,3],"openings":[{{"at":3.0,"width":{width}}}]}},
                {{"id":"e","type":"wall","from":[3,-3],"to":[3,3]}},
                {{"id":"w","type":"wall","from":[-3,-3],"to":[-3,3]}}]}}"##
            );
            let r = compute(&world(&json), &ReachParams { cell: 0.05, ..Default::default() });
            assert_eq!(!r.leaks.is_empty(), expect_leak, "door width {width}");
        }
    }

    #[test]
    fn stairs_to_a_landing_reach_the_upper_floor() {
        // A stair run climbing +Z from z=1 to z=5, landing on a slab that starts at z=5.
        let w = world(r##"{"camera":{"position":[0,1.7,-1]},"objects":[
            {"id":"st","type":"stairs","position":[0,0,3],"width":1.2,"run":4.0,"rise":2.8,"steps":14},
            {"id":"deck","type":"box","size":[6,0.2,4],"position":[0,2.7,7]}]}"##);
        let r = compute(&w, &ReachParams { cell: 0.1, ..Default::default() });
        assert!(r.reachable(Vec2::new(0.0, 6.5), 2.8, 0.1), "the deck above the stairs must be reachable");
        assert!(r.reachable(Vec2::new(0.0, 3.0), 1.4, 0.3), "and so must the middle of the ramp");
    }

    #[test]
    fn stairs_do_not_let_you_walk_into_the_stair_volume_from_the_side() {
        let w = world(r##"{"camera":{"position":[0,1.7,-1]},"objects":[
            {"id":"st","type":"stairs","position":[0,0,3],"width":1.2,"run":4.0,"rise":2.8,"steps":14}]}"##);
        let r = compute(&w, &ReachParams { cell: 0.1, ..Default::default() });
        assert!(!r.reachable(Vec2::new(0.0, 4.0), 0.0, 0.1), "the stair volume must not be enterable at floor level");
    }
}
