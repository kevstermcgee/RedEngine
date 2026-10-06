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
            // Terrain is ground, not a wall: it contributes to `ground_height_at`, never a 2-D collider.
            ObjectKind::Terrain(_) => {}
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
    let wrap = scene.player.expanse.wrap;
    scene
        .objects
        .iter()
        .enumerate()
        .map(|(i, object)| {
            let mut out = Vec::new();
            if !skip.contains(&i) {
                collect_object_colliders(std::slice::from_ref(object), Mat4::IDENTITY, &mut out);
                if let Some(w) = wrap {
                    add_collider_images(&mut out, w);
                }
            }
            out
        })
        .collect()
}

/// How close to the seam of a looping world a collider or a ground surface is copied to the other side of it, m: a body (radius 0.35) and
/// everything near it that can matter.
const SEAM_MARGIN: f32 = 4.0;

fn shifted(c: &Collider2D, wrap: crate::expanse::Wrap, by: f32) -> Collider2D {
    let d = wrap.axis.offset(by);
    Collider2D { min: c.min + d, max: c.max + d, min_y: c.min_y, max_y: c.max_y }
}

/// On a looping world, near the seam every collider also exists one period away, so a player standing just past the edge collides with
/// what is just past the *other* edge. Colliders that straddle or touch the seam get both copies.
fn add_collider_images(out: &mut Vec<Collider2D>, wrap: crate::expanse::Wrap) {
    let period = wrap.period();
    let n = out.len();
    for i in 0..n {
        let c = out[i];
        let (lo, hi) = (wrap.axis.of(c.min), wrap.axis.of(c.max));
        if hi > wrap.max - SEAM_MARGIN {
            out.push(shifted(&c, wrap, -period));
        }
        if lo < wrap.min + SEAM_MARGIN {
            out.push(shifted(&c, wrap, period));
        }
    }
}

