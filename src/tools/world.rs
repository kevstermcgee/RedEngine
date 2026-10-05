//! `MapWorld`: a loaded scene flattened into world-space, tool-friendly pieces.
//!
//! Every map-analysis tool (`lint`, `reach`, `plan`, `ls`, `scatter`, ...) starts from this: the
//! parsed [`Scene`] (macros already expanded), one [`Item`] per leaf object with its world
//! bounds / solid volumes / footprint, the *same* collider and ground data the live viewer
//! builds, plus the optional authoring metadata that lives in the raw JSON but not in the
//! compiled scene (`zones`, per-object `lint_ignore`).

use crate::collide::{
    collect_box_colliders, collect_box_colliders_grouped_except, collect_ground_candidates, collect_ground_candidates_grouped_except, Collider2D,
    GroundCandidates,
};
use crate::geometry::trs;
use crate::props::{collision, collision_box, local_bounds, prop_parts, Collision, PropKind};
use crate::schema::{Object, ObjectKind, PrimKind, Scene};
use glam::{Mat4, Vec2, Vec3};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// An oriented rectangle in the XZ plane (rotation about Y only).
#[derive(Debug, Clone, Copy)]
pub struct Obb {
    pub center: Vec2,
    /// Half-extents along the two local axes.
    pub half: Vec2,
    /// Unit vector of the first local axis in world XZ; the second is its perpendicular.
    pub axis: Vec2,
}

impl Obb {
    /// The four corners of an oriented rectangle footprint.
    pub fn corners(&self) -> [Vec2; 4] {
        let ax = self.axis * self.half.x;
        let az = Vec2::new(-self.axis.y, self.axis.x) * self.half.y;
        [self.center - ax - az, self.center + ax - az, self.center + ax + az, self.center - ax + az]
    }

    /// Whether a point lies inside the footprint.
    pub fn contains(&self, p: Vec2) -> bool {
        let d = p - self.center;
        let u = d.dot(self.axis);
        let v = d.dot(Vec2::new(-self.axis.y, self.axis.x));
        u.abs() <= self.half.x && v.abs() <= self.half.y
    }

    /// Axis-aligned bounds `(min, max)` of the footprint.
    pub fn aabb(&self) -> (Vec2, Vec2) {
        let cs = self.corners();
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for c in cs {
            min = min.min(c);
            max = max.max(c);
        }
        (min, max)
    }
}

/// What an item is for the tools: box, plane, prop, stairs or other decor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ItemKind {
    Box,
    Plane,
    Prop(PropKind),
    Stairs,
    /// sphere / cylinder / cone / capsule primitive
    Other,
    Humanoid,
    Rat,
}

impl ItemKind {
    /// Human-readable label for the item's kind.
    pub fn label(&self) -> String {
        match self {
            ItemKind::Box => "box".into(),
            ItemKind::Plane => "plane".into(),
            ItemKind::Prop(k) => format!("prop:{}", k.name()),
            ItemKind::Stairs => "stairs".into(),
            ItemKind::Other => "prim".into(),
            ItemKind::Humanoid => "humanoid".into(),
            ItemKind::Rat => "rat".into(),
        }
    }
}

/// Geometry of a staircase item, for landing/clearance checks.
#[derive(Debug, Clone, Copy)]
pub struct StairsInfo {
    /// Stairs-local -> world (local +Z is the run axis, bottom at `-run/2`).
    pub world: Mat4,
    pub width: f32,
    pub run: f32,
    pub rise: f32,
    pub steps: u32,
}

impl StairsInfo {
    /// World XZ of a point `along` the run axis (0 = bottom end, `run` = top end), `side`
    /// meters off-center.
    pub fn point(&self, along: f32, side: f32) -> Vec2 {
        let p = self.world.transform_point3(Vec3::new(side, 0.0, along - self.run * 0.5));
        Vec2::new(p.x, p.z)
    }
    /// Height of the item's base.
    pub fn base_y(&self) -> f32 {
        self.world.transform_point3(Vec3::ZERO).y
    }
}

