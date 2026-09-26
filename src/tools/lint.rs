//! `red_engine2 lint`: static analysis of a map for the mistakes that make a level feel broken.
//!
//! Each check has a stable code (shown as `[code]`), so a finding can be silenced for one
//! object with `"lint_ignore": ["code"]` in the scene JSON when it is intentional (a wall-mounted
//! fire extinguisher legitimately "floats").
//!
//! | code           | catches                                                                     |
//! |----------------|-----------------------------------------------------------------------------|
//! | `duplicate-id` | two objects with the same id (edit tools can't address them)                |
//! | `overlap`      | props/stairs interpenetrating other props/walls/floors                      |
//! | `floating`     | a prop hovering above the surface under it (or over a void)                 |
//! | `sunk`         | a prop buried in a floor slab or the ground                                 |
//! | `headroom`     | a walkable spot with a ceiling lower than the player's height               |
//! | `stairs-*`     | stairs that don't connect: blocked/absent landings, too narrow, too steep   |
//! | `drop`         | a walkable edge with a big fall and no barrier (stairwell without a railing)|
//! | `leak`         | the player can walk off the edge of the map                                 |
//! | `zone`         | a declared zone (`zones` in the JSON) the player cannot reach               |
//! | `floor`        | a floor slab (walls stand on it) that can't be walked onto                  |
//! | `unreachable`  | a prop the player can't get near (sealed behind walls/other props)          |
//! | `spawn`        | the player spawns inside something                                          |
//! | `light`        | a light placed inside solid geometry                                        |
//! | `z-fight`      | two coplanar overlapping planes that will flicker                           |
//! | `door`         | a connection between zones narrower than a comfortable doorway              |
//! | `door-blocked` | a door/arch opening you can't actually walk through (furniture in front of it)|