/// The ground twin of [`add_collider_images`]: box tops and stairs near the seam exist on both sides of it.
fn add_ground_images(g: &mut GroundCandidates, wrap: crate::expanse::Wrap) {
    let period = wrap.period();
    add_collider_images(&mut g.box_tops, wrap);
    let n = g.stairs.len();
    for i in 0..n {
        let s = g.stairs[i].clone();
        // The ramp's footprint in the looping coordinate, from its own frame: the four extents around its origin.
        let centre = s.world_to_local.inverse().transform_point3(Vec3::ZERO);
        let reach = s.half_width.max(s.half_run) * 1.5 + 0.5;
        let c = wrap.axis.of(glam::Vec2::new(centre.x, centre.z));
        for by in [-period, period] {
            let near_edge = if by < 0.0 { c + reach > wrap.max - SEAM_MARGIN } else { c - reach < wrap.min + SEAM_MARGIN };
            if near_edge {
                let offset = match wrap.axis {
                    crate::expanse::Axis::X => Vec3::new(by, 0.0, 0.0),
                    crate::expanse::Axis::Z => Vec3::new(0.0, 0.0, by),
                };
                let mut copy = s.clone();
                copy.world_to_local *= Mat4::from_translation(-offset);
                g.stairs.push(copy);
            }
        }
    }
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

/// A uniform grid over the footprints of the box tops: each bin lists the tops overlapping it. Map analysis asks for the ground height millions of times (every cell
/// of the reachability flood fill, every neighbour) and the player step asks every tick; scanning every top made a furnished map (about 2 800 tops) spend 92% of
/// its `reach` time there (measured, 7.45 of 8.06 s).
///
/// The answer is unchanged by construction: the ground height is the highest top that contains the point, an order-independent maximum, and the bin a point falls
/// in lists every top whose footprint contains it (bin numbers are a monotonic function of the coordinate, used identically to register and to look up).
#[derive(Clone)]
struct BoxIndex {
    min: glam::Vec2,
    inv_bin: f32,
    nx: usize,
    nz: usize,
    bins: Vec<Vec<u32>>,
    /// How many tops were indexed (a debug check that the set did not change underneath).
    count: usize,
}

impl BoxIndex {
    /// Bin edge in metres: about the size of furniture, so a bin holds a handful of tops.
    const BIN: f32 = 2.0;
    /// Bins are capped so a huge or looping map cannot ask for gigabytes of index.
    const MAX_BINS: f32 = 262_144.0;

    fn build(tops: &[Collider2D]) -> BoxIndex {
        if tops.is_empty() {
            return BoxIndex { min: glam::Vec2::ZERO, inv_bin: 1.0, nx: 0, nz: 0, bins: Vec::new(), count: 0 };
        }
        let (mut lo, mut hi) = (glam::Vec2::splat(f32::INFINITY), glam::Vec2::splat(f32::NEG_INFINITY));
        for t in tops {
            lo = lo.min(t.min);
            hi = hi.max(t.max);
        }
        let (w, h) = ((hi.x - lo.x).max(0.0), (hi.y - lo.y).max(0.0));
        let bin = Self::BIN.max((w * h / Self::MAX_BINS).sqrt());
        let inv_bin = 1.0 / bin;
        let nx = ((w * inv_bin).floor() as usize + 1).max(1);
        let nz = ((h * inv_bin).floor() as usize + 1).max(1);
        let mut index = BoxIndex { min: lo, inv_bin, nx, nz, bins: vec![Vec::new(); nx * nz], count: tops.len() };
        for (i, t) in tops.iter().enumerate() {
            let (x0, z0) = index.cell_of(t.min);
            let (x1, z1) = index.cell_of(t.max);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    index.bins[z * nx + x].push(i as u32);
                }
            }
        }
        index
    }

    /// The bin a point falls in, clamped into the grid (the same function registers a top and looks a point up).
    fn cell_of(&self, p: glam::Vec2) -> (usize, usize) {
        let x = ((p.x - self.min.x) * self.inv_bin).floor().clamp(0.0, (self.nx - 1) as f32) as usize;
        let z = ((p.y - self.min.y) * self.inv_bin).floor().clamp(0.0, (self.nz - 1) as f32) as usize;
        (x, z)
    }

    /// Every top registered in the bin of `p`; empty when `p` is outside the indexed area (no top can contain it).
    fn at(&self, p: glam::Vec2) -> &[u32] {
        if self.bins.is_empty() {
            return &[];
        }
        let (fx, fz) = ((p.x - self.min.x) * self.inv_bin, (p.y - self.min.y) * self.inv_bin);
        // Outside the bounding box of every top: nothing contains the point. (A NaN falls through to the clamp and is checked against the tops.)
        if fx < 0.0 || fz < 0.0 || fx.floor() > (self.nx - 1) as f32 || fz.floor() > (self.nz - 1) as f32 {
            return &[];
        }
        let (x, z) = self.cell_of(p);
        &self.bins[z * self.nx + x]
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
    /// Uniform-grid index over `box_tops` (see [`BoxIndex`]), built on the first ground query and dropped whenever the set changes, so a lookup
    /// reads the few tops near a point instead of every box top in the map. `box_tops` stays the source of truth.
    box_index: std::sync::OnceLock<BoxIndex>,
    stairs: Vec<StairsRamp>,
    /// Heightfield terrains: where one exists it is the ground (no invisible floor at `y = 0` beneath it).
    terrains: Vec<std::sync::Arc<crate::terrain::Terrain>>,
    /// The scene's looping axis: ground queries are brought into the period first.
    wrap: Option<crate::expanse::Wrap>,
    /// An endless generated world (`procgen` block): it is the ground everywhere, and its tree trunks block.
    procgen: Option<std::sync::Arc<crate::procgen::ProcgenGround>>,
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
        self.box_index = std::sync::OnceLock::new();
        self.stairs.extend_from_slice(&other.stairs);
        self.terrains.extend(other.terrains.iter().cloned());
        self.wrap = self.wrap.or(other.wrap);
        if self.procgen.is_none() {
            self.procgen = other.procgen.clone();
        }
    }

    /// The generated world under this ground, if the scene has a `procgen` block.
    pub fn procgen(&self) -> Option<&crate::procgen::ProcgenGround> {
        self.procgen.as_deref()
    }

    /// Whether the ground is anything but the flat `y = 0` floor: a heightfield `terrain` or a generated `procgen` world. Loose props must rest on *that* surface, not on
    /// a flat floor the player never stands on (see `physics::PropWorld`).
    pub fn has_natural_ground(&self) -> bool {
        !self.terrains.is_empty() || self.procgen.is_some()
    }

    /// The indices (into `box_tops`) of every box top whose footprint may contain `xz`: a superset of the tops that do, never missing one.
    fn box_candidates(&self, xz: glam::Vec2) -> &[u32] {
        let index = self.box_index.get_or_init(|| BoxIndex::build(&self.box_tops));
        debug_assert_eq!(index.count, self.box_tops.len(), "box_tops changed after the ground index was built");
        index.at(xz)
    }

    /// Height of the terrain under `xz`, if any terrain covers it (the highest, if several overlap).
    pub fn terrain_height_at(&self, xz: glam::Vec2) -> Option<f32> {
        let xz = self.wrap.map_or(xz, |w| w.wrap_pos(xz));
        let generated = self.procgen.as_ref().map(|p| p.height(xz));
        self.terrains.iter().filter_map(|t| t.height_at(xz.x, xz.y)).chain(generated).fold(None, |a, h| Some(a.map_or(h, |m: f32| m.max(h))))
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
            ObjectKind::Terrain(td) => out.terrains.push(td.terrain.clone()),
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
            let mut out = GroundCandidates { wrap: scene.player.expanse.wrap, ..Default::default() };
            if !skip.contains(&i) {
                collect_object_ground(std::slice::from_ref(object), Mat4::IDENTITY, &mut out);
                if let Some(w) = scene.player.expanse.wrap {
                    add_ground_images(&mut out, w);
                }
            }
            out
        })
        .collect()
}