/// One leaf object in world space.
#[derive(Debug, Clone)]
pub struct Item {
    /// Leaf id — for macro pieces `wall_front.seg0`, for everything else the object's own id.
    pub id: String,
    /// Id of the top-level scene object this belongs to (what `set`/`rm`/`move` operate on).
    pub top_id: String,
    pub kind: ItemKind,
    /// World AABB of everything drawn.
    pub min: Vec3,
    pub max: Vec3,
    /// World AABBs of the parts that physically block/occupy space (empty = walk-through).
    pub volumes: Vec<(Vec3, Vec3)>,
    /// Oriented XZ footprint of the solid extent, if it has one.
    pub footprint: Option<Obb>,
    /// World position of the object's origin.
    pub origin: Vec3,
    /// Display color (sRGB 0..1) for plans.
    pub color: Vec3,
    /// Lint checks to skip for this item (`"lint_ignore": ["floating", ...]` in the JSON).
    pub ignore: Vec<String>,
    pub stairs: Option<StairsInfo>,
    /// True for a `terrain` object: ground, not a floor slab (its heights are read through `MapWorld::ground`).
    pub is_terrain: bool,
}

impl Item {
    /// Whether the item blocks the player.
    pub fn is_solid(&self) -> bool {
        !self.volumes.is_empty()
    }
    /// Whether this item opted out of a lint code via `lint_ignore`.
    pub fn ignores(&self, check: &str) -> bool {
        self.ignore.iter().any(|c| c == check || c == "all")
    }
    /// Whether the item is a prop.
    pub fn is_prop(&self) -> bool {
        matches!(self.kind, ItemKind::Prop(_))
    }
}

/// A named region, authored in the scene JSON's optional top-level `"zones"` array:
/// `{"id": "kitchen", "rect": [x0, z0, x1, z1], "y": 0.0}` (`y` = floor height, default 0).
/// Zones give `reach`/`lint` names to talk about ("kitchen is unreachable") and label `plan`.
#[derive(Debug, Clone)]
pub struct Zone {
    pub id: String,
    pub min: Vec2,
    pub max: Vec2,
    pub y: f32,
    /// Free-form tag, e.g. "room" or "outdoor" — `lint` expects every zone to be reachable
    /// regardless, but `reach` groups the connection listing by it.
    pub kind: String,
}

impl Zone {
    /// Whether a point lies inside the item's footprint.
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
}

/// A scene flattened into world-space items plus the game's own colliders and ground candidates; the input to every analysis tool.
pub struct MapWorld {
    pub path: PathBuf,
    pub raw: Value,
    pub scene: Scene,
    pub items: Vec<Item>,
    pub colliders: Vec<Collider2D>,
    pub ground: GroundCandidates,
    pub zones: Vec<Zone>,
    /// Where the player spawns: the first `spawns` entry's XZ, or the scene camera's XZ when the scene has no spawns.
    pub spawn: Vec2,
    /// The spawn's foot height (the first `spawns` entry's y; 0 for a camera spawn): reach, lint and walks start on that floor.
    pub spawn_y: f32,
    /// Top-level objects an unconditional `start` rule switches collision off for: the analysis tools treat them as open,
    /// the way the game does from its first tick (ADR 2026-09-29-verification-honours-the-map).
    pub collision_disabled: Vec<String>,
    /// The scene `phases` entry the world was built for (`None` = the initial state): see [`crate::sim::rules::RuleSet::open_objects`].
    pub phase: Option<String>,
}

impl MapWorld {
    /// Loads a scene file into a `MapWorld`.
    pub fn load(path: &Path) -> Result<Self, Vec<String>> {
        Self::load_phase(path, None)
    }

    /// Loads a scene file as it is in the named `phases` state (`None` = the initial state).
    pub fn load_phase(path: &Path, phase: Option<&str>) -> Result<Self, Vec<String>> {
        let text = std::fs::read_to_string(path).map_err(|e| vec![format!("io: {}: {e}", path.display())])?;
        Self::from_text_phase(&text, path, phase)
    }

    /// Builds a `MapWorld` from scene JSON text; `path` is only for messages and `checks` lookups.
    pub fn from_text(text: &str, path: &Path) -> Result<Self, Vec<String>> {
        Self::from_text_phase(text, path, None)
    }

