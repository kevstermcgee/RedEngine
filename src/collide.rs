//! Static-world collision and ground queries: the renderer-free half of what used to live in `viewer.rs`.
//!
//! Everything a walking player, the tools (`lint`/`reach`/`walk`) and the headless server need to answer
//! "what blocks me here, and how high is the floor?": [`Collider2D`] footprints, [`GroundCandidates`]
//! (box tops + stair ramps), [`resolve_collision`], and [`Interactable`] boxes for aiming rays. Nothing in
//! this module touches a window, GPU or audio device (the `--no-default-features` server build depends on it).

use crate::geometry::trs;
use crate::player::PLAYER_RADIUS;
use crate::props::{collision, game_collision_boxes, prop_parts, Collision};
use crate::schema::{Object, ObjectKind, PrimKind, Scene, StairsDef};
use glam::{Mat4, Vec3};

/// A static (load-time) world-space axis-aligned bounding box, used for simple walk-around
/// wall/furniture collision. `min`/`max` are the XZ footprint (rotation ignored — conservative:
/// the AABB of the rotated box — fine for the axis-aligned rooms this viewer targets);
/// `min_y`/`max_y` are the world Y-range it actually occupies, kept (not resolved away at
/// collection time) so multi-floor maps can decide per-frame whether a given collider is at the
/// player's current floor — see [`colliders_on_floor`].
#[derive(Clone, Copy)]
pub struct Collider2D {
    pub min: glam::Vec2,
    pub max: glam::Vec2,
    pub min_y: f32,
    pub max_y: f32,
}

/// Vertical band a walking player's capsule occupies, *relative to their current foot height* —
/// a collider only blocks movement if its Y-range overlaps `foot_y + PLAYER_BAND_MIN_Y ..
/// foot_y + PLAYER_BAND_MAX_Y` (see [`colliders_on_floor`]). Absolute-`y=0`-relative would only
/// be correct on a single-floor map; keeping it relative to the player's actual current height
/// is what makes upstairs walls collide on a multi-story map without the ground floor's walls
/// leaking up through them (or vice versa).
///
/// The bottom of the band is the *step-up height* ([`GROUND_SNAP_EPS`]): anything whose top is
/// within that of the player's feet doesn't block — it is something to step *onto*, because
/// [`ground_height_at`] treats exactly those box tops as reachable ground. The two rules must
/// stay complementary (a collider either blocks or is standable, never neither): with the old
/// 0.05 m band, the edge of a floor slab 0.25 m above the top of a staircase blocked the player
/// while the ground snap refused to lift them onto it, so no staircase could ever be climbed
/// onto a floor.
const PLAYER_BAND_MIN_Y: f32 = GROUND_SNAP_EPS;
/// Top of a person's body band relative to their feet, m (see `player::BodySpec::band_top`).
pub const PLAYER_BAND_MAX_Y: f32 = 2.0;

