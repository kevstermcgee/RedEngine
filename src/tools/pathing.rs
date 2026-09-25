//! Why a walk stops, and how to author a route without guessing coordinates.
//!
//! Two jobs, both built on the exact collision/ground functions the game runs (never a second model):
//!
//! * [`diagnose`]: given where a [`super::walk`] leg gave up, name the objects touching the player (with their ids and
//!   kinds, and whether they are in the way of the heading) and measure the passage the player was trying to squeeze
//!   through against the body width. `reach` answers "is there a route" on a grid; a straight-line `walk` leg can still
//!   fail on a corner, and this says which corner and which object.
//! * [`plan_route`]: `walk --auto FROM TO`. A* over the reach grid with a slightly fatter body (so the route keeps clear of
//!   corners), string-pulled with real-physics segment checks, then validated end to end with [`super::walk::walk_from`].
//!   The waypoints it prints can be pasted into a `--path` or a scene's `checks.walk`.

use super::reach::ColliderGrid;
use super::walk::{walk_from, WalkStep};
use super::world::MapWorld;
use crate::collide::{collider_blocks_at, ground_height_at, Collider2D};
use crate::player::PLAYER_RADIUS;
use glam::Vec2;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// How far from the player's body a collider still counts as "touching" (the walk gives up a little short of contact).
const TOUCH: f32 = 0.08;
/// Farthest a passage-width probe looks to each side, m.
const PROBE: f32 = 2.5;

/// One thing the player is pressed against when a walk gives up.
#[derive(Debug, Clone)]
pub struct Blocker {
    /// Leaf id (`wall_north.seg2`) of the object that owns the collider, or `?` if none could be matched.
    pub id: String,
    /// Id of the top-level scene object (what `set`/`move`/`rm` take).
    pub top_id: String,
    /// `box`, `wall`, `prop:crate`, `stairs`, ... (see `ItemKind::label`).
    pub kind: String,
    /// Space between the player's body and the collider, m (0 = touching).
    pub gap: f32,
    /// Whether it lies on the side the player was heading (it, not something behind, stopped the walk).
    pub ahead: bool,
    /// World XZ bounds of the collider `(min, max)`.
    pub bounds: (Vec2, Vec2),
}

/// The passage the player was trying to use, measured perpendicular to the heading.
#[derive(Debug, Clone, Copy)]
pub struct Clearance {
    /// Free width between the two surfaces either side of the path, m.
    pub width: f32,
    /// Width the body needs (2 * radius), m.
    pub needed: f32,
}

/// Everything known about where and why a walk leg stopped.
#[derive(Debug, Clone)]
pub struct Diagnosis {
    pub pos: Vec2,
    pub foot_y: f32,
    pub target: Vec2,
    /// Touching colliders, the ones ahead first.
    pub blockers: Vec<Blocker>,
    pub clearance: Option<Clearance>,
    /// Whether a flood fill from `pos` can reach `target` at all (`false` = the destination is sealed off).
    pub reachable: bool,
    /// One-paragraph plain-language conclusion with the next command to try.
    pub summary: String,
}

impl Diagnosis {
    /// Multi-line text for the terminal / a `verify` failure.
    pub fn render(&self) -> String {
        let mut s = format!("stopped at ({:.2}, {:.2}) y={:.2} heading to ({:.2}, {:.2})\n", self.pos.x, self.pos.y, self.foot_y, self.target.x, self.target.y);
        if self.blockers.is_empty() {
            s.push_str("  nothing solid is touching the player (the leg stalled without contact)\n");
        }
        for b in self.blockers.iter().take(5) {
            s.push_str(&format!(
                "  {} '{}' [{}]{} gap {:.2} m, occupies x {:.2}..{:.2} z {:.2}..{:.2}\n",
                if b.ahead { "BLOCKED BY" } else { "beside" },
                b.id,
                b.kind,
                if b.top_id != b.id && b.top_id != "?" { format!(" (part of '{}')", b.top_id) } else { String::new() },
                b.gap.max(0.0),
                b.bounds.0.x,
                b.bounds.1.x,
                b.bounds.0.y,
                b.bounds.1.y
            ));
        }
        if let Some(c) = self.clearance {
            s.push_str(&format!(
                "  passage here is {:.2} m wide; the body needs {:.2} m{}\n",
                c.width,
                c.needed,
                if c.width < c.needed + 0.02 { " -> TOO NARROW" } else { "" }
            ));
        }
        s.push_str(&format!("  {}\n", self.summary));
        s
    }