    /// [`from_text`](Self::from_text) for a named `phases` state: what the unconditional `start` rules and then that phase's rules
    /// open (collision off, `deactivate`) is open to every tool; a gate that is closed in the initial state is walkable in `gate_open`.
    pub fn from_text_phase(text: &str, path: &Path, phase: Option<&str>) -> Result<Self, Vec<String>> {
        let scene = crate::schema::parse_scene_in(text, path.parent())?;
        let raw: Value = serde_json::from_str(text).map_err(|e| vec![format!("json: {e}")])?;
        let ignores = collect_ignores(&raw);
        // What the unconditional `start` rules open on the first tick is open to the tools too, and so is what the chosen phase's rules
        // open (a gate a game keeps closed until something happens stays solid in the initial state: the tools cannot know when).
        let collision_disabled: Vec<String> = scene.rules.open_objects(phase).map_err(|e| vec![format!("phase: {e}")])?;
        let disabled_indices: HashSet<usize> = scene.objects.iter().enumerate().filter(|(_, o)| collision_disabled.contains(&o.id)).map(|(i, _)| i).collect();
        let mut items = Vec::new();
        for o in &scene.objects {
            flatten_object(o, &o.id, Mat4::IDENTITY, !collision_disabled.contains(&o.id), &ignores, &mut items);
        }
        let zones = parse_zones(&raw);
        let cam = scene.camera.position.sample(0.0);
        // The first authored spawn is where a match (and the offline client) really starts; the camera is the fallback.
        let (spawn, spawn_y) = match crate::sim::spawns::parse_spawns(text).ok().and_then(|s| s.into_iter().next()) {
            Some(s) => (Vec2::new(s.position[0], s.position[2]), s.position[1]),
            None => (Vec2::new(cam.x, cam.z), 0.0),
        };
        let (colliders, ground) = if disabled_indices.is_empty() {
            (collect_box_colliders(&scene), collect_ground_candidates(&scene))
        } else {
            let mut ground = GroundCandidates::default();
            for g in &collect_ground_candidates_grouped_except(&scene, &disabled_indices) {
                ground.append(g);
            }
            (collect_box_colliders_grouped_except(&scene, &disabled_indices).into_iter().flatten().collect(), ground)
        };
        Ok(MapWorld {
            path: path.to_path_buf(),
            colliders,
            ground,
            scene,
            raw,
            items,
            zones,
            spawn,
            spawn_y,
            collision_disabled,
            phase: phase.map(str::to_string),
        })
    }

    /// Whether the scene is an endless generated world (a `procgen` block): ground everywhere, no edge, and trees that block the player which no authored object lists.
    /// Analysis of such a scene cannot use "the bounds of the solid things" (there are none worth the name) or look for a perimeter to seal; it looks at a *window* of
    /// the world around the points that matter ([`Self::analysis_box`]) and asks [`Self::blockers_in`] what stops the player there.
    pub fn is_endless(&self) -> bool {
        self.ground.procgen().is_some()
    }