use super::reach::{Reach, DROP_THRESHOLD};
use super::world::{Item, ItemKind, MapWorld};
use crate::collide::ground_height_at;
use crate::player::{MIN_COMFORTABLE_DOOR_WIDTH, PLAYER_HEADROOM};
use crate::props::PropKind;
use crate::schema::LightKind;
use glam::{Vec2, Vec3};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Finding severity: errors fail `lint`, warnings only with `--strict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

impl Severity {
    /// Lower-case label (`error`/`warning`).
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warn => "WARN ",
            Severity::Info => "INFO ",
        }
    }
}

/// One lint finding: severity, stable code, message, and a world position to look at.
#[derive(Debug, Clone)]
pub struct Finding {
    pub sev: Severity,
    pub code: &'static str,
    pub message: String,
    /// World position to look at (for `plan` overlays and `frame --at`).
    pub at: Option<Vec3>,
    /// Top-level object ids involved.
    pub ids: Vec<String>,
}

fn finding(sev: Severity, code: &'static str, message: String, at: Option<Vec3>, ids: &[&str]) -> Finding {
    Finding { sev, code, message, at, ids: ids.iter().map(|s| s.to_string()).collect() }
}

/// Findings as JSON (for `--json` and the MCP server).
pub fn to_json(findings: &[Finding]) -> Value {
    Value::Array(
        findings
            .iter()
            .map(|f| {
                json!({
                    "severity": f.sev.label().trim(),
                    "code": f.code,
                    "message": f.message,
                    "at": f.at.map(|p| json!([p.x, p.y, p.z])),
                    "ids": f.ids,
                })
            })
            .collect(),
    )
}

fn overlap_1d(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    a1.min(b1) - a0.max(b0)
}

fn contains_xz(min: Vec3, max: Vec3, p: Vec2) -> bool {
    p.x >= min.x && p.x <= max.x && p.y >= min.z && p.y <= max.z
}

/// Runs every lint check over a loaded map and its reachability analysis.
pub fn lint(world: &MapWorld, reach: &Reach) -> Vec<Finding> {
    let mut out = Vec::new();
    check_duplicate_ids(world, &mut out);
    check_overlaps(world, &mut out);
    check_support(world, &mut out);
    check_headroom(world, reach, &mut out);
    check_stairs(world, reach, &mut out);
    check_reach(world, reach, &mut out);
    check_lights(world, &mut out);
    check_z_fight(world, &mut out);
    check_openings(world, &mut out);
    out.sort_by(|a, b| b.sev.cmp(&a.sev).then(a.code.cmp(b.code)));
    out
}

// ---------------------------------------------------------------------------------------------

fn check_duplicate_ids(world: &MapWorld, out: &mut Vec<Finding>) {
    fn walk(objs: &[Value], seen: &mut HashMap<String, usize>) {
        for o in objs {
            if let Some(id) = o.get("id").and_then(Value::as_str) {
                *seen.entry(id.to_string()).or_default() += 1;
            }
            if let Some(kids) = o.get("children").and_then(Value::as_array) {
                walk(kids, seen);
            }
        }
    }
    let mut seen = HashMap::new();
    if let Some(arr) = world.raw.get("objects").and_then(Value::as_array) {
        walk(arr, &mut seen);
    }
    let mut dups: Vec<_> = seen.into_iter().filter(|(_, n)| *n > 1).collect();
    dups.sort();
    for (id, n) in dups {
        out.push(finding(
            Severity::Error,
            "duplicate-id",
            format!("id '{id}' is used by {n} objects — edit tools (set/move/rm) can't address them uniquely"),
            None,
            &[&id],
        ));
    }
}

/// Item pairs worth an interpenetration test: at least one side must be a prop or stairs;
/// box-vs-box overlaps (walls meeting at corners, slabs under walls) are normal construction.
fn overlap_candidates(a: &Item, b: &Item) -> bool {
    // Tileable props (fence sections share their end posts) are meant to abut/overlap slightly.
    if let (ItemKind::Prop(ka), ItemKind::Prop(kb)) = (a.kind, b.kind) {
        if ka == kb && matches!(ka, PropKind::FenceSection | PropKind::Hedge) {
            return false;
        }
    }
    let interesting = |i: &Item| matches!(i.kind, ItemKind::Prop(_) | ItemKind::Stairs);
    (interesting(a) || interesting(b)) && a.is_solid() && b.is_solid() && a.top_id != b.top_id
}

fn check_overlaps(world: &MapWorld, out: &mut Vec<Finding>) {
    const MIN_DEPTH: f32 = 0.06;
    let items: Vec<&Item> = world.items.iter().filter(|i| i.is_solid()).collect();
    // (top A, top B) -> (worst depth product, message, pos, leaf ids)
    let mut worst: HashMap<(String, String), (f32, String, Vec3)> = HashMap::new();
    for (i, a) in items.iter().enumerate() {
        for b in items.iter().skip(i + 1) {
            if !overlap_candidates(a, b) || a.ignores("overlap") || b.ignores("overlap") {
                continue;
            }
            if overlap_1d(a.min.x, a.max.x, b.min.x, b.max.x) <= 0.0
                || overlap_1d(a.min.z, a.max.z, b.min.z, b.max.z) <= 0.0
                || overlap_1d(a.min.y, a.max.y, b.min.y, b.max.y) <= 0.0
            {
                continue;
            }
            for (amin, amax) in &a.volumes {
                for (bmin, bmax) in &b.volumes {
                    let d = Vec3::new(
                        overlap_1d(amin.x, amax.x, bmin.x, bmax.x),
                        overlap_1d(amin.y, amax.y, bmin.y, bmax.y),
                        overlap_1d(amin.z, amax.z, bmin.z, bmax.z),
                    );
                    if d.x < MIN_DEPTH || d.y < MIN_DEPTH || d.z < MIN_DEPTH {
                        continue;
                    }
                    let key = if a.top_id < b.top_id { (a.top_id.clone(), b.top_id.clone()) } else { (b.top_id.clone(), a.top_id.clone()) };
                    let score = d.x * d.y * d.z;
                    let at = (amin.max(*bmin) + amax.min(*bmax)) * 0.5;
                    let msg = format!("'{}' overlaps '{}' by {:.2} x {:.2} x {:.2} m (x,y,z)", a.id, b.id, d.x, d.y, d.z);
                    let e = worst.entry(key).or_insert((0.0, String::new(), Vec3::ZERO));
                    if score > e.0 {
                        *e = (score, msg, at);
                    }
                }
            }
        }
    }
    let mut v: Vec<_> = worst.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap());
    for ((ta, tb), (_, msg, at)) in v {
        out.push(finding(Severity::Error, "overlap", format!("{msg} near ({:.1}, {:.1}, {:.1})", at.x, at.y, at.z), Some(at), &[&ta, &tb]));
    }
}

/// Everything a prop could be resting on: box tops, plane heights, and other props' solid parts.
struct Surface<'a> {
    top: f32,
    min: Vec3,
    max: Vec3,
    owner: &'a Item,
}

/// Something that should rest on a surface: a `prop`, or a floor-mounted `prefab` instance (all its
/// pieces together — its base is the lowest piece, its center the instance origin).
struct Placed<'a> {
    top_id: &'a str,
    base: f32,
    center: Vec2,
    support_samples: Vec<Vec2>,
    ignore: &'a [String],
}

impl Placed<'_> {
    fn ignores(&self, check: &str) -> bool {
        self.ignore.iter().any(|c| c == check || c == "all")
    }
}

/// Whether the prefab instance named `name` is mounted on the floor (vs. on a wall).
fn prefab_is_floor_mounted(raw: &Value, name: &str) -> bool {
    if let Some(local) = raw.get("prefabs") {
        let def = local.get(name).or_else(|| local.as_array().and_then(|a| a.iter().find(|d| d.get("name").and_then(Value::as_str) == Some(name))));
        if let Some(m) = def.and_then(|d| d.get("mount")).and_then(Value::as_str) {
            return m == "floor";
        }
    }
    crate::prefabs::builtin().0.find(name).is_none_or(|d| d.mount == "floor")
}

fn placed_things(world: &MapWorld) -> Vec<Placed<'_>> {
    let mut out: Vec<Placed> = world
        .items
        .iter()
        .filter(|i| i.is_prop())
        .map(|it| Placed {
            top_id: &it.top_id,
            base: it.min.y,
            center: it.footprint.map(|f| f.center).unwrap_or(Vec2::new((it.min.x + it.max.x) * 0.5, (it.min.z + it.max.z) * 0.5)),
            support_samples: it.footprint.map_or_else(
                || {
                    let min = Vec2::new(it.min.x, it.min.z);
                    let max = Vec2::new(it.max.x, it.max.z);
                    let center = (min + max) * 0.5;
                    vec![center, min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)]
                },
                |footprint| {
                    let corners = footprint.corners();
                    let mut points = vec![footprint.center];
                    points.extend(corners);
                    points.extend(corners.into_iter().map(|corner| (corner + footprint.center) * 0.5));
                    points
                },
            ),
            ignore: &it.ignore,
        })
        .collect();
    let objs = world.raw.get("objects").and_then(Value::as_array);
    for o in objs.into_iter().flatten() {
        if o.get("type").and_then(Value::as_str) != Some("prefab") {
            continue;
        }
        let (Some(id), Some(name)) = (o.get("id").and_then(Value::as_str), o.get("prefab").and_then(Value::as_str)) else { continue };
        if !prefab_is_floor_mounted(&world.raw, name) {
            continue;
        }
        let pieces: Vec<&Item> = world.items.iter().filter(|i| i.top_id == id).collect();
        let Some(first) = pieces.first() else { continue };
        let base = pieces.iter().map(|i| i.min.y).fold(f32::INFINITY, f32::min);
        let min = pieces.iter().fold(Vec2::splat(f32::INFINITY), |v, item| v.min(Vec2::new(item.min.x, item.min.z)));
        let max = pieces.iter().fold(Vec2::splat(f32::NEG_INFINITY), |v, item| v.max(Vec2::new(item.max.x, item.max.z)));
        let center = Vec2::new(first.origin.x, first.origin.z);
        out.push(Placed {
            top_id: id,
            base,
            center,
            support_samples: vec![center, min, max, Vec2::new(min.x, max.y), Vec2::new(max.x, min.y)],
            ignore: &first.ignore,
        });
    }
    out
}

fn check_support(world: &MapWorld, out: &mut Vec<Finding>) {
    let mut surfaces: Vec<Surface> = Vec::new();
    for it in &world.items {
        match it.kind {
            ItemKind::Box | ItemKind::Other => surfaces.push(Surface { top: it.max.y, min: it.min, max: it.max, owner: it }),
            ItemKind::Plane => surfaces.push(Surface { top: it.max.y, min: it.min, max: it.max, owner: it }),
            ItemKind::Prop(_) => {
                for (mn, mx) in &it.volumes {
                    surfaces.push(Surface { top: mx.y, min: *mn, max: *mx, owner: it });
                }
            }
            _ => {}
        }
    }
    for it in placed_things(world) {
        if it.ignores("floating") && it.ignores("sunk") {
            continue;
        }
        let base = it.base;
        let center = it.center;
        // Highest surface directly under the footprint center that is at or below the prop's base.
        let mut best: (f32, Option<&Item>) = (0.0, None);
        for s in &surfaces {
            if s.owner.top_id == it.top_id {
                continue;
            }
            let supported = contains_xz(s.min, s.max, center) || it.support_samples.iter().filter(|&&sample| contains_xz(s.min, s.max, sample)).count() >= 2;
            if s.top <= base + 0.06 && supported && s.top > best.0 {
                best = (s.top, Some(s.owner));
            }
        }
        let gap = base - best.0;
        if gap > 0.08 && !it.ignores("floating") {
            let on = best.1.map(|o| format!("'{}'", o.id)).unwrap_or_else(|| "the ground".to_string());
            out.push(finding(
                Severity::Warn,
                "floating",
                format!(
                    "'{}' hovers {:.2} m above {} (its base is at y={:.2}, the surface under it at y={:.2}) near ({:.1}, {:.1})",
                    it.top_id, gap, on, base, best.0, center.x, center.y
                ),
                Some(Vec3::new(center.x, base, center.y)),
                &[it.top_id],
            ));
        }
        if it.ignores("sunk") {
            continue;
        }
        if base < -0.08 {
            out.push(finding(
                Severity::Warn,
                "sunk",
                format!("'{}' is sunk {:.2} m into the ground (base at y={:.2})", it.top_id, -base, base),
                Some(Vec3::new(center.x, base, center.y)),
                &[it.top_id],
            ));
            continue;
        }
        for s in &surfaces {
            let thin = s.max.y - s.min.y <= 0.6;
            if s.owner.top_id != it.top_id
                && matches!(s.owner.kind, ItemKind::Box)
                && thin
                && s.min.y < base - 0.03
                && s.top > base + 0.08
                && contains_xz(s.min, s.max, center)
            {
                out.push(finding(
                    Severity::Warn,
                    "sunk",
                    format!(
                        "'{}' is buried {:.2} m in '{}' (its base y={:.2} is below that slab's top y={:.2})",
                        it.top_id,
                        s.top - base,
                        s.owner.id,
                        base,
                        s.top
                    ),
                    Some(Vec3::new(center.x, base, center.y)),
                    &[it.top_id],
                ));
                break;
            }
        }
    }
}

fn check_headroom(world: &MapWorld, reach: &Reach, out: &mut Vec<Finding>) {
    let ceilings: Vec<&Item> = world.items.iter().filter(|i| i.is_solid() && !matches!(i.kind, ItemKind::Stairs)).collect();
    // worst clearance per ceiling owner
    let mut worst: HashMap<String, (f32, Vec3)> = HashMap::new();
    let stride = ((0.4 / reach.cell).round() as usize).max(1);
    for (i, levels) in reach.levels.iter().enumerate() {
        if levels.is_empty() || !(i % reach.nx).is_multiple_of(stride) || !(i / reach.nx).is_multiple_of(stride) {
            continue;
        }
        let p = reach.cell_center(i);
        for &y in levels {
            for c in &ceilings {
                if c.ignores("headroom") {
                    continue;
                }
                for (mn, mx) in &c.volumes {
                    if contains_xz(*mn, *mx, p) && mn.y > y + 0.05 && mn.y < y + PLAYER_HEADROOM {
                        let clearance = mn.y - y;
                        let e = worst.entry(c.top_id.clone()).or_insert((f32::INFINITY, Vec3::ZERO));
                        if clearance < e.0 {
                            *e = (clearance, Vec3::new(p.x, y, p.y));
                        }
                    }
                }
            }
        }
    }
    for (id, (clearance, at)) in worst {
        out.push(finding(
            Severity::Error,
            "headroom",
            format!(
                "'{id}' leaves only {:.2} m of headroom (need {:.2}) over walkable floor at ({:.1}, {:.1}), y={:.1}",
                clearance, PLAYER_HEADROOM, at.x, at.z, at.y
            ),
            Some(at),
            &[&id],
        ));
    }
}

fn check_stairs(world: &MapWorld, reach: &Reach, out: &mut Vec<Finding>) {
    for it in world.items.iter().filter(|i| i.stairs.is_some()) {
        let s = it.stairs.unwrap();
        let base = s.base_y();
        let top = base + s.rise;
        let id = it.top_id.as_str();
        if s.width < 0.8 {
            out.push(finding(
                Severity::Error,
                "stairs-narrow",
                format!("stairs '{id}' are {:.2} m wide; the player needs > 0.8 m between the side rails (use >= 1.1)", s.width),
                Some(it.origin),
                &[id],
            ));
        } else if s.width < 1.0 {
            out.push(finding(
                Severity::Warn,
                "stairs-narrow",
                format!("stairs '{id}' are only {:.2} m wide — a tight squeeze (>= 1.1 recommended)", s.width),
                Some(it.origin),
                &[id],
            ));
        }
        let step_h = s.rise / s.steps as f32;
        let tread = s.run / s.steps as f32;
        if step_h > 0.22 || tread < 0.22 {
            out.push(finding(
                Severity::Warn,
                "stairs-steep",
                format!("stairs '{id}': {} steps of {:.2} m rise x {:.2} m tread look too steep (aim for <= 0.20 x >= 0.26)", s.steps, step_h, tread),
                Some(it.origin),
                &[id],
            ));
        }
        let bottom_pt = s.point(-0.5, 0.0);
        let top_pt = s.point(s.run + 0.5, 0.0);
        if !reach.reachable(bottom_pt, base, 0.2) {
            out.push(finding(
                Severity::Error,
                "stairs-bottom",
                format!("stairs '{id}': the bottom landing ({:.1}, {:.1}) at y={:.1} is not reachable — nothing to walk in from (blocked by a wall/prop, or no floor there)", bottom_pt.x, bottom_pt.y, base),
                Some(Vec3::new(bottom_pt.x, base, bottom_pt.y)),
                &[id],
            ));
        }
        if !reach.reachable(top_pt, top, 0.2) {
            let floor_there = ground_height_at(&world.ground, top_pt, top);
            let why = if (floor_there - top).abs() > 0.1 {
                format!(
                    "there is no floor at y={:.1} beyond the top step (ground there is y={:.2}) — the floor slab must start where the stairs end",
                    top, floor_there
                )
            } else {
                "something blocks the exit (a wall or prop right after the top step)".to_string()
            };
            out.push(finding(
                Severity::Error,
                "stairs-top",
                format!("stairs '{id}' lead nowhere: the top landing ({:.1}, {:.1}) at y={:.1} is not reachable — {why}", top_pt.x, top_pt.y, top),
                Some(Vec3::new(top_pt.x, top, top_pt.y)),
                &[id],
            ));
        }
    }
}

fn check_reach(world: &MapWorld, reach: &Reach, out: &mut Vec<Finding>) {
    if !reach.start_ok {
        out.push(finding(
            Severity::Error,
            "spawn",
            format!("the player spawn ({:.1}, {:.1}) is inside solid geometry with no free space within 1 m", reach.start.x, reach.start.y),
            Some(Vec3::new(reach.start.x, 0.0, reach.start.y)),
            &[],
        ));
        return;
    }
    if reach.start_adjusted {
        out.push(finding(
            Severity::Warn,
            "spawn",
            format!("the player spawn ({:.1}, {:.1}) overlaps a collider; the player would be pushed out", reach.start.x, reach.start.y),
            Some(Vec3::new(reach.start.x, 0.0, reach.start.y)),
            &[],
        ));
    }
    if reach.truncated {
        out.push(finding(Severity::Warn, "reach", "reachability search hit its state limit; results are partial (try a larger --cell)".to_string(), None, &[]));
    }

    // Perimeter leaks: group border cells into ~4 m clusters.
    if !reach.leaks.is_empty() {
        let mut clusters: Vec<(Vec2, usize)> = Vec::new();
        for p in &reach.leaks {
            match clusters.iter_mut().find(|c| (c.0 - *p).length() < 4.0) {
                Some(c) => c.1 += 1,
                None => clusters.push((*p, 1)),
            }
        }
        let list: Vec<String> = clusters.iter().take(4).map(|(p, n)| format!("({:.0}, {:.0})~{}", p.x, p.y, n)).collect();
        out.push(finding(
            Severity::Error,
            "leak",
            format!(
                "the player can walk off the map: {} border cells reachable in {} place(s), near {} — close the perimeter (fence/wall gap)",
                reach.leaks.len(),
                clusters.len(),
                list.join(", ")
            ),
            Some(Vec3::new(clusters[0].0.x, 0.0, clusters[0].0.y)),
            &[],
        ));
    }

    for (p, from, to, n) in reach.drop_clusters() {
        out.push(finding(
            Severity::Warn,
            "drop",
            format!(
                "unprotected {:.1} m drop near ({:.1}, {:.1}): walkable floor at y={:.1} ends with nothing to stop the player (~{} cells) — add a railing/wall",
                from - to,
                p.x,
                p.y,
                from,
                n
            ),
            Some(Vec3::new(p.x, from, p.y)),
            &[],
        ));
    }

    // Zones.
    for z in &world.zones {
        let (got, free) = reach.area_in(world, z.min, z.max, z.y, 0.35);
        if free < 0.5 {
            out.push(finding(
                Severity::Warn,
                "zone",
                format!("zone '{}' has almost no free floor at y={:.1} ({:.1} m^2) — is its rect/y right?", z.id, z.y, free),
                Some(Vec3::new((z.min.x + z.max.x) * 0.5, z.y, (z.min.y + z.max.y) * 0.5)),
                &[],
            ));
        } else if got < 0.5 {
            out.push(finding(
                Severity::Error,
                "zone",
                format!("zone '{}' is UNREACHABLE: 0 of {:.1} m^2 can be walked to from the spawn", z.id, free),
                Some(Vec3::new((z.min.x + z.max.x) * 0.5, z.y, (z.min.y + z.max.y) * 0.5)),
                &[],
            ));
        } else if got / free < 0.6 {
            out.push(finding(
                Severity::Warn,
                "zone",
                format!("zone '{}' is only {:.0}% reachable ({:.1} of {:.1} m^2) — part of it is sealed off", z.id, got / free * 100.0, got, free),
                Some(Vec3::new((z.min.x + z.max.x) * 0.5, z.y, (z.min.y + z.max.y) * 0.5)),
                &[],
            ));
        }
    }

    // Floor slabs: a big thin box that walls stand on, but nobody can walk onto.
    for it in world.items.iter().filter(|i| matches!(i.kind, ItemKind::Box)) {
        let (w, d, h) = (it.max.x - it.min.x, it.max.z - it.min.z, it.max.y - it.min.y);
        if h > 0.5 || w * d < 4.0 || it.max.y < 0.5 || it.ignores("floor") {
            continue;
        }
        let carries_walls = world.items.iter().any(|o| {
            o.top_id != it.top_id
                && matches!(o.kind, ItemKind::Box)
                && (o.max.y - o.min.y) >= 1.0
                && (o.min.y - it.max.y).abs() < 0.06
                && overlap_1d(o.min.x, o.max.x, it.min.x, it.max.x) > 0.0
                && overlap_1d(o.min.z, o.max.z, it.min.z, it.max.z) > 0.0
        });
        if !carries_walls {
            continue;
        }
        let mut hit = false;
        'scan: for iz in 0..reach.nz {
            let z = reach.min.y + iz as f32 * reach.cell;
            if z < it.min.z || z > it.max.z {
                continue;
            }
            for ix in 0..reach.nx {
                let x = reach.min.x + ix as f32 * reach.cell;
                if x >= it.min.x && x <= it.max.x && reach.levels[iz * reach.nx + ix].iter().any(|l| (l - it.max.y).abs() < 0.12) {
                    hit = true;
                    break 'scan;
                }
            }
        }
        if !hit {
            out.push(finding(
                Severity::Error,
                "floor",
                format!("floor '{}' (top y={:.2}, {:.0} m^2, walls stand on it) can't be reached — no stairs/path leads up to it", it.id, it.max.y, w * d),
                Some(Vec3::new((it.min.x + it.max.x) * 0.5, it.max.y, (it.min.z + it.max.z) * 0.5)),
                &[&it.top_id],
            ));
        }
    }

    // Props sealed off from the walkable floor they stand on.
    let floors: Vec<f32> = reach.floors().iter().map(|f| f.0).collect();
    let mut seen_top: HashSet<&str> = HashSet::new();
    for it in world.items.iter().filter(|i| i.is_prop() && i.is_solid()) {
        if it.ignores("unreachable") || !seen_top.insert(it.top_id.as_str()) {
            continue;
        }
        let base = it.min.y;
        if !floors.iter().any(|f| (f - base).abs() < 0.12) {
            continue; // on a table/shelf/etc., not on a walkable floor
        }
        let Some(fp) = it.footprint else { continue };
        let (mn, mx) = fp.aabb();
        let (mn, mx) = (mn - Vec2::splat(0.9), mx + Vec2::splat(0.9));
        let mut near = false;
        'find: for iz in 0..reach.nz {
            let z = reach.min.y + iz as f32 * reach.cell;
            if z < mn.y || z > mx.y {
                continue;
            }
            for ix in 0..reach.nx {
                let x = reach.min.x + ix as f32 * reach.cell;
                if x >= mn.x && x <= mx.x && reach.levels[iz * reach.nx + ix].iter().any(|l| (l - base).abs() < 0.35) {
                    near = true;
                    break 'find;
                }
            }
        }
        if !near {
            out.push(finding(
                Severity::Warn,
                "unreachable",
                format!(
                    "prop '{}' at ({:.1}, {:.1}) can't be approached — it's sealed behind walls or other props (no reachable floor within 0.9 m)",
                    it.top_id, it.origin.x, it.origin.z
                ),
                Some(it.origin),
                &[&it.top_id],
            ));
        }
    }

    // Tight connections between zones.
    for (a, b, mid, width) in reach.passages(&world.zones) {
        if width < MIN_COMFORTABLE_DOOR_WIDTH - 0.05 && width > 0.0 {
            out.push(finding(
                Severity::Warn,
                "door",
                format!(
                    "the connection {a} <-> {b} near ({:.1}, {:.1}) is only ~{:.2} m wide (>= {:.1} m is comfortable)",
                    mid.x, mid.y, width, MIN_COMFORTABLE_DOOR_WIDTH
                ),
                Some(Vec3::new(mid.x, 0.0, mid.y)),
                &[],
            ));
        }
    }
    let _ = DROP_THRESHOLD;
}

/// Every `door`/`arch` opening of every `wall` object must be usable: standing in it, the player
/// must be able to reach real floor on *both* sides within a couple of meters. Catches a sofa
/// parked in front of a door, a wardrobe blocking an archway, a door leading straight into a wall.
fn check_openings(world: &MapWorld, out: &mut Vec<Finding>) {
    let Some(objs) = world.raw.get("objects").and_then(Value::as_array) else { return };
    let get2 = |v: Option<&Value>| -> Option<Vec2> {
        let a = v?.as_array()?;
        Some(Vec2::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
    };
    for o in objs.iter().filter(|o| o.get("type").and_then(Value::as_str) == Some("wall")) {
        let (Some(id), Some(from), Some(to)) = (o.get("id").and_then(Value::as_str), get2(o.get("from")), get2(o.get("to"))) else { continue };
        let Some(openings) = o.get("openings").and_then(Value::as_array) else { continue };
        let y = o.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let d = to - from;
        let len = d.length();
        if len < 0.05 {
            continue;
        }
        let (dir, normal) = (d / len, Vec2::new(-d.y, d.x) / len);
        for (i, op) in openings.iter().enumerate() {
            let kind = op.get("kind").and_then(Value::as_str).unwrap_or("door");
            if kind == "window" {
                continue;
            }
            let Some(at) = op.get("at").and_then(Value::as_f64) else { continue };
            let center = from + dir * at as f32;
            let floor = ground_height_at(&world.ground, center, y + 0.05);
            let region = super::reach::local_reach(world, center, floor, 2.4, 0.05);
            let side = |sgn: f32| region.iter().any(|p| (*p - center).dot(normal) * sgn >= 0.9);
            let (a, b) = (side(1.0), side(-1.0));
            if !(a && b) {
                let which = if !a && !b {
                    "either side"
                } else if !a {
                    "one side"
                } else {
                    "the other side"
                };
                out.push(finding(
                    Severity::Error,
                    "door-blocked",
                    format!(
                        "opening #{i} ({kind}) of '{id}' at ({:.1}, {:.1}) can't be walked through: nothing reachable to stand on beyond it on {which} within 2.4 m — furniture right in front of it, or it opens onto a wall/void",
                        center.x, center.y
                    ),
                    Some(Vec3::new(center.x, floor, center.y)),
                    &[id],
                ));
            }
        }
    }
}

fn check_lights(world: &MapWorld, out: &mut Vec<Finding>) {
    for l in &world.scene.lights {
        let LightKind::Point { position, .. } = &l.kind else { continue };
        let p = position.sample(0.0);
        for it in world.items.iter().filter(|i| matches!(i.kind, ItemKind::Box) && i.is_solid()) {
            if p.x >= it.min.x && p.x <= it.max.x && p.y >= it.min.y && p.y <= it.max.y && p.z >= it.min.z && p.z <= it.max.z {
                out.push(finding(
                    Severity::Warn,
                    "light",
                    format!("light '{}' at ({:.1}, {:.1}, {:.1}) is inside '{}' — it will be blocked/look wrong", l.id, p.x, p.y, p.z, it.id),
                    Some(p),
                    &[&it.top_id],
                ));
                break;
            }
        }
    }
}

fn check_z_fight(world: &MapWorld, out: &mut Vec<Finding>) {
    let planes: Vec<&Item> = world.items.iter().filter(|i| i.kind == ItemKind::Plane).collect();
    for (i, a) in planes.iter().enumerate() {
        for b in planes.iter().skip(i + 1) {
            if (a.max.y - b.max.y).abs() < 0.004 {
                let area = overlap_1d(a.min.x, a.max.x, b.min.x, b.max.x).max(0.0) * overlap_1d(a.min.z, a.max.z, b.min.z, b.max.z).max(0.0);
                if area > 0.25 {
                    out.push(finding(
                        Severity::Warn,
                        "z-fight",
                        format!(
                            "planes '{}' and '{}' overlap by {:.1} m^2 at the same height (y={:.3}) and will flicker — raise one by ~0.01",
                            a.id, b.id, area, a.max.y
                        ),
                        Some(Vec3::new((a.min.x + a.max.x) * 0.5, a.max.y, (a.min.z + a.max.z) * 0.5)),
                        &[&a.top_id, &b.top_id],
                    ));
                }
            }
        }
    }
}

/// One-line-per-finding text report plus a summary line.
pub fn format_report(findings: &[Finding]) -> String {
    let mut s = String::new();
    for f in findings {
        s.push_str(&format!("{} [{}] {}\n", f.sev.label(), f.code, f.message));
    }
    let count = |sev| findings.iter().filter(|f| f.sev == sev).count();
    s.push_str(&format!("{} error(s), {} warning(s), {} note(s)\n", count(Severity::Error), count(Severity::Warn), count(Severity::Info)));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::reach::{compute, ReachParams};
    use std::path::Path;

    fn run(json: &str) -> Vec<Finding> {
        let w = MapWorld::from_text(json, Path::new("t.json")).unwrap();
        let r = compute(&w, &ReachParams { cell: 0.1, ..Default::default() });
        lint(&w, &r)
    }

    fn room(extra: &str) -> String {
        format!(
            r##"{{"camera":{{"position":[0,1.7,0]}},"objects":[
            {{"id":"n","type":"wall","from":[-4,-4],"to":[4,-4]}},
            {{"id":"s","type":"wall","from":[-4,4],"to":[4,4]}},
            {{"id":"e","type":"wall","from":[4,-4],"to":[4,4]}},
            {{"id":"w","type":"wall","from":[-4,-4],"to":[-4,4]}}{extra}]}}"##
        )
    }

    #[test]
    fn clean_room_has_no_errors() {
        let f = run(&room(r##",{"id":"c","type":"prop","prop":"crate","position":[2,0,2]}"##));
        assert!(!f.iter().any(|x| x.sev == Severity::Error), "{}", format_report(&f));
    }

    #[test]
    fn prop_inside_a_wall_is_an_overlap_error() {
        let f = run(&room(r##",{"id":"c","type":"prop","prop":"crate","position":[3.9,0,0]}"##));
        assert!(f.iter().any(|x| x.code == "overlap" && x.ids.contains(&"c".to_string())), "{}", format_report(&f));
    }

    #[test]
    fn hovering_and_sunk_props_are_flagged() {
        let f =
            run(&room(r##",{"id":"up","type":"prop","prop":"crate","position":[1,1.0,1]},{"id":"down","type":"prop","prop":"chair","position":[-2,-0.4,1]}"##));
        assert!(f.iter().any(|x| x.code == "floating" && x.ids[0] == "up"), "{}", format_report(&f));
        assert!(f.iter().any(|x| x.code == "sunk" && x.ids[0] == "down"), "{}", format_report(&f));
    }

    #[test]
    fn lint_ignore_silences_a_check() {
        let f = run(&room(r##",{"id":"up","type":"prop","prop":"crate","position":[1,1.0,1],"lint_ignore":["floating"]}"##));
        assert!(!f.iter().any(|x| x.code == "floating"), "{}", format_report(&f));
    }

    #[test]
    fn open_perimeter_is_a_leak() {
        let f = run(r##"{"camera":{"position":[0,1.7,0]},"objects":[{"id":"b","type":"box","size":[1,1,1],"position":[3,0.5,3]}]}"##);
        assert!(f.iter().any(|x| x.code == "leak"), "{}", format_report(&f));
    }

    #[test]
    fn stairs_into_a_wall_are_reported_as_leading_nowhere() {
        // Stairs climb toward +Z and end at a wall — the exact bug the original house had.
        let f = run(&room(
            r##",{"id":"st","type":"stairs","position":[0,0,1.5],"width":1.2,"run":4.0,"rise":2.8,"steps":14}
            ,{"id":"deck","type":"box","size":[8,0.2,3],"position":[0,2.7,-2.5]}"##,
        ));
        assert!(f.iter().any(|x| x.code == "stairs-top" || x.code == "stairs-bottom"), "{}", format_report(&f));
    }

    #[test]
    fn missing_railing_around_a_hole_is_a_drop() {
        // A raised deck reached by stairs, with open edges and nothing to stop the player.
        let f = run(r##"{"camera":{"position":[0,1.7,3.2]},"objects":[
            {"id":"n","type":"wall","from":[-4,-4],"to":[4,-4]},
            {"id":"s","type":"wall","from":[-4,4],"to":[4,4]},
            {"id":"e","type":"wall","from":[4,-4],"to":[4,4]},
            {"id":"w","type":"wall","from":[-4,-4],"to":[-4,4]},
            {"id":"st","type":"stairs","position":[0,0,-1.5],"width":1.2,"run":3.0,"rise":2.0,"steps":10},
            {"id":"deck","type":"box","size":[6,0.2,2.5],"position":[0,1.9,1.25]}]}"##);
        assert!(f.iter().any(|x| x.code == "drop"), "{}", format_report(&f));
        assert!(
            !f.iter().any(|x| x.code == "stairs-top"),
            "the deck starts right where the stairs end:
{}",
            format_report(&f)
        );
    }
}

#[cfg(test)]
mod door_tests {
    use super::*;
    use crate::tools::reach::{compute, ReachParams};
    use std::path::Path;

    fn run(extra: &str) -> Vec<Finding> {
        let json = format!(
            r##"{{"camera":{{"position":[0,1.7,-3]}},"objects":[
            {{"id":"w","type":"wall","from":[-4,0],"to":[4,0],"openings":[{{"at":4.0,"width":1.2}}]}}{extra}]}}"##
        );
        let w = MapWorld::from_text(&json, Path::new("t.json")).unwrap();
        let r = compute(&w, &ReachParams::default());
        lint(&w, &r)
    }

    #[test]
    fn a_clear_doorway_is_fine() {
        assert!(!run("").iter().any(|f| f.code == "door-blocked"));
    }

    #[test]
    fn a_wardrobe_in_front_of_a_door_is_reported() {
        let f = run(r##",{"id":"wd","type":"prop","prop":"wardrobe","position":[0,0,1.0],"rotation":[0,180,0]}"##);
        assert!(f.iter().any(|f| f.code == "door-blocked" && f.ids[0] == "w"), "{}", format_report(&f));
    }
}

#[cfg(test)]
mod prefab_tests {
    use super::*;
    use crate::tools::reach::{compute, ReachParams};
    use std::path::Path;

    fn run(extra: &str) -> Vec<Finding> {
        let json = format!(
            r##"{{"camera":{{"position":[0,1.7,0]}},"objects":[
            {{"id":"n","type":"wall","from":[-4,-4],"to":[4,-4]}},
            {{"id":"s","type":"wall","from":[-4,4],"to":[4,4]}},
            {{"id":"e","type":"wall","from":[4,-4],"to":[4,4]}},
            {{"id":"w","type":"wall","from":[-4,-4],"to":[-4,4]}}{extra}]}}"##
        );
        let w = MapWorld::from_text(&json, Path::new("t.json")).unwrap();
        lint(&w, &compute(&w, &ReachParams::default()))
    }

    #[test]
    fn a_prefab_hovering_or_resting_is_judged_like_a_prop() {
        let f = run(
            r##",{"id":"a_up","type":"prefab","prefab":"apple_red","position":[1,1.0,1]},{"id":"a_ok","type":"prefab","prefab":"apple_red","position":[-1,0,-1]}"##,
        );
        assert!(f.iter().any(|x| x.code == "floating" && x.ids[0] == "a_up"), "{}", format_report(&f));
        assert!(!f.iter().any(|x| x.ids.first().is_some_and(|i| i == "a_ok")), "{}", format_report(&f));
    }

    #[test]
    fn a_prefab_resting_on_a_table_is_fine_and_lint_ignore_works() {
        let on_table = run(
            r##",{"id":"t","type":"prop","prop":"dining_table","position":[0,0,0]},{"id":"bowl","type":"prefab","prefab":"fruit_bowl","position":[0,0.78,0]}"##,
        );
        assert!(!on_table.iter().any(|x| x.code == "floating" || x.code == "sunk"), "{}", format_report(&on_table));
        let ignored = run(r##",{"id":"a_up","type":"prefab","prefab":"apple_red","position":[1,1.0,1],"lint_ignore":["floating"]}"##);
        assert!(!ignored.iter().any(|x| x.code == "floating"), "{}", format_report(&ignored));
    }

    #[test]
    fn an_edge_aligned_decoration_with_meaningful_support_is_not_floating() {
        let findings = run(
            r##",{"id":"tabletop","type":"box","size":[1,0.5,1],"position":[0,0.25,0]},{"id":"apple","type":"prefab","prefab":"apple_red","position":[0.53,0.5,0]}"##,
        );
        assert!(!findings.iter().any(|finding| finding.code == "floating" && finding.ids[0] == "apple"), "{}", format_report(&findings));
    }

    #[test]
    fn wall_mounted_prefabs_are_not_expected_on_the_floor() {
        let f = run(r##",{"id":"clock","type":"prefab","prefab":"wall_clock","position":[0,2.0,-3.88]}"##);
        assert!(!f.iter().any(|x| x.code == "floating"), "{}", format_report(&f));
    }
}