    /// A compact one-liner for check lists (`verify`).
    pub fn one_line(&self) -> String {
        let ahead: Vec<String> = self.blockers.iter().filter(|b| b.ahead).take(2).map(|b| format!("'{}'", b.id)).collect();
        let mut s = if ahead.is_empty() { "no object in the way".to_string() } else { format!("blocked by {}", ahead.join(" and ")) };
        if let Some(c) = self.clearance.filter(|c| c.width < c.needed + 0.02) {
            s.push_str(&format!("; passage {:.2} m < body {:.2} m", c.width, c.needed));
        }
        s.push_str(if self.reachable { "; target IS reachable (try `walk --auto`)" } else { "; target is NOT reachable from here" });
        s
    }

    /// JSON form for `--json`.
    pub fn to_json(&self) -> serde_json::Value {
        let r = |v: f32| (v * 100.0).round() / 100.0;
        serde_json::json!({
            "pos": [r(self.pos.x), r(self.pos.y)], "foot_y": r(self.foot_y), "target": [r(self.target.x), r(self.target.y)],
            "blockers": self.blockers.iter().map(|b| serde_json::json!({
                "id": b.id, "top_id": b.top_id, "kind": b.kind, "gap": r(b.gap.max(0.0)), "ahead": b.ahead,
                "bounds": [r(b.bounds.0.x), r(b.bounds.0.y), r(b.bounds.1.x), r(b.bounds.1.y)],
            })).collect::<Vec<_>>(),
            "clearance": self.clearance.map(|c| serde_json::json!({"width": r(c.width), "needed": r(c.needed), "too_narrow": c.width < c.needed + 0.02})),
            "reachable": self.reachable,
            "summary": self.summary,
        })
    }
}

/// Which scene objects own a collider: solid items whose volumes cover it (matched by overlap, so a prop's single footprint
/// collider finds the prop and a wall segment finds its `.segN`).
fn owners(world: &MapWorld, c: &Collider2D) -> Vec<(String, String, String)> {
    let area = |min: Vec2, max: Vec2| ((max.x - min.x) * (max.y - min.y)).max(1e-6);
    let ca = area(c.min, c.max);
    let mut out: Vec<(f32, (String, String, String))> = Vec::new();
    for it in world.items.iter().filter(|i| i.is_solid()) {
        let mut best = 0.0f32;
        for (vmin, vmax) in &it.volumes {
            if vmax.y < c.min_y || vmin.y > c.max_y {
                continue;
            }
            let lo = Vec2::new(vmin.x.max(c.min.x), vmin.z.max(c.min.y));
            let hi = Vec2::new(vmax.x.min(c.max.x), vmax.z.min(c.max.y));
            if hi.x <= lo.x || hi.y <= lo.y {
                continue;
            }
            let va = area(Vec2::new(vmin.x, vmin.z), Vec2::new(vmax.x, vmax.z));
            best = best.max(area(lo, hi) / ca.min(va));
        }
        if best >= 0.5 {
            out.push((best, (it.id.clone(), it.top_id.clone(), it.kind.label())));
        }
    }
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter().map(|(_, o)| o).collect()
}