    /// Every collider that can stop the player's body inside the XZ box `min..max`: the authored ones, then (in an endless world) the generated trunks and shrub crowns
    /// the player's own movement collides with (`ProcgenGround::colliders_near`, the very call `sim::player` makes each tick). One answer for every tool that reasons about
    /// where the player can stand, so reachability, routes and diagnostics agree with a real walk. Borrowed, and free, when the scene has no generated world.
    /// Purely visual vegetation (the grass and flowers that do not block) is not here, because it does not stop the player either.
    pub fn blockers_in(&self, min: Vec2, max: Vec2) -> std::borrow::Cow<'_, [Collider2D]> {
        match self.ground.procgen() {
            None => std::borrow::Cow::Borrowed(&self.colliders),
            Some(world) => {
                let (centre, half) = ((min + max) * 0.5, (max - min) * 0.5);
                std::borrow::Cow::Owned(world.colliders_near(centre, half.x.max(half.y).max(0.0) + 1.0, &self.colliders))
            }
        }
    }

    /// The XZ box an analysis of this world should cover, as `(min, max)`. A bounded scene: its solid bounds plus `margin` (so a leak at the edge is found), widened to
    /// hold every one of `points`. An endless world: a square of half-size `radius` around the first point (the start), widened to hold every other point with
    /// `margin` to spare, because there is no edge to measure.
    pub fn analysis_box(&self, points: &[Vec2], margin: f32, radius: f32) -> (Vec2, Vec2) {
        let hold = |(mut lo, mut hi): (Vec2, Vec2), pts: &[Vec2], pad: f32| {
            for p in pts {
                lo = lo.min(*p - Vec2::splat(pad));
                hi = hi.max(*p + Vec2::splat(pad));
            }
            (lo, hi)
        };
        if self.is_endless() {
            let centre = points.first().copied().unwrap_or(self.spawn);
            hold((centre - Vec2::splat(radius), centre + Vec2::splat(radius)), points, margin)
        } else {
            let (smin, smax) = self.solid_bounds();
            hold((smin - Vec2::splat(margin), smax + Vec2::splat(margin)), points, 1.0)
        }
    }

    /// XZ bounds of everything solid (planes excluded, so a giant ground plane doesn't count).
    pub fn solid_bounds(&self) -> (Vec2, Vec2) {
        let mut min = Vec2::splat(f32::INFINITY);
        let mut max = Vec2::splat(f32::NEG_INFINITY);
        for it in self.items.iter().filter(|i| i.is_solid()) {
            min = min.min(Vec2::new(it.min.x, it.min.z));
            max = max.max(Vec2::new(it.max.x, it.max.z));
        }
        if !min.x.is_finite() {
            return (Vec2::splat(-5.0), Vec2::splat(5.0));
        }
        (min, max)
    }

    /// All items that came from the top-level object with this id (macros expand to several).
    pub fn item_by_top_id(&self, id: &str) -> impl Iterator<Item = &Item> {
        let id = id.to_string();
        self.items.iter().filter(move |i| i.top_id == id || i.id == id)
    }
}

fn parse_zones(raw: &Value) -> Vec<Zone> {
    let mut out = Vec::new();
    let Some(arr) = raw.get("zones").and_then(Value::as_array) else { return out };
    for z in arr {
        let id = z.get("id").and_then(Value::as_str).unwrap_or("zone").to_string();
        let Some(r) = z.get("rect").and_then(Value::as_array).filter(|r| r.len() == 4) else { continue };
        let f = |i: usize| r[i].as_f64().unwrap_or(0.0) as f32;
        out.push(Zone {
            id,
            min: Vec2::new(f(0).min(f(2)), f(1).min(f(3))),
            max: Vec2::new(f(0).max(f(2)), f(1).max(f(3))),
            y: z.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32,
            kind: z.get("kind").and_then(Value::as_str).unwrap_or("room").to_string(),
        });
    }
    out
}

/// id -> lint_ignore tags, for every raw object (top-level and nested).
fn collect_ignores(raw: &Value) -> HashMap<String, Vec<String>> {
    fn walk(objs: &[Value], out: &mut HashMap<String, Vec<String>>) {
        for o in objs {
            if let (Some(id), Some(tags)) = (o.get("id").and_then(Value::as_str), o.get("lint_ignore").and_then(Value::as_array)) {
                out.insert(id.to_string(), tags.iter().filter_map(|t| t.as_str().map(str::to_string)).collect());
            }
            if let Some(kids) = o.get("children").and_then(Value::as_array) {
                walk(kids, out);
            }
        }
    }
    let mut out = HashMap::new();
    if let Some(arr) = raw.get("objects").and_then(Value::as_array) {
        walk(arr, &mut out);
    }
    out
}

fn aabb_of(world: Mat4, center: Vec3, half: Vec3) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for sx in [-1.0f32, 1.0] {
        for sy in [-1.0f32, 1.0] {
            for sz in [-1.0f32, 1.0] {
                let p = world.transform_point3(center + Vec3::new(half.x * sx, half.y * sy, half.z * sz));
                min = min.min(p);
                max = max.max(p);
            }
        }
    }
    (min, max)
}