/// [`collect_ground_candidates`] leaving out the top-level objects in `skip` (loose physics props).
pub fn collect_ground_candidates_except(scene: &Scene, skip: &std::collections::HashSet<usize>) -> GroundCandidates {
    ground_from_groups(scene, &collect_ground_candidates_grouped_except(scene, skip))
}

/// The part of a scene's ground that belongs to the *scene* and to no object: its looping axis and its generated world (`procgen`). A scene with no objects at all (an endless
/// meadow) is all of this, so a ground built from per-object groups alone is a flat floor at `y = 0` under hills and trees that do not block. Everything that assembles a ground from
/// groups (the single-player client, the match behind a server or a `LocalSession`) starts from this, and keeps it to start from again when it rebuilds.
pub fn scene_ground(scene: &Scene) -> GroundCandidates {
    GroundCandidates {
        wrap: scene.player.expanse.wrap,
        procgen: scene.procgen.clone().map(|cfg| std::sync::Arc::new(crate::procgen::ProcgenGround::new(cfg))),
        ..Default::default()
    }
}

/// The ground under a scene made of per-object `groups` (the ones whose object is switched on) on top of [`scene_ground`].
pub fn ground_from_groups<'a>(scene: &Scene, groups: impl IntoIterator<Item = &'a GroundCandidates>) -> GroundCandidates {
    let mut out = scene_ground(scene);
    for group in groups {
        out.append(group);
    }
    out
}

/// **The one answer to "given this scene and this collision state, what physical world exists?"** Every consumer of the static world asks this type, so none of them can drift:
/// the single-player client, the server and any `LocalSession` (`MatchSim`), the online client's prediction and the bots (`ClientWorld`), and every analysis tool (`MapWorld`:
/// `lint`, `reach`, `walk`, `plan`, `verify`, scripted checks, phase analysis).
///
/// It is the scene's own ground ([`scene_ground`]: the generated `procgen` world and the loop) plus, for every top-level object whose collision is **on**, that object's
/// colliders and standable surfaces. Switching an object's collision off (a rule's `collision_off`, a phase that opens a gate) removes exactly that object's part and nothing
/// else: hills, trees and every other object stay. Objects in `loose` are physics props, simulated elsewhere, and are never part of the static world.
#[derive(Clone)]
pub struct PhysicalWorld {
    scene_ground: GroundCandidates,
    ids: Vec<String>,
    collider_groups: Vec<Vec<Collider2D>>,
    ground_groups: Vec<GroundCandidates>,
    off: Vec<bool>,
    colliders: Vec<Collider2D>,
    ground: GroundCandidates,
}