/// Distance along `dir` from `origin` to the first blocking collider (`None` if clear for [`PROBE`] metres; `Some(0)` if inside).
fn ray_to_collider(colliders: &[Collider2D], foot_y: f32, origin: Vec2, dir: Vec2) -> Option<f32> {
    let mut best: Option<f32> = None;
    for c in colliders.iter().filter(|c| collider_blocks_at(c, foot_y)) {
        let (mut t0, mut t1) = (0.0f32, PROBE);
        for (o, d, lo, hi) in [(origin.x, dir.x, c.min.x, c.max.x), (origin.y, dir.y, c.min.y, c.max.y)] {
            if d.abs() < 1e-6 {
                if o < lo || o > hi {
                    t0 = f32::INFINITY;
                }
            } else {
                let (a, b) = ((lo - o) / d, (hi - o) / d);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
        }
        if t0 <= t1 {
            best = Some(best.map_or(t0, |b| b.min(t0)));
        }
    }
    best
}

/// Free width across the route at `at`, perpendicular to `heading`, if there is a surface on both sides within [`PROBE`].
fn passage_width(colliders: &[Collider2D], foot_y: f32, at: Vec2, heading: Vec2) -> Option<f32> {
    let perp = Vec2::new(-heading.y, heading.x);
    let l = ray_to_collider(colliders, foot_y, at, perp)?;
    let r = ray_to_collider(colliders, foot_y, at, -perp)?;
    // A probe point that is itself inside a solid (the obstacle we are pressed against) says nothing about a passage.
    if l < 1e-4 || r < 1e-4 {
        return None;
    }
    // Centres sit a body radius off each surface at best, so the width between surfaces is the sum of the distances.
    Some(l + r)
}

/// Explains where and why a walk leg stopped at `pos` (feet at `foot_y`) heading for `target`.
pub fn diagnose(world: &MapWorld, pos: Vec2, foot_y: f32, target: Vec2) -> Diagnosis {
    let heading = (target - pos).normalize_or_zero();
    let mut blockers: Vec<Blocker> = Vec::new();
    for c in world.colliders.iter().filter(|c| collider_blocks_at(c, foot_y)) {
        let closest = pos.clamp(c.min, c.max);
        let dist = (pos - closest).length();
        if dist > PLAYER_RADIUS + TOUCH {
            continue;
        }
        let toward = (closest - pos).normalize_or_zero();
        let (id, top_id, kind) = owners(world, c).into_iter().next().unwrap_or(("?".into(), "?".into(), "collider".into()));
        blockers.push(Blocker { id, top_id, kind, gap: dist - PLAYER_RADIUS, ahead: toward.dot(heading) > 0.2 || dist < 1e-3, bounds: (c.min, c.max) });
    }
    blockers.sort_by(|a, b| b.ahead.cmp(&a.ahead).then(a.gap.partial_cmp(&b.gap).unwrap_or(std::cmp::Ordering::Equal)));

    // Passage width: measure where the player is and a little further along the heading (the pinch is often just ahead).
    let needed = 2.0 * PLAYER_RADIUS;
    let probes = [pos, pos + heading * 0.35, pos + heading * 0.7];
    let clearance = probes
        .iter()
        .filter_map(|&p| passage_width(&world.colliders, foot_y, p, heading))
        .fold(None, |m: Option<f32>, w| Some(m.map_or(w, |m| m.min(w))))
        .map(|width| Clearance { width, needed });

    let rr = super::reach::compute(world, &super::reach::ReachParams { start: Some(pos), ..Default::default() });
    let reachable = !rr.levels_at(target).is_empty();

    let mut summary = String::new();
    if let Some(b) = blockers.iter().find(|b| b.ahead) {
        summary.push_str(&format!("'{}' ({}) is in the way. ", b.id, b.kind));
        if b.kind.starts_with("prop:") || b.kind == "box" {
            summary.push_str(&format!("Move it (`red_engine2 move <scene> {} --by dx,dy,dz`) or route around it. ", b.top_id));
        }
    }
    if let Some(c) = clearance.filter(|c| c.width < c.needed + 0.02) {
        summary.push_str(&format!("The gap is {:.2} m but the body needs {:.2} m: widen it by {:.2} m. ", c.width, c.needed, c.needed - c.width + 0.05));
    }
    if reachable {
        summary.push_str("`reach` agrees the target is reachable from here, so the straight leg is what is obstructed: `red_engine2 walk <scene> --auto --from X,Z --to X,Z` plans a route around it.");
    } else {
        summary.push_str("Even a flood fill cannot reach the target from here: a wall/door/stairs/slab is sealing it (see `red_engine2 lint`, `plan --bounds`).");
    }
    Diagnosis { pos, foot_y, target, blockers, clearance, reachable, summary }
}

/// Knobs for [`plan_route`].
#[derive(Debug, Clone, Copy)]
pub struct RouteOptions {
    /// Grid resolution, m.
    pub cell: f32,
    /// Foot height wanted at the destination (`None` = any floor).
    pub to_y: Option<f32>,
    /// Foot height at the start, if it is not the ground floor.
    pub from_y: Option<f32>,
}

impl Default for RouteOptions {
    fn default() -> Self {
        RouteOptions { cell: 0.1, to_y: None, from_y: None }
    }
}

/// A planned route that has been replayed with the real physics.
#[derive(Debug, Clone)]
pub struct Route {
    /// Waypoints to feed `walk --path` / `checks.walk[].path`.
    pub waypoints: Vec<Vec2>,
    /// The validated replay of `waypoints` (every leg reached).
    pub steps: Vec<WalkStep>,
    /// Extra body clearance the route was planned with, m (larger = safer).
    pub margin: f32,
    /// Total planned length, m.
    pub length: f32,
}

impl Route {
    /// `x,z; x,z; ...` with two decimals, exactly what was validated.
    pub fn path_string(&self) -> String {
        self.waypoints.iter().map(|p| format!("{:.2},{:.2}", p.x, p.y)).collect::<Vec<_>>().join("; ")
    }
}

type Key = (u32, u32, i32);

fn round2(v: Vec2) -> Vec2 {
    Vec2::new((v.x * 100.0).round() / 100.0, (v.y * 100.0).round() / 100.0)
}

/// Grid A* from `from` to `to` for a body of `PLAYER_RADIUS + margin`. Returns the cell centres with the floor height at each.
fn astar(world: &MapWorld, from: Vec2, from_y: f32, to: Vec2, opts: &RouteOptions, margin: f32) -> Result<Vec<(Vec2, f32)>, String> {
    let (smin, smax) = world.solid_bounds();
    let bmin = (smin - Vec2::splat(3.0)).min(from.min(to) - Vec2::splat(1.0));
    let bmax = (smax + Vec2::splat(3.0)).max(from.max(to) + Vec2::splat(1.0));
    let cell = opts.cell.max(0.05);
    let nx = ((bmax.x - bmin.x) / cell).ceil() as usize + 1;
    let nz = ((bmax.y - bmin.y) / cell).ceil() as usize + 1;
    let grid = ColliderGrid::with_margin(&world.colliders, bmin, bmax, margin);
    let radius = PLAYER_RADIUS + margin;
    let center = |ix: usize, iz: usize| Vec2::new(bmin.x + ix as f32 * cell, bmin.y + iz as f32 * cell);
    let cell_of = |p: Vec2| (((p.x - bmin.x) / cell).round() as isize, ((p.y - bmin.y) / cell).round() as isize);
    let key = |ix: usize, iz: usize, y: f32| -> Key { (ix as u32, iz as u32, (y * 20.0).round() as i32) };

    // Start: the given cell, or the nearest free one within a metre (a waypoint typed by hand may sit a hair inside a wall).
    let (sx, sz) = cell_of(from);
    let mut start = None;
    'find: for ring in 0..=((1.0 / cell).ceil() as isize) {
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
                let y = ground_height_at(&world.ground, p, from_y);
                if !grid.blocked_r(p, y, radius) {
                    start = Some((ix as usize, iz as usize, y));
                    break 'find;
                }
            }
        }
    }
    let Some((ix0, iz0, y0)) = start else {
        return Err(format!("the start ({:.2}, {:.2}) is inside something solid (no free floor within 1 m)", from.x, from.y));
    };

    let goal_ok = |p: Vec2, y: f32| (p - to).length() <= cell * 1.5 && opts.to_y.is_none_or(|ty| (y - ty).abs() <= 0.3);
    let h = |p: Vec2| (p - to).length();
    let mut open: BinaryHeap<Reverse<(u32, u32, u32, i32, u32)>> = BinaryHeap::new(); // (f_mm, ix, iz, y_bucket, g_mm)
    let mut best_g: HashMap<Key, f32> = HashMap::new();
    let mut parent: HashMap<Key, Key> = HashMap::new();
    let mut ys: HashMap<Key, f32> = HashMap::new();
    let k0 = key(ix0, iz0, y0);
    best_g.insert(k0, 0.0);
    ys.insert(k0, y0);
    open.push(Reverse(((h(center(ix0, iz0)) * 1000.0) as u32, ix0 as u32, iz0 as u32, k0.2, 0)));
    const DIRS: [(isize, isize); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
    let mut expanded = 0usize;
    while let Some(Reverse((_f, ix, iz, yb, g_mm))) = open.pop() {
        let (ix, iz) = (ix as usize, iz as usize);
        let k: Key = (ix as u32, iz as u32, yb);
        let g = g_mm as f32 / 1000.0;
        if best_g.get(&k).is_some_and(|&b| g > b + 1e-3) {
            continue;
        }
        let y = ys[&k];
        if goal_ok(center(ix, iz), y) {
            let mut out = vec![(center(ix, iz), y)];
            let mut cur = k;
            while let Some(&p) = parent.get(&cur) {
                out.push((center(p.0 as usize, p.1 as usize), ys[&p]));
                cur = p;
            }
            out.reverse();
            return Ok(out);
        }
        expanded += 1;
        if expanded > 3_000_000 {
            return Err("route search gave up (map too large for this cell size; try --cell 0.2)".into());
        }
        for (dx, dz) in DIRS {
            let (jx, jz) = (ix as isize + dx, iz as isize + dz);
            if jx < 0 || jz < 0 || jx as usize >= nx || jz as usize >= nz {
                continue;
            }
            let (jx, jz) = (jx as usize, jz as usize);
            let p = center(jx, jz);
            if grid.blocked_r(p, y, radius) {
                continue;
            }
            let ny = ground_height_at(&world.ground, p, y);
            // Diagonal moves must not cut a corner: both orthogonal neighbours have to be free too.
            if dx != 0 && dz != 0 && (grid.blocked_r(center(jx, iz), y, radius) || grid.blocked_r(center(ix, jz), y, radius)) {
                continue;
            }
            let step = if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 } * cell;
            // Walking off a ledge is legal but a bad default route: make it expensive so stairs win when both exist.
            let drop = (y - ny).max(0.0);
            let ng = g + step + if drop > crate::tools::reach::DROP_THRESHOLD { 3.0 + drop } else { 0.0 };
            let nk = key(jx, jz, ny);
            if best_g.get(&nk).is_none_or(|&b| ng < b - 1e-3) {
                best_g.insert(nk, ng);
                parent.insert(nk, k);
                ys.insert(nk, ny);
                open.push(Reverse((((ng + h(p)) * 1000.0) as u32, jx as u32, jz as u32, nk.2, (ng * 1000.0) as u32)));
            }
        }
    }
    Err(format!("no walkable route from ({:.2}, {:.2}) to ({:.2}, {:.2}) with a {:.2} m body", from.x, from.y, to.x, to.y, radius * 2.0))
}