/// Oriented XZ footprint of a local box under `world` — exact for the yaw-only transforms map
/// objects use; falls back to the AABB if the transform tilts about X/Z.
fn footprint_of(world: Mat4, center: Vec3, half: Vec3) -> Obb {
    let x_axis = Vec2::new(world.x_axis.x, world.x_axis.z);
    let z_axis = Vec2::new(world.z_axis.x, world.z_axis.z);
    let tilted = world.x_axis.y.abs() > 1e-3 || world.z_axis.y.abs() > 1e-3 || world.y_axis.x.abs() > 1e-3 || world.y_axis.z.abs() > 1e-3;
    if !tilted && x_axis.length() > 1e-6 && z_axis.length() > 1e-6 {
        let c = world.transform_point3(center);
        return Obb { center: Vec2::new(c.x, c.z), half: Vec2::new(half.x * x_axis.length(), half.z * z_axis.length()), axis: x_axis.normalize() };
    }
    let (min, max) = aabb_of(world, center, half);
    Obb { center: Vec2::new((min.x + max.x) * 0.5, (min.z + max.z) * 0.5), half: Vec2::new((max.x - min.x) * 0.5, (max.z - min.z) * 0.5), axis: Vec2::X }
}

fn material_color(o: &Object) -> Vec3 {
    let lin = match &o.kind {
        ObjectKind::Prop(p) => p.material.color.sample(0.0),
        ObjectKind::Stairs(s) => s.material.color.sample(0.0),
        ObjectKind::Humanoid(h) => h.material.color.sample(0.0),
        ObjectKind::Rat(r) => r.material.color.sample(0.0),
        _ => o.material.as_ref().map(|m| m.color.sample(0.0)).unwrap_or(Vec3::splat(0.7)),
    };
    let to_srgb = |c: f32| -> f32 {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    };
    Vec3::new(to_srgb(lin.x), to_srgb(lin.y), to_srgb(lin.z))
}

fn flatten_object(o: &Object, top_id: &str, parent: Mat4, collide: bool, ignores: &HashMap<String, Vec<String>>, out: &mut Vec<Item>) {
    let collide = collide && o.collide;
    let world = parent * trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
    let origin = world.transform_point3(Vec3::ZERO);
    let ignore = ignores.get(&o.id).or_else(|| ignores.get(top_id)).cloned().unwrap_or_default();
    let color = material_color(o);
    let mut push = |kind: ItemKind, min: Vec3, max: Vec3, volumes: Vec<(Vec3, Vec3)>, footprint: Option<Obb>| {
        out.push(Item {
            id: o.id.clone(),
            top_id: top_id.to_string(),
            kind,
            min,
            max,
            volumes: if collide { volumes } else { Vec::new() },
            footprint,
            origin,
            color,
            ignore: ignore.clone(),
            stairs: None,
            is_terrain: false,
        });
    };
    match &o.kind {
        ObjectKind::Group(children) => {
            for c in children {
                flatten_object(c, top_id, world, collide, ignores, out);
            }
        }
        ObjectKind::Prim(PrimKind::Box { size }) => {
            let (min, max) = aabb_of(world, Vec3::ZERO, *size * 0.5);
            push(ItemKind::Box, min, max, vec![(min, max)], Some(footprint_of(world, Vec3::ZERO, *size * 0.5)));
        }
        ObjectKind::Prim(PrimKind::Plane { size }) => {
            let half = Vec3::new(size.0 * 0.5, 0.0, size.1 * 0.5);
            let (min, max) = aabb_of(world, Vec3::ZERO, half);
            push(ItemKind::Plane, min, max, vec![], Some(footprint_of(world, Vec3::ZERO, half)));
        }
        ObjectKind::Prim(p) => {
            let (min, max) = aabb_of(world, Vec3::ZERO, p.half_extent());
            push(ItemKind::Other, min, max, vec![(min, max)], Some(footprint_of(world, Vec3::ZERO, p.half_extent())));
        }
        ObjectKind::Humanoid(h) => {
            let half = (h.height * 0.5).max(0.1);
            let (min, max) = aabb_of(world, Vec3::new(0.0, half, 0.0), Vec3::new(0.4, half, 0.4));
            push(ItemKind::Humanoid, min, max, vec![], None);
        }
        ObjectKind::Rat(_) => {
            let (min, max) = aabb_of(world, Vec3::new(0.0, 0.09, 0.0), Vec3::new(0.12, 0.09, 0.3));
            push(ItemKind::Rat, min, max, vec![], None);
        }
        // Ground, not a solid: a footprint like a floor plane, with no volume (players walk on it, `lint` reads its heights separately).
        ObjectKind::Terrain(td) => {
            let (lo, hi) = td.terrain.height_range();
            let size = td.terrain.size();
            let y0 = world.transform_point3(Vec3::ZERO).y;
            let half = Vec3::new(size.x * 0.5, ((hi - lo) * 0.5).max(0.01), size.y * 0.5);
            let (min, max) = aabb_of(world, Vec3::new(0.0, (lo + hi) * 0.5 - y0, 0.0), half);
            push(ItemKind::Plane, min, max, vec![], Some(footprint_of(world, Vec3::ZERO, Vec3::new(size.x * 0.5, 0.0, size.y * 0.5))));
            if let Some(last) = out.last_mut() {
                last.is_terrain = true;
            }
        }
        ObjectKind::Stairs(s) => {
            let half = Vec3::new(s.width * 0.5, s.rise * 0.5, s.run * 0.5);
            let center = Vec3::new(0.0, s.rise * 0.5, 0.0);
            let (min, max) = aabb_of(world, center, half);
            push(ItemKind::Stairs, min, max, vec![(min, max)], Some(footprint_of(world, center, half)));
            if let Some(last) = out.last_mut() {
                last.stairs = Some(StairsInfo { world, width: s.width, run: s.run, rise: s.rise, steps: s.steps });
            }
        }
        ObjectKind::Prop(p) => {
            let (lmin, lmax) = local_bounds(p.kind);
            let (min, max) = aabb_of(world, (lmin + lmax) * 0.5, (lmax - lmin) * 0.5);
            let volumes: Vec<(Vec3, Vec3)> = match collision(p.kind) {
                Collision::None => vec![],
                Collision::Box { .. } => {
                    let (bmin, bmax) = collision_box(p.kind).expect("box collision has a box");
                    vec![aabb_of(world, (bmin + bmax) * 0.5, (bmax - bmin) * 0.5)]
                }
                Collision::Union => prop_parts(p.kind).iter().map(|part| aabb_of(world * part.local_transform, Vec3::ZERO, part.shape.half_extent())).collect(),
            };
            let footprint = collision_box(p.kind)
                .map(|(bmin, bmax)| footprint_of(world, (bmin + bmax) * 0.5, (bmax - bmin) * 0.5))
                .or_else(|| Some(footprint_of(world, (lmin + lmax) * 0.5, (lmax - lmin) * 0.5)));
            push(ItemKind::Prop(p.kind), min, max, volumes, footprint);
        }
    }
}