/// Computes the world-space AABB (XZ footprint + Y-range) swept by a box of `half`-extents
/// centered on its own local origin under `transform`, and pushes it as a collider — shared by
/// a plain `box` primitive (`transform` = the object's own world transform) and a prop's
/// overall footprint (`transform` = the object's world transform, half-extent built from the
/// union of all its parts' local AABBs by the caller).
fn push_box_collider(transform: Mat4, half: Vec3, out: &mut Vec<Collider2D>) {
    let corners = [
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(-half.x, half.y, -half.z),
        Vec3::new(-half.x, half.y, half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(half.x, half.y, half.z),
    ];
    let mut min = glam::Vec2::splat(f32::INFINITY);
    let mut max = glam::Vec2::splat(f32::NEG_INFINITY);
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for c in corners {
        let wp = transform.transform_point3(c);
        min = min.min(glam::Vec2::new(wp.x, wp.z));
        max = max.max(glam::Vec2::new(wp.x, wp.z));
        min_y = min_y.min(wp.y);
        max_y = max_y.max(wp.y);
    }
    out.push(Collider2D { min, max, min_y, max_y });
}

/// Thickness of the solid side rails ("stringers") added along a staircase's long edges.
const STAIRS_STRINGER: f32 = 0.06;

/// A staircase is a solid block of steps: you climb it from its bottom end and nowhere else. The
/// walkable *ramp* (see [`StairsRamp`]) only says how high the ground is; without help, a player
/// at floor level could stroll sideways or from the tall end straight into the visual stair
/// mesh. So a stairs object also contributes:
/// - two full-height side rails along its long edges (keeps the player in the stair lane, and
///   out of the stair volume from the sides), and
/// - a barrier across the tall end, deliberately a little *shorter* than the top step (by more
///   than a player radius of ramp rise) so a player who has climbed to the top walks off it
///   onto the upper floor, while one at floor level is stopped.
/// Both are height-band colliders like any other, so they don't block a player standing on the
/// upper floor.
fn push_stairs_colliders(world: Mat4, s: &StairsDef, out: &mut Vec<Collider2D>) {
    let half_w = s.width * 0.5;
    let half_run = s.run * 0.5;
    for side in [-1.0f32, 1.0] {
        let center = Vec3::new(side * (half_w + STAIRS_STRINGER * 0.5), s.rise * 0.5, 0.0);
        push_box_collider(world * Mat4::from_translation(center), Vec3::new(STAIRS_STRINGER * 0.5, s.rise * 0.5, half_run), out);
    }
    let end_h = (s.rise * (1.0 - (PLAYER_RADIUS + 0.1) / s.run)).max(0.1);
    let center = Vec3::new(0.0, end_h * 0.5, half_run + STAIRS_STRINGER * 0.5);
    push_box_collider(world * Mat4::from_translation(center), Vec3::new(half_w + STAIRS_STRINGER, end_h * 0.5, STAIRS_STRINGER * 0.5), out);
}

/// Walks every `box` primitive and every `prop` in the scene (pose sampled at `t=0`, since
/// walls/furniture/props aren't expected to animate) and returns one collider per object —
/// [`colliders_on_floor`] filters these down to whichever ones are actually at the player's
/// current height before they're used for movement resolution. A prop gets a single collider
/// sized to its overall footprint (the union of all its parts), not one per part — a barrel's
/// thin rim bands or a crate's corner posts becoming their own tiny colliders would leave
/// gap-riddled, unintuitive collision instead of "you can't walk through this prop". `stairs`
/// contribute no collider at all here — you walk onto one, not around it (see
/// `ground_height_at`).
pub fn collect_box_colliders(scene: &Scene) -> Vec<Collider2D> {
    collect_box_colliders_except(scene, &std::collections::HashSet::new())
}

fn collect_object_colliders(objects: &[Object], parent: Mat4, out: &mut Vec<Collider2D>) {
    for o in objects {
        if !o.collide {
            continue;
        }
        let local = trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
        let world = parent * local;
        match &o.kind {
            ObjectKind::Prim(PrimKind::Box { size }) => push_box_collider(world, *size * 0.5, out),
            ObjectKind::Prim(_) => {}
            ObjectKind::Group(children) => collect_object_colliders(children, world, out),
            ObjectKind::Humanoid(_) | ObjectKind::Rat(_) => {}
            ObjectKind::Stairs(st) => push_stairs_colliders(world, st, out),
            ObjectKind::Prop(p) => {
                for (lmin, lmax) in game_collision_boxes(p.kind) {
                    push_box_collider(world * Mat4::from_translation((lmin + lmax) * 0.5), (lmax - lmin) * 0.5, out);
                }
            }
        }
    }
}

/// Static colliders grouped by top-level scene object. Ownership is retained so live game rules
/// can disable one object's collision without rebuilding its geometry.
pub fn collect_box_colliders_grouped_except(scene: &Scene, skip: &std::collections::HashSet<usize>) -> Vec<Vec<Collider2D>> {
    scene
        .objects
        .iter()
        .enumerate()
        .map(|(i, object)| {
            let mut out = Vec::new();
            if !skip.contains(&i) {
                collect_object_colliders(std::slice::from_ref(object), Mat4::IDENTITY, &mut out);
            }
            out
        })
        .collect()
}

/// [`collect_box_colliders`] leaving out the top-level objects in `skip` — loose physics props
/// (see `crate::physics`), which move and so are not static walls.
pub fn collect_box_colliders_except(scene: &Scene, skip: &std::collections::HashSet<usize>) -> Vec<Collider2D> {
    collect_box_colliders_grouped_except(scene, skip).into_iter().flatten().collect()
}

/// Filters a full collider list down to the ones that actually block movement *at the player's
/// current foot height* — see [`PLAYER_BAND_MIN_Y`]/[`PLAYER_BAND_MAX_Y`]'s doc comment for why
/// this has to be dynamic (relative to `foot_y`) rather than a fixed absolute band once a map
/// has more than one floor.
pub fn colliders_on_floor(colliders: &[Collider2D], foot_y: f32) -> Vec<Collider2D> {
    colliders_on_floor_h(colliders, foot_y, PLAYER_BAND_MAX_Y)
}

/// [`colliders_on_floor`] for a body whose top is `band_top` metres above its feet (a rat is far shorter
/// than a person, so a tabletop overhead does not block it).
pub fn colliders_on_floor_h(colliders: &[Collider2D], foot_y: f32, band_top: f32) -> Vec<Collider2D> {
    colliders.iter().copied().filter(|c| collider_blocks_at_h(c, foot_y, band_top)).collect()
}

/// Whether `c` blocks a player whose feet are at `foot_y` (its Y-range overlaps the player's
/// body band). The per-collider form of [`colliders_on_floor`], for callers that test one
/// position at a time and don't want to allocate a filtered list.
pub fn collider_blocks_at(c: &Collider2D, foot_y: f32) -> bool {
    collider_blocks_at_h(c, foot_y, PLAYER_BAND_MAX_Y)
}

/// [`collider_blocks_at`] for a body `band_top` metres tall.
pub fn collider_blocks_at_h(c: &Collider2D, foot_y: f32, band_top: f32) -> bool {
    c.max_y > foot_y + PLAYER_BAND_MIN_Y && c.min_y <= foot_y + band_top
}

/// A staircase's walkable ramp, world-space. `world_to_local` maps a world XZ (any Y — a pure
/// yaw rotation never mixes Y into X/Z, so the ramp's footprint test and height formula don't
/// need the query point's real world Y at all) back into the stairs' own frame, where the ramp
/// runs along local `+Z` from `-half_run` (height `base_y`) to `+half_run` (height
/// `base_y + rise`).
#[derive(Clone)]
struct StairsRamp {
    world_to_local: Mat4,
    half_width: f32,
    half_run: f32,
    base_y: f32,
    rise: f32,
    steps: u32,
}

/// Steps taller than this are walked as a plain linear ramp (a tread you could not step onto would otherwise be a wall).
const MAX_TREAD_HEIGHT: f32 = 0.30;
/// The last part of each tread over which the walking surface rises to the next tread's top.
const TREAD_BLEND: f32 = 0.4;

impl StairsRamp {
    /// The world height of the ramp at `xz`, or `None` outside its footprint.
    fn height_at(&self, xz: glam::Vec2) -> Option<f32> {
        let local = self.world_to_local.transform_point3(Vec3::new(xz.x, 0.0, xz.y));
        if local.x.abs() > self.half_width || local.z.abs() > self.half_run {
            return None;
        }
        let f = ((local.z + self.half_run) / (2.0 * self.half_run)).clamp(0.0, 1.0);
        Some(self.base_y + self.surface(f))
    }

    /// Height above the base at fraction `f` of the run. The rendered stairs are solid boxes whose tread `i` has its top at
    /// `(i + 1) * step_h`; a straight ramp through the corners runs *inside* every tread, so a low eye (Cheddar's is 15 cm up)
    /// sank into the stairs. This surface never dips below the tread under the feet: it holds the tread top for the first part of
    /// each tread and rises to the next tread's top over the last [`TREAD_BLEND`] of it (continuous, so no lurching).
    fn surface(&self, f: f32) -> f32 {
        let n = self.steps.max(1) as f32;
        let h = self.rise / n;
        if h > MAX_TREAD_HEIGHT || self.steps <= 1 {
            return self.rise * f;
        }
        let i = (f * n).floor().min(n - 1.0);
        if i >= n - 1.0 {
            return self.rise;
        }
        let t = f * n - i;
        let k = ((t - (1.0 - TREAD_BLEND)) / TREAD_BLEND).clamp(0.0, 1.0);
        let k = k * k * (3.0 - 2.0 * k);
        (i + 1.0 + k) * h
    }
}

/// Every standable surface in the scene, precomputed once at load (like [`Collider2D`]s):
/// every `box` primitive's and box-shaped `Prop` part's top face (reusing [`push_box_collider`]
/// — a `Collider2D`'s `max_y` doubles as "the height of this box's top"), plus every
/// [`crate::schema::StairsDef`]'s ramp. See [`ground_height_at`] for how these become an actual
/// walkable ground height.
#[derive(Default, Clone)]
pub struct GroundCandidates {
    box_tops: Vec<Collider2D>,
    stairs: Vec<StairsRamp>,
}

impl GroundCandidates {
    /// Every standable box top (`Collider2D::max_y` is the standing height), for analysis tools.
    pub fn box_tops(&self) -> &[Collider2D] {
        &self.box_tops
    }

    /// Height of the highest staircase ramp over `xz`, regardless of reachability, or `None`.
    pub fn stairs_height_at(&self, xz: glam::Vec2) -> Option<f32> {
        self.stairs.iter().filter_map(|st| st.height_at(xz)).fold(None, |a, h| Some(a.map_or(h, |m: f32| m.max(h))))
    }

    pub fn append(&mut self, other: &GroundCandidates) {
        self.box_tops.extend_from_slice(&other.box_tops);
        self.stairs.extend_from_slice(&other.stairs);
    }
}

pub fn collect_ground_candidates(scene: &Scene) -> GroundCandidates {
    collect_ground_candidates_except(scene, &std::collections::HashSet::new())
}

fn collect_object_ground(objects: &[Object], parent: Mat4, out: &mut GroundCandidates) {
    for o in objects {
        if !o.collide {
            continue;
        }
        let local = trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
        let world = parent * local;
        match &o.kind {
            ObjectKind::Prim(PrimKind::Box { size }) => push_box_collider(world, *size * 0.5, &mut out.box_tops),
            ObjectKind::Prim(_) => {}
            ObjectKind::Group(children) => collect_object_ground(children, world, out),
            ObjectKind::Humanoid(_) | ObjectKind::Rat(_) => {}
            ObjectKind::Prop(p) => {
                if collision(p.kind) == Collision::Union {
                    for part in prop_parts(p.kind) {
                        if let PrimKind::Box { size } = part.shape {
                            push_box_collider(world * part.local_transform, size * 0.5, &mut out.box_tops);
                        }
                    }
                }
            }
            ObjectKind::Stairs(s) => out.stairs.push(StairsRamp {
                world_to_local: world.inverse(),
                half_width: s.width * 0.5,
                half_run: s.run * 0.5,
                base_y: world.transform_point3(Vec3::ZERO).y,
                rise: s.rise,
                steps: s.steps,
            }),
        }
    }
}

/// Standable surfaces grouped by top-level scene object, parallel to the grouped static colliders.
pub fn collect_ground_candidates_grouped_except(scene: &Scene, skip: &std::collections::HashSet<usize>) -> Vec<GroundCandidates> {
    scene
        .objects
        .iter()
        .enumerate()
        .map(|(i, object)| {
            let mut out = GroundCandidates::default();
            if !skip.contains(&i) {
                collect_object_ground(std::slice::from_ref(object), Mat4::IDENTITY, &mut out);
            }
            out
        })
        .collect()
}

/// [`collect_ground_candidates`] leaving out the top-level objects in `skip` (loose physics props).
pub fn collect_ground_candidates_except(scene: &Scene, skip: &std::collections::HashSet<usize>) -> GroundCandidates {
    let mut out = GroundCandidates::default();
    for group in collect_ground_candidates_grouped_except(scene, skip) {
        out.append(&group);
    }
    out
}

/// A small tolerance, in world units, for how far above the player's *current* foot height a
/// candidate surface may be and still count as "reachable" — comfortably larger than the
/// per-tick height gain from walking up a normal-slope staircase (a few centimeters at typical
/// walk speed and the 60Hz fixed timestep), but far smaller than a floor-to-floor gap (a few
/// meters). This is the whole mechanism that keeps a flat second-floor deck from being walkable
/// from underneath: nothing marks it "upstairs" vs. "downstairs", it's just another box, and
/// it's simply too far above the player's current height to be a candidate until they've
/// climbed near it (via stairs, whose ramp height rises in exactly such small increments).
const GROUND_SNAP_EPS: f32 = 0.35;

/// The height of the highest walkable surface reachable from `current_foot_y` at `xz` — `0.0`
/// (the base ground floor) is always a valid fallback; see [`GROUND_SNAP_EPS`] for the
/// reachability rule layered on top of that for every other candidate.
pub fn ground_height_at(candidates: &GroundCandidates, xz: glam::Vec2, current_foot_y: f32) -> f32 {
    let limit = current_foot_y + GROUND_SNAP_EPS;
    let mut best = 0.0f32;
    for b in &candidates.box_tops {
        if b.max_y <= limit && xz.x >= b.min.x && xz.x <= b.max.x && xz.y >= b.min.y && xz.y <= b.max.y {
            best = best.max(b.max_y);
        }
    }
    for s in &candidates.stairs {
        if let Some(h) = s.height_at(xz) {
            if h <= limit {
                best = best.max(h);
            }
        }
    }
    best
}

/// Pushes a `radius`-sized circle at `pos` out of every collider it overlaps. Call once per
/// movement axis (resolve X, then resolve Z) for stable sliding-along-walls behavior.
pub fn resolve_collision(pos: glam::Vec2, radius: f32, colliders: &[Collider2D]) -> glam::Vec2 {
    let mut p = pos;
    for c in colliders {
        let closest = p.clamp(c.min, c.max);
        let diff = p - closest;
        let dist_sq = diff.length_squared();
        if dist_sq < radius * radius {
            if dist_sq > 1e-8 {
                let dist = dist_sq.sqrt();
                p += diff * ((radius - dist) / dist);
            } else {
                // Center is exactly on the boundary/inside; push out along the shallowest axis.
                let push_x = (c.max.x - p.x).min(p.x - c.min.x);
                let push_z = (c.max.y - p.y).min(p.y - c.min.y);
                if push_x < push_z {
                    p.x += if p.x - c.min.x < c.max.x - p.x { -radius } else { radius };
                } else {
                    p.y += if p.y - c.min.y < c.max.y - p.y { -radius } else { radius };
                }
            }
        }
    }
    p
}

/// A whole top-level scene object, reduced to one world-space AABB for "what am I looking at"
/// raycasting. Deliberately coarse (one box per top-level `Object`, covering the full subtree
/// for a `group` or the whole rig for a `humanoid`) rather than per-leaf-mesh — "look at the
/// table and press E" should mean the whole table, not one leg. An AABB rather than a bounding
/// sphere specifically because a sphere badly over-approximates a flat or elongated object (a
/// floor plane's bounding sphere, built from its diagonal, would reach room-wide in every
/// direction — nowhere close to the thin slab it's actually meant to represent).
pub struct Interactable {
    pub object_index: usize,
    pub id: String,
    pub min: Vec3,
    pub max: Vec3,
}

fn prim_half_extent(p: &PrimKind) -> Vec3 {
    p.half_extent()
}

fn accumulate_world_bounds(o: &Object, parent: Mat4, min: &mut Vec3, max: &mut Vec3) {
    let local = trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
    let world = parent * local;
    let mut expand = |transform: Mat4, center_local: Vec3, half: Vec3| {
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let corner = center_local + Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                    let wp = transform.transform_point3(corner);
                    *min = min.min(wp);
                    *max = max.max(wp);
                }
            }
        }
    };
    match &o.kind {
        ObjectKind::Prim(p) => expand(world, Vec3::ZERO, prim_half_extent(p)),
        ObjectKind::Group(children) => {
            for c in children {
                accumulate_world_bounds(c, world, min, max);
            }
        }
        ObjectKind::Humanoid(h) => {
            let half = (h.height * 0.5).max(0.1);
            expand(world, Vec3::new(0.0, half, 0.0), Vec3::splat(half));
        }
        ObjectKind::Rat(_) => expand(world, Vec3::new(0.0, 0.09, 0.0), Vec3::new(0.12, 0.09, 0.3)),
        // Tighter than one coarse box: union of each part's own AABB, transformed through both
        // the object's world transform and that part's own local placement.
        ObjectKind::Prop(p) => {
            for part in prop_parts(p.kind) {
                expand(world * part.local_transform, Vec3::ZERO, prim_half_extent(&part.shape));
            }
        }
        // One coarse box covering the whole ramp footprint at full height — not used for
        // movement (stairs aren't an XZ collider, see `collect_box_colliders`), only so the
        // crosshair/melee raycast can target a staircase like any other object.
        ObjectKind::Stairs(s) => {
            expand(world, Vec3::new(0.0, s.rise * 0.5, 0.0), Vec3::new(s.width * 0.5, s.rise * 0.5, s.run * 0.5));
        }
    }
}