impl PhysicalWorld {
    /// The world with every object's collision on (`loose` objects left out).
    pub fn new(scene: &Scene, loose: &std::collections::HashSet<usize>) -> PhysicalWorld {
        let collider_groups = collect_box_colliders_grouped_except(scene, loose);
        let ground_groups = collect_ground_candidates_grouped_except(scene, loose);
        let mut w = PhysicalWorld {
            scene_ground: scene_ground(scene),
            ids: scene.objects.iter().map(|o| o.id.clone()).collect(),
            off: vec![false; collider_groups.len()],
            collider_groups,
            ground_groups,
            colliders: Vec::new(),
            ground: GroundCandidates::default(),
        };
        w.assemble();
        w
    }

    /// The world with the objects named in `disabled` (their collision off); names that match no top-level object are ignored.
    pub fn of<S: AsRef<str>>(scene: &Scene, loose: &std::collections::HashSet<usize>, disabled: impl IntoIterator<Item = S>) -> PhysicalWorld {
        let mut w = PhysicalWorld::new(scene, loose);
        w.set_collision_disabled(disabled);
        w
    }

    /// Makes `disabled` (object ids) the set whose collision is off. Returns whether the world changed (it is rebuilt only then).
    pub fn set_collision_disabled<S: AsRef<str>>(&mut self, disabled: impl IntoIterator<Item = S>) -> bool {
        let names: std::collections::HashSet<String> = disabled.into_iter().map(|s| s.as_ref().to_string()).collect();
        let off: Vec<bool> = self.ids.iter().map(|id| names.contains(id)).collect();
        if off == self.off {
            return false;
        }
        self.off = off;
        self.assemble();
        true
    }

    /// Every blocking footprint of the objects whose collision is on. The generated world's trees are not here: they come from [`GroundCandidates::procgen`] by position.
    pub fn colliders(&self) -> &[Collider2D] {
        &self.colliders
    }

    /// The ground: the scene's generated world and loop, and the standable surfaces of the objects whose collision is on.
    pub fn ground(&self) -> &GroundCandidates {
        &self.ground
    }

    /// The top-level object ids whose collision is currently off.
    pub fn disabled(&self) -> Vec<&str> {
        self.ids.iter().zip(&self.off).filter(|(_, off)| **off).map(|(id, _)| id.as_str()).collect()
    }