/// Reads the scene at `path` as `MapWorld`, printing each parse error on failure — the shared
/// front door for every CLI tool.
pub fn load_or_report(path: &Path) -> Result<MapWorld, String> {
    load_or_report_phase(path, None)
}

/// [`load_or_report`] for a named `phases` state (`--phase`, `checks.*[].phase`).
pub fn load_or_report_phase(path: &Path, phase: Option<&str>) -> Result<MapWorld, String> {
    MapWorld::load_phase(path, phase).map_err(|errs| {
        let mut msg = format!("{}: scene is invalid:", path.display());
        for e in errs {
            msg.push_str(&format!("\n  error: {e}"));
        }
        msg
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_tracks_top_id_and_world_bounds() {
        let json = r##"{"camera":{},"objects":[
            {"id":"w","type":"wall","from":[0,0],"to":[4,0],"height":2.5,"thickness":0.2},
            {"id":"c","type":"prop","prop":"crate","position":[2,0,3]}
        ]}"##;
        let w = MapWorld::from_text(json, Path::new("t.json")).unwrap();
        let wall_items: Vec<&Item> = w.items.iter().filter(|i| i.top_id == "w").collect();
        assert!(!wall_items.is_empty());
        let (min, max) = wall_items.iter().fold((Vec3::splat(1e9), Vec3::splat(-1e9)), |(a, b), i| (a.min(i.min), b.max(i.max)));
        assert!((min.x - -0.1).abs() < 1e-3 && (max.x - 4.1).abs() < 1e-3, "wall spans its length plus the corner extension: {min:?} {max:?}");
        assert!((max.y - 2.5).abs() < 1e-3);
        let c = w.items.iter().find(|i| i.top_id == "c").unwrap();
        assert!((c.min.y - 0.0).abs() < 0.01, "crate rests on the floor");
    }
}