/// One world-space AABB per top-level scene object (pose sampled at `t=0`, same static-pose
/// assumption as [`collect_box_colliders`]), for [`raycast_nearest`].
pub fn collect_interactables(scene: &Scene) -> Vec<Interactable> {
    interactables_of(&scene.objects)
}

/// [`collect_interactables`] for a bare object list (what the parser has before the `Scene` exists).
pub fn interactables_of(objects: &[Object]) -> Vec<Interactable> {
    let mut out = Vec::with_capacity(objects.len());
    for (object_index, o) in objects.iter().enumerate() {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        accumulate_world_bounds(o, Mat4::IDENTITY, &mut min, &mut max);
        if min.x.is_finite() {
            out.push(Interactable { object_index, id: o.id.clone(), min, max });
        }
    }
    out
}

/// Nearest [`Interactable`] a ray hits within `max_dist`, or `None` — a standard ray-vs-AABB
/// slab test. `dir` need not be normalized. Used to find what the player is aiming at
/// (crosshair = screen center = ray from the camera along its look direction).
pub fn raycast_nearest(origin: Vec3, dir: Vec3, max_dist: f32, items: &[Interactable]) -> Option<usize> {
    let dir = dir.normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    let inv_dir = Vec3::ONE / dir;
    let mut best: Option<(usize, f32)> = None;
    for (i, it) in items.iter().enumerate() {
        let t1 = (it.min - origin) * inv_dir;
        let t2 = (it.max - origin) * inv_dir;
        let t_enter = t1.min(t2).max_element().max(0.0);
        let t_exit = t1.max(t2).min_element();
        if t_enter <= t_exit && t_enter <= max_dist && best.is_none_or(|(_, bt)| t_enter < bt) {
            best = Some((i, t_enter));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod ground_tests {
    use super::*;

    // Mirrors the exact per-tick clamp `App::fixed_step_physics` uses (`if foot_y <= ground_now
    // { foot_y = ground_now }`), without gravity's small downward nudge — irrelevant here since
    // it only ever makes `foot_y` a hair lower before the same clamp catches it right back.
    fn walk(candidates: &GroundCandidates, xz_path: impl Iterator<Item = glam::Vec2>) -> f32 {
        let mut foot_y = 0.0f32;
        for xz in xz_path {
            let ground = ground_height_at(candidates, xz, foot_y);
            if foot_y <= ground {
                foot_y = ground;
            }
        }
        foot_y
    }

    fn straight_line(from: glam::Vec2, to: glam::Vec2, step: f32) -> impl Iterator<Item = glam::Vec2> {
        let dist = (to - from).length();
        let steps = (dist / step).ceil() as u32;
        (1..=steps).map(move |i| from.lerp(to, i as f32 / steps as f32))
    }

    /// A straight run of a real `WALK_SPEED`-at-`FIXED_DT` step, walked bottom-to-top, should
    /// climb the ramp smoothly all the way to (approximately) full rise — this is the whole
    /// point of `GROUND_SNAP_EPS`: reachability never lags behind by more than one tick's worth
    /// of height gain at a normal walking pace.
    #[test]
    fn stairs_ramp_climbs_smoothly_bottom_to_top() {
        let ramp = StairsRamp { world_to_local: Mat4::IDENTITY, half_width: 1.0, half_run: 2.0, base_y: 0.0, rise: 3.0, steps: 16 };
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp] };
        // ~WALK_SPEED (3.2 m/s) at FIXED_DT (1/60s) — the real per-tick horizontal step size.
        let foot_y = walk(&candidates, straight_line(glam::Vec2::new(0.0, -2.0), glam::Vec2::new(0.0, 2.0), 3.2 / 60.0));
        assert!(foot_y > 2.9, "expected to reach near the top of a rise-3.0 ramp, got {foot_y}");
    }

    /// The walking surface never dips below the rendered tread under the feet (so a rat's 15 cm eye stays out of the stairs), never
    /// rises more than one step above it, and is continuous from tread to tread.
    #[test]
    fn stairs_surface_follows_the_treads_without_sinking_into_them() {
        let ramp = StairsRamp { world_to_local: Mat4::IDENTITY, half_width: 1.0, half_run: 2.0, base_y: 0.0, rise: 3.0, steps: 16 };
        let h = 3.0 / 16.0;
        let mut prev = ramp.surface(0.0);
        for k in 0..=1600 {
            let f = k as f32 / 1600.0;
            let s = ramp.surface(f);
            let tread = (((f * 16.0).floor().min(15.0)) + 1.0) * h;
            assert!(s >= tread - 1e-4, "f={f}: surface {s} is inside the tread (top {tread})");
            assert!(s <= tread + h + 1e-4, "f={f}: surface {s} floats more than a step above the tread");
            assert!((s - prev).abs() < 0.03, "f={f}: jump {} between samples", (s - prev).abs());
            prev = s;
        }
        assert!((ramp.surface(1.0) - 3.0).abs() < 1e-4, "it ends at the full rise");
        // Tall steps (few, big ones) keep the plain ramp so they stay climbable.
        let tall = StairsRamp { steps: 4, ..ramp };
        assert!((tall.surface(0.5) - 1.5).abs() < 1e-4);
    }

    /// The same ramp walked in reverse (top to bottom) should descend smoothly back to ~0,
    /// not get stuck partway — a player should be able to walk back down a staircase.
    #[test]
    fn stairs_ramp_descends_smoothly_top_to_bottom() {
        let ramp = StairsRamp { world_to_local: Mat4::IDENTITY, half_width: 1.0, half_run: 2.0, base_y: 0.0, rise: 3.0, steps: 16 };
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp] };
        let mut foot_y = 3.0; // start already on top, as if having just climbed up
        for xz in straight_line(glam::Vec2::new(0.0, 2.0), glam::Vec2::new(0.0, -2.0), 3.2 / 60.0) {
            let ground = ground_height_at(&candidates, xz, foot_y);
            // Gravity pulls it down between ticks in the real game; here just track the ground
            // height directly, since a descending ramp is always "reachable" from above (you
            // fall onto it, you don't need to climb up to it).
            foot_y = ground;
        }
        assert!(foot_y < 0.2, "expected to have descended to the first tread (0.1875), got {foot_y}");
    }

    /// Approaching a ramp from its *tall* end while standing at ground level must not teleport
    /// the player straight up to full rise — only once they're close enough (within
    /// `GROUND_SNAP_EPS`) should the ramp's height become a valid candidate at all.
    #[test]
    fn stairs_ramp_tall_end_is_unreachable_from_ground_level() {
        let ramp = StairsRamp { world_to_local: Mat4::IDENTITY, half_width: 1.0, half_run: 2.0, base_y: 0.0, rise: 3.0, steps: 16 };
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp] };
        // Standing right at the tall end (local z = +2, height = 3.0) with feet still at 0.
        let ground = ground_height_at(&candidates, glam::Vec2::new(0.0, 2.0), 0.0);
        assert_eq!(ground, 0.0, "the tall end of a ramp must be rejected as unreachable from ground level");
    }

    /// A flat elevated surface (e.g. a second-floor deck) is unreachable from ground level, but
    /// becomes a valid candidate once the player is already close to its height — this is the
    /// whole mechanism that keeps a deck from being "walkable" from underneath it.
    #[test]
    fn elevated_box_top_is_gated_by_current_height() {
        let deck = Collider2D { min: glam::Vec2::new(-5.0, -5.0), max: glam::Vec2::new(5.0, 5.0), min_y: 2.8, max_y: 3.0 };
        let candidates = GroundCandidates { box_tops: vec![deck], stairs: vec![] };
        let xz = glam::Vec2::new(0.0, 0.0);
        assert_eq!(ground_height_at(&candidates, xz, 0.0), 0.0, "deck must be unreachable from ground level");
        assert_eq!(ground_height_at(&candidates, xz, 2.9), 3.0, "deck must become reachable once already close to its height");
    }
}