/// Whether the real physics walks straight from `a` (feet at `ay`) to `b` and ends on floor height `by`.
fn segment_ok(world: &MapWorld, a: Vec2, ay: f32, b: Vec2, by: f32) -> bool {
    let steps = walk_from(world, a, ay, &[b]);
    steps.last().is_some_and(|s| s.reached && (s.foot_y - by).abs() <= 0.3)
}

/// Removes waypoints the physics can walk straight past: from each point, gallop to the farthest cell that is still one
/// clean straight walk, then bisect. Every accepted segment is verified, so the result is never worse than the input.
fn simplify(world: &MapWorld, pts: &[(Vec2, f32)]) -> Vec<Vec2> {
    let n = pts.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < n {
        let ok = |j: usize| segment_ok(world, pts[i].0, pts[i].1, pts[j].0, pts[j].1);
        // Gallop.
        let mut good = i + 1;
        let mut step = 2usize;
        let mut bad = None;
        while good < n - 1 {
            let j = (i + step).min(n - 1);
            if ok(j) {
                good = j;
                if j == n - 1 {
                    break;
                }
                step *= 2;
            } else {
                bad = Some(j);
                break;
            }
        }
        // Bisect between the last good and first bad.
        if let Some(mut hi) = bad {
            let mut lo = good;
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if ok(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            good = lo;
        }
        out.push(pts[good].0);
        i = good;
    }
    if out.is_empty() {
        out.push(pts[n - 1].0);
    }
    out
}

/// Plans and *verifies* a route from `from` to `to` (world XZ). Tries progressively tighter body margins (0.15, 0.08, 0 m
/// extra) and returns the first route the real per-tick physics walks end to end.
pub fn plan_route(world: &MapWorld, from: Vec2, to: Vec2, opts: &RouteOptions) -> Result<Route, String> {
    let from_y = ground_height_at(&world.ground, from, opts.from_y.unwrap_or(0.0));
    let mut last_err = String::new();
    for margin in [0.15f32, 0.08, 0.0] {
        let cells = match astar(world, from, from_y, to, opts, margin) {
            Ok(c) => c,
            Err(e) => {
                last_err = e;
                continue;
            }
        };
        let length: f32 = cells.windows(2).map(|w| (w[1].0 - w[0].0).length()).sum();
        // Candidate waypoint sets, most economical first: physics-simplified, then a dense fallback every ~0.6 m.
        let simplified: Vec<Vec2> = simplify(world, &cells).into_iter().map(round2).collect();
        let dense: Vec<Vec2> = cells.iter().skip(1).step_by(((0.6 / opts.cell).round() as usize).max(1)).map(|c| round2(c.0)).chain(std::iter::once(round2(to))).collect();
        for wps in [simplified, dense] {
            let steps = walk_from(world, from, from_y, &wps);
            if steps.len() == wps.len() && steps.iter().all(|s| s.reached) && opts.to_y.is_none_or(|ty| steps.last().is_some_and(|s| (s.foot_y - ty).abs() <= 0.3)) {
                return Ok(Route { waypoints: wps, steps, margin, length });
            }
            last_err = match steps.last() {
                Some(s) if !s.reached => format!("planned route failed physics validation at ({:.2}, {:.2})", s.pos.x, s.pos.y),
                _ => "planned route ended on the wrong floor".to_string(),
            };
        }
    }
    Err(last_err)
}

/// A plan image of a walk: the route (yellow numbered path), where it stopped (red ring at the real body radius), the
/// objects touching the player (red boxes with ids) and the walkable area, zoomed to the action. `walk --explain out.png`.
pub fn explain_image(world: &MapWorld, start: Vec2, waypoints: &[Vec2], steps: &[WalkStep], diag: Option<&Diagnosis>) -> image::RgbImage {
    use super::plan::{render_png, Overlay, PlanOptions};
    let mut pts = vec![start];
    pts.extend(waypoints.iter().copied());
    let mut lo = pts.iter().fold(Vec2::splat(f32::INFINITY), |m, p| m.min(*p));
    let mut hi = pts.iter().fold(Vec2::splat(f32::NEG_INFINITY), |m, p| m.max(*p));
    for st in steps {
        lo = lo.min(st.pos);
        hi = hi.max(st.pos);
    }
    let mut overlays = vec![Overlay::Path { pts: pts.clone(), color: [255, 214, 60] }];
    let mut y = steps.first().map_or(0.0, |s| s.foot_y);
    let ok = diag.is_none();
    if let Some(last) = steps.last().filter(|s| !s.reached) {
        y = last.foot_y;
        overlays.push(Overlay::Ring { at: last.pos, radius: PLAYER_RADIUS, color: [255, 70, 70] });
        overlays.push(Overlay::Marker { at: last.pos, label: "STOPPED".into(), color: [255, 70, 70] });
        overlays.push(Overlay::Marker { at: last.target, label: "TARGET".into(), color: [90, 200, 255] });
    } else if let Some(last) = steps.last() {
        overlays.push(Overlay::Marker { at: last.pos, label: "END".into(), color: [90, 230, 120] });
    }
    if let Some(d) = diag {
        for b in d.blockers.iter().take(4) {
            lo = lo.min(b.bounds.0);
            hi = hi.max(b.bounds.1);
            overlays.push(Overlay::Rect { min: b.bounds.0, max: b.bounds.1, label: b.id.clone(), color: if b.ahead { [255, 60, 60] } else { [255, 170, 50] } });
        }
    }
    overlays.push(Overlay::Marker { at: start, label: "START".into(), color: [120, 170, 255] });
    let pad = Vec2::splat(if ok { 3.0 } else { 2.5 });
    let opts = PlanOptions { y, scale: 60.0, bounds: Some((lo - pad, hi + pad)), overlays, ..Default::default() };
    let reach = super::reach::compute(world, &super::reach::ReachParams { start: Some(start), ..Default::default() });
    render_png(world, Some(&reach), &[], &opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn world(json: &str) -> MapWorld {
        MapWorld::from_text(json, Path::new("t.json")).unwrap()
    }

    /// A closed 10x16 room with a full-width partition at z=0 that has a 1.2 m door at x=3.5..4.7, and a crate off to one side.
    const ROOM: &str = r##"{"camera":{"position":[0,1.7,-3]},"objects":[
        {"id":"wall","type":"wall","from":[-5,0],"to":[5,0],"openings":[{"at":9.1,"width":1.2}]},
        {"id":"side_w","type":"wall","from":[-5,-8],"to":[-5,8]},{"id":"side_e","type":"wall","from":[5,-8],"to":[5,8]},
        {"id":"end_s","type":"wall","from":[-5,-8],"to":[5,-8]},{"id":"end_n","type":"wall","from":[-5,8],"to":[5,8]},
        {"id":"crate_a","type":"prop","prop":"crate","position":[1.0,0,-2.0],"material":{"color":"#aa8844"}}
    ]}"##;

    #[test]
    fn diagnosis_names_the_object_in_the_way() {
        let w = world(r##"{"camera":{"position":[0,1.7,-3]},"objects":[
            {"id":"crate_a","type":"prop","prop":"crate","position":[0,0,0],"material":{"color":"#aa8844"}}]}"##);
        // Walk straight at the crate and stop against it.
        let steps = walk_from(&w, Vec2::new(0.0, -3.0), 0.0, &[Vec2::new(0.0, 3.0)]);
        let last = steps.last().unwrap();
        assert!(!last.reached);
        let d = diagnose(&w, last.pos, last.foot_y, last.target);
        let b = d.blockers.iter().find(|b| b.ahead).expect("something ahead");
        assert_eq!(b.id, "crate_a", "{}", d.render());
        assert!(d.render().contains("BLOCKED BY 'crate_a'"), "{}", d.render());
        assert!(d.reachable, "open floor around the crate stays reachable: {}", d.render());
        assert!(d.one_line().contains("crate_a"));
    }

    #[test]
    fn diagnosis_reports_a_passage_narrower_than_the_body() {
        // Two blocks 0.5 m apart (the body needs 0.6 m).
        let w = world(r##"{"camera":{"position":[0,1.7,-3]},"objects":[
            {"id":"left","type":"box","size":[2,2.5,0.4],"position":[-1.25,1.25,0]},
            {"id":"right","type":"box","size":[2,2.5,0.4],"position":[1.25,1.25,0]}]}"##);
        let steps = walk_from(&w, Vec2::new(0.0, -3.0), 0.0, &[Vec2::new(0.0, 3.0)]);
        let last = steps.last().unwrap();
        assert!(!last.reached, "a 0.5 m gap must stop a 0.6 m body");
        let d = diagnose(&w, last.pos, last.foot_y, last.target);
        let c = d.clearance.expect("both sides measured");
        assert!((c.width - 0.5).abs() < 0.06 && c.width < c.needed, "{c:?}\n{}", d.render());
        assert!(d.render().contains("TOO NARROW"), "{}", d.render());
    }

    #[test]
    fn auto_route_goes_through_the_door_and_is_physics_validated() {
        let w = world(ROOM);
        let r = plan_route(&w, Vec2::new(0.0, -3.0), Vec2::new(0.0, 3.0), &RouteOptions::default()).expect("a route exists through the door");
        assert!(r.steps.iter().all(|s| s.reached), "{:?}", r.steps);
        // The only opening is at x = -5 + 9.1 ... = 4.1: the route must pass near it.
        assert!(r.waypoints.iter().any(|p| (p.x - 4.1).abs() < 0.8), "route should use the doorway, got {}", r.path_string());
        // And pasting the printed path back into `walk` reproduces success.
        let wps: Vec<Vec2> = r
            .path_string()
            .split(';')
            .map(|p| {
                let n: Vec<f32> = p.split(',').map(|x| x.trim().parse().unwrap()).collect();
                Vec2::new(n[0], n[1])
            })
            .collect();
        let again = walk_from(&w, Vec2::new(0.0, -3.0), 0.0, &wps);
        assert!(again.iter().all(|s| s.reached) && again.len() == wps.len());
    }

    #[test]
    fn a_crate_parked_in_front_of_the_door_is_named_and_the_route_fails() {
        let sealed = ROOM.replace(r#""position":[1.0,0,-2.0]"#, r#""position":[4.1,0,-1.0]"#);
        let w = world(&sealed);
        assert!(plan_route(&w, Vec2::new(0.0, -3.0), Vec2::new(0.0, 3.0), &RouteOptions::default()).is_err(), "crate seals the doorway");
        // Walking at the door stops against the crate or the wall beside it; the diagnosis must name what is touching.
        let steps = walk_from(&w, Vec2::new(4.1, -4.0), 0.0, &[Vec2::new(4.1, 3.0)]);
        let last = steps.last().unwrap();
        assert!(!last.reached);
        let d = diagnose(&w, last.pos, last.foot_y, last.target);
        assert!(d.blockers.iter().any(|b| b.ahead && b.id == "crate_a"), "{}", d.render());
        assert!(!d.reachable, "the doorway is sealed, so nothing beyond it is reachable: {}", d.render());
    }

    #[test]
    fn auto_route_climbs_stairs_and_ends_on_the_upper_floor() {
        let w = world(
            r##"{"camera":{"position":[0,1.7,-3]},"objects":[
            {"id":"st","type":"stairs","position":[0,0,3],"width":1.2,"run":4.0,"rise":2.8,"steps":14},
            {"id":"deck","type":"box","size":[6,0.2,4],"position":[0,2.7,7]}]}"##,
        );
        let r = plan_route(&w, Vec2::new(0.0, -2.0), Vec2::new(0.0, 6.5), &RouteOptions { to_y: Some(2.8), ..Default::default() }).expect("stairs route");
        assert!((r.steps.last().unwrap().foot_y - 2.8).abs() < 0.1, "{:?}", r.steps.last());
    }

    #[test]
    fn auto_route_reports_a_sealed_target() {
        let w = world(r##"{"camera":{"position":[0,1.7,-3]},"objects":[
            {"id":"n","type":"wall","from":[-2,2],"to":[2,2]},{"id":"e","type":"wall","from":[2,2],"to":[2,6]},
            {"id":"s","type":"wall","from":[2,6],"to":[-2,6]},{"id":"w","type":"wall","from":[-2,6],"to":[-2,2]}]}"##);
        assert!(plan_route(&w, Vec2::new(0.0, -3.0), Vec2::new(0.0, 4.0), &RouteOptions::default()).is_err());
    }
}