    fn assemble(&mut self) {
        self.colliders = self.collider_groups.iter().zip(&self.off).filter(|(_, off)| !**off).flat_map(|(g, _)| g.iter().copied()).collect();
        let mut ground = self.scene_ground.clone();
        for (g, _) in self.ground_groups.iter().zip(&self.off).filter(|(_, off)| !**off) {
            ground.append(g);
        }
        self.ground = ground;
    }
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
    // Terrain *is* the ground where it exists: it replaces the `y = 0` fallback (a seabed below sea level is walkable), and boxes and
    // stairs may still stand on top of it.
    let xz = candidates.wrap.map_or(xz, |w| w.wrap_pos(xz));
    let mut best = candidates.terrain_height_at(xz).unwrap_or(0.0);
    for &i in candidates.box_candidates(xz) {
        let b = &candidates.box_tops[i as usize];
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
        ObjectKind::Terrain(td) => {
            let (lo, hi) = td.terrain.height_range();
            let size = td.terrain.size();
            let y0 = o.position.sample(0.0).y;
            expand(world, Vec3::new(0.0, (lo + hi) * 0.5 - y0, 0.0), Vec3::new(size.x * 0.5, ((hi - lo) * 0.5).max(0.01), size.y * 0.5));
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
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp], ..Default::default() };
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
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp], ..Default::default() };
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
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp], ..Default::default() };
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
        let candidates = GroundCandidates { box_tops: vec![deck], stairs: vec![], ..Default::default() };
        let xz = glam::Vec2::new(0.0, 0.0);
        assert_eq!(ground_height_at(&candidates, xz, 0.0), 0.0, "deck must be unreachable from ground level");
        assert_eq!(ground_height_at(&candidates, xz, 2.9), 3.0, "deck must become reachable once already close to its height");
    }

    /// The ground height the way it was computed before the index: every top, in order.
    fn ground_height_naive(c: &GroundCandidates, xz: glam::Vec2, current_foot_y: f32) -> f32 {
        let limit = current_foot_y + GROUND_SNAP_EPS;
        let mut best = c.terrain_height_at(xz).unwrap_or(0.0);
        for b in &c.box_tops {
            if b.max_y <= limit && xz.x >= b.min.x && xz.x <= b.max.x && xz.y >= b.min.y && xz.y <= b.max.y {
                best = best.max(b.max_y);
            }
        }
        for s in &c.stairs {
            if let Some(h) = s.height_at(xz) {
                if h <= limit {
                    best = best.max(h);
                }
            }
        }
        best
    }

    /// A tiny deterministic generator (the tests must not depend on a rand crate or on the clock).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
        }
        fn range(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (hi - lo) * self.next()
        }
    }

    fn top(min: (f32, f32), max: (f32, f32), y: f32) -> Collider2D {
        Collider2D { min: glam::Vec2::new(min.0, min.1), max: glam::Vec2::new(max.0, max.1), min_y: y - 0.1, max_y: y }
    }

    #[test]
    fn the_indexed_ground_height_equals_the_full_scan_everywhere() {
        let mut rng = Lcg(0x5eed);
        for trial in 0..40 {
            let mut tops = Vec::new();
            // A big floor, a scatter of furniture-sized tops at assorted heights, and a few long thin ones (benches, walls' caps).
            tops.push(top((-20.0, -15.0), (20.0, 15.0), 0.0));
            for _ in 0..(20 + trial * 15) {
                let (x, z) = (rng.range(-19.0, 19.0), rng.range(-14.0, 14.0));
                let (w, d) = (rng.range(0.2, 2.5), rng.range(0.2, 2.5));
                tops.push(top((x, z), (x + w, z + d), [0.0, 0.3, 0.5, 0.9, 2.8][(rng.next() * 5.0) as usize % 5]));
            }
            tops.push(top((-18.0, 3.0), (18.0, 3.1), 1.0));
            let c = GroundCandidates { box_tops: tops.clone(), ..Default::default() };
            let mut points: Vec<glam::Vec2> = (0..400).map(|_| glam::Vec2::new(rng.range(-25.0, 25.0), rng.range(-20.0, 20.0))).collect();
            // Every edge and corner of every top, exactly and a hair either side: the places an index can get wrong.
            for t in &tops {
                for (x, z) in [(t.min.x, t.min.y), (t.max.x, t.max.y), (t.min.x, t.max.y), (t.max.x, t.min.y)] {
                    for (dx, dz) in [(0.0, 0.0), (1e-4, 0.0), (-1e-4, 0.0), (0.0, 1e-4), (0.0, -1e-4)] {
                        points.push(glam::Vec2::new(x + dx, z + dz));
                    }
                }
            }
            for p in &points {
                for foot in [0.0, 0.4, 1.0, 3.0] {
                    assert_eq!(ground_height_at(&c, *p, foot), ground_height_naive(&c, *p, foot), "trial {trial} point {p:?} foot {foot}");
                }
            }
        }
    }

    #[test]
    fn the_ground_index_handles_no_tops_outside_points_and_a_changed_set() {
        let empty = GroundCandidates::default();
        assert_eq!(ground_height_at(&empty, glam::Vec2::new(3.0, 4.0), 0.0), 0.0);
        let mut c = GroundCandidates { box_tops: vec![top((0.0, 0.0), (2.0, 2.0), 0.3)], ..Default::default() };
        assert_eq!(ground_height_at(&c, glam::Vec2::new(1.0, 1.0), 0.0), 0.3);
        assert_eq!(ground_height_at(&c, glam::Vec2::new(1.0, 1.0), -1.0), 0.0, "a top above the step limit is not ground yet");
        assert_eq!(ground_height_at(&c, glam::Vec2::new(-50.0, 90.0), 0.0), 0.0, "far outside the indexed area");
        assert_eq!(ground_height_at(&c, glam::Vec2::new(f32::NAN, 1.0), 0.0), 0.0, "NaN is on no top");
        // `append` drops the index: a top added afterwards is seen.
        let more = GroundCandidates { box_tops: vec![top((10.0, 10.0), (12.0, 12.0), 0.3)], ..Default::default() };
        assert_eq!(ground_height_at(&c, glam::Vec2::new(11.0, 11.0), 0.0), 0.0);
        c.append(&more);
        assert_eq!(ground_height_at(&c, glam::Vec2::new(11.0, 11.0), 0.0), 0.3, "the index was rebuilt after append");
        // A clone carries its own answer.
        assert_eq!(ground_height_at(&c.clone(), glam::Vec2::new(1.0, 1.0), 0.0), 0.3);
    }
}
