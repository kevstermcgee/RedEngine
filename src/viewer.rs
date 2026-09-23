//! Real-time first-person rendering: the "Red Engine 2" viewer.
//!
//! This reuses the offline engine's scene schema, mesh generation, and shader pipelines
//! (see [`crate::render`] / [`crate::gpu`]) but draws directly into a window's swapchain
//! surface every frame instead of an offscreen texture read back to PNG/MP4, and the camera
//! is driven by player input ([`FpsCamera`]) instead of the scene's `camera` track.

use crate::gpu::{
    create_crosshair_pipeline, create_pipelines, make_shadow_sampler, CrosshairPipeline, CrosshairUniform,
    GlobalUniform, GpuMesh, ObjectUniform, Pipelines, MSAA_SAMPLES, SHADOW_SIZE,
};
use crate::mesh::{Mesh, Vertex};
use crate::props::prop_parts;
use crate::render::{build_globals_common, collect_leaf_meshes, collect_leaf_transforms};
use crate::schema::{Object, ObjectKind, PrimKind, Scene};
use glam::{Mat4, Quat, Vec3, Vec4};

fn align_up(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

/// A free-look camera driven by player input rather than a scene keyframe track. Yaw/pitch are
/// radians; yaw 0 / pitch 0 looks down `-Z` (matching the offline engine's default camera
/// convention), yaw increases turning right, pitch increases looking up.
pub struct FpsCamera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
    pub near: f32,
    pub far: f32,
}

impl FpsCamera {
    pub const PITCH_LIMIT: f32 = 89.0_f32.to_radians() - 0.001;

    pub fn new(position: Vec3, yaw_deg: f32) -> Self {
        FpsCamera { position, yaw: yaw_deg.to_radians(), pitch: 0.0, fov_deg: 70.0, near: 0.05, far: 200.0 }
    }

    /// Full look direction, pitch included (used for the view matrix).
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp).normalize()
    }

    /// Horizontal-only look direction (used for walking, so looking up/down doesn't fly you
    /// into the ceiling or floor).
    pub fn forward_flat(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(sy, 0.0, -cy)
    }

    /// Horizontal-only right vector, perpendicular to `forward_flat`.
    pub fn right_flat(&self) -> Vec3 {
        let f = self.forward_flat();
        Vec3::new(-f.z, 0.0, f.x)
    }

    /// Right vector for the *full* (pitch-included) look direction. Since this viewer never
    /// rolls the camera, it's identical to `right_flat` — exposed under this name for viewmodel
    /// placement, where it naturally pairs with `forward`/`up` rather than the walk-only
    /// `*_flat` vectors.
    pub fn right(&self) -> Vec3 {
        self.right_flat()
    }

    /// Up vector orthogonal to `forward` and `right`, so a held item tips with the player's
    /// pitch (looking down tilts it down) instead of staying screen-locked.
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward()).normalize()
    }

    pub fn look(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    fn view_proj(&self, aspect: f32) -> Mat4 {
        let proj =
            glam::camera::rh::proj::directx::perspective(self.fov_deg.to_radians(), aspect, self.near, self.far);
        let target = self.position + self.forward();
        let view = glam::camera::rh::view::look_at_mat4(self.position, target, Vec3::Y);
        proj * view
    }
}

/// Builds the world transform for something held in the player's hand (a viewmodel), given a
/// pose expressed in the camera's own local frame: local `+X` = camera right, `+Y` = camera up,
/// `+Z` = camera forward. Authoring a hand-held item's offset/rotation in that frame means "tip
/// raised, tilted right, half a meter forward" instead of hand-deriving basis vectors per call.
pub fn viewmodel_transform(camera: &FpsCamera, local_offset: Vec3, local_rotation: Mat4) -> Mat4 {
    let basis = Mat4::from_cols(
        camera.right().extend(0.0),
        camera.up().extend(0.0),
        camera.forward().extend(0.0),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    );
    Mat4::from_translation(camera.position) * basis * Mat4::from_translation(local_offset) * local_rotation
}

/// Steel-grey, slightly worn material for the crowbar viewmodel built by [`build_crowbar_mesh`].
pub const CROWBAR_COLOR: Vec3 = Vec3::new(0.36, 0.38, 0.42);
pub const CROWBAR_METALLIC: f32 = 0.65;
pub const CROWBAR_ROUGHNESS: f32 = 0.4;

/// Appends `src`'s vertices/indices into `dst`, transformed by `transform` — the same
/// "combine primitives placed by local transforms" approach `humanoid` uses for its capsule rig,
/// but done directly (a viewmodel isn't a scene object, so it has no `schema`/`skeleton` node of
/// its own to hang a `group` off of).
fn append_transformed(dst: &mut Mesh, src: &Mesh, transform: Mat4) {
    let normal_mat = transform.inverse().transpose();
    let base = dst.vertices.len() as u32;
    for v in &src.vertices {
        let p = transform.transform_point3(Vec3::from_array(v.pos));
        let n = normal_mat.transform_vector3(Vec3::from_array(v.normal)).normalize_or_zero();
        dst.vertices.push(Vertex { pos: p.to_array(), normal: n.to_array() });
    }
    dst.indices.extend(src.indices.iter().map(|&i| base + i));
}

/// A crowbar for the player's hand: grip at the local origin, extending toward local `+Z`
/// (matching [`viewmodel_transform`]'s pose convention), built as a chain of short segments —
/// each welded to the last with a small cumulative bend — rather than one straight rod, so it
/// reads as a gently curved pry bar that curls into a forked claw at the tip instead of a
/// perfectly straight stick with a single kink. Built from primitives at a fixed pose rather
/// than authored as a scene `humanoid`/`group`, since it's a viewmodel, not a world object.
pub fn build_crowbar_mesh() -> Mesh {
    let mut m = Mesh::default();

    // `Mesh::cylinder` extends along local Y; rotating +90 deg about X turns that into local Z,
    // matching the "grip at origin, bar extends toward +Z" convention used everywhere below.
    let align_z = Mat4::from_quat(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2));

    // `cursor` accumulates transforms up to the *end* of the last placed segment (the next
    // joint), in the same pre-`align_z` local space every segment below is built in — rotating
    // before advancing means each bend pivots exactly at the joint, not at the next segment's
    // own center.
    let mut cursor = Mat4::IDENTITY;
    let place_segment = |m: &mut Mesh, mesh: &Mesh, len: f32, bend_after: f32, cursor: &mut Mat4| {
        append_transformed(m, mesh, align_z * *cursor * Mat4::from_translation(Vec3::new(0.0, len * 0.5, 0.0)));
        *cursor *= Mat4::from_translation(Vec3::new(0.0, len, 0.0)) * Mat4::from_quat(Quat::from_rotation_x(bend_after));
    };

    // Grip: a slightly thicker handle with two raised collar bands standing in for a
    // wrapped-cord texture the engine has no way to actually paint on.
    let grip_len = 0.10;
    let grip_radius = 0.021;
    place_segment(&mut m, &Mesh::cylinder(grip_radius, grip_len, 10), grip_len, 0.0, &mut cursor);
    for collar_y in [0.03, 0.08] {
        let collar = Mesh::cylinder(grip_radius * 1.25, 0.012, 10);
        append_transformed(&mut m, &collar, align_z * Mat4::from_translation(Vec3::new(0.0, collar_y, 0.0)));
    }

    // Shaft: three segments with a small bend apiece, so the bar gently curves along its whole
    // length rather than staying razor-straight until the hook.
    let segment_len = 0.11;
    let segment_radius = 0.016;
    let segment_bend = 4.0_f32.to_radians();
    for _ in 0..3 {
        place_segment(&mut m, &Mesh::cylinder(segment_radius, segment_len, 10), segment_len, segment_bend, &mut cursor);
    }

    // Hooked tip: two more segments bending hard, curling the end over into a claw.
    let tip_len = 0.075;
    let tip_radius = 0.014;
    let tip_bend = 22.0_f32.to_radians();
    for _ in 0..2 {
        place_segment(&mut m, &Mesh::cylinder(tip_radius, tip_len, 9), tip_len, tip_bend, &mut cursor);
    }

    // Forked pry claw capping the hook: two thin flattened prongs splayed apart around the
    // bar's own axis (a rotation about local Y, the "along the bar" direction here), leaving a
    // V notch between them — the recognizable nail-pulling business end.
    for side in [-1.0f32, 1.0] {
        let claw = Mesh::cuboid(Vec3::new(0.05, 0.012, 0.045));
        let splay = cursor
            * Mat4::from_quat(Quat::from_rotation_y(side * 16.0_f32.to_radians()))
            * Mat4::from_translation(Vec3::new(side * 0.02, 0.022, 0.0));
        append_transformed(&mut m, &claw, align_z * splay);
    }

    m
}

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
/// leaking up through them (or vice versa). Deliberately narrower than head-to-toe so a rug or
/// low curb doesn't block walking, same as before.
const PLAYER_BAND_MIN_Y: f32 = 0.05;
const PLAYER_BAND_MAX_Y: f32 = 2.0;

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
    fn walk(objects: &[crate::schema::Object], parent: Mat4, out: &mut Vec<Collider2D>) {
        for o in objects {
            let local = crate::render::trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
            let world = parent * local;
            match &o.kind {
                crate::schema::ObjectKind::Prim(crate::schema::PrimKind::Box { size }) => {
                    push_box_collider(world, *size * 0.5, out);
                }
                crate::schema::ObjectKind::Prim(_) => {}
                crate::schema::ObjectKind::Group(children) => walk(children, world, out),
                crate::schema::ObjectKind::Humanoid(_) => {}
                crate::schema::ObjectKind::Stairs(_) => {}
                crate::schema::ObjectKind::Prop(p) => {
                    let mut min = Vec3::splat(f32::INFINITY);
                    let mut max = Vec3::splat(f32::NEG_INFINITY);
                    for part in prop_parts(p.kind) {
                        let part_world = world * part.local_transform;
                        let half = prim_half_extent(&part.shape);
                        for sx in [-1.0f32, 1.0] {
                            for sy in [-1.0f32, 1.0] {
                                for sz in [-1.0f32, 1.0] {
                                    let corner = Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                                    let wp = part_world.transform_point3(corner);
                                    min = min.min(wp);
                                    max = max.max(wp);
                                }
                            }
                        }
                    }
                    out.push(Collider2D {
                        min: glam::Vec2::new(min.x, min.z),
                        max: glam::Vec2::new(max.x, max.z),
                        min_y: min.y,
                        max_y: max.y,
                    });
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&scene.objects, Mat4::IDENTITY, &mut out);
    out
}

/// Filters a full collider list down to the ones that actually block movement *at the player's
/// current foot height* — see [`PLAYER_BAND_MIN_Y`]/[`PLAYER_BAND_MAX_Y`]'s doc comment for why
/// this has to be dynamic (relative to `foot_y`) rather than a fixed absolute band once a map
/// has more than one floor.
pub fn colliders_on_floor(colliders: &[Collider2D], foot_y: f32) -> Vec<Collider2D> {
    let lo = foot_y + PLAYER_BAND_MIN_Y;
    let hi = foot_y + PLAYER_BAND_MAX_Y;
    colliders.iter().copied().filter(|c| c.max_y >= lo && c.min_y <= hi).collect()
}

/// A staircase's walkable ramp, world-space. `world_to_local` maps a world XZ (any Y — a pure
/// yaw rotation never mixes Y into X/Z, so the ramp's footprint test and height formula don't
/// need the query point's real world Y at all) back into the stairs' own frame, where the ramp
/// runs along local `+Z` from `-half_run` (height `base_y`) to `+half_run` (height
/// `base_y + rise`).
struct StairsRamp {
    world_to_local: Mat4,
    half_width: f32,
    half_run: f32,
    base_y: f32,
    rise: f32,
}

impl StairsRamp {
    /// The world height of the ramp at `xz`, or `None` outside its footprint.
    fn height_at(&self, xz: glam::Vec2) -> Option<f32> {
        let local = self.world_to_local.transform_point3(Vec3::new(xz.x, 0.0, xz.y));
        if local.x.abs() > self.half_width || local.z.abs() > self.half_run {
            return None;
        }
        let f = ((local.z + self.half_run) / (2.0 * self.half_run)).clamp(0.0, 1.0);
        Some(self.base_y + self.rise * f)
    }
}

/// Every standable surface in the scene, precomputed once at load (like [`Collider2D`]s):
/// every `box` primitive's and box-shaped `Prop` part's top face (reusing [`push_box_collider`]
/// — a `Collider2D`'s `max_y` doubles as "the height of this box's top"), plus every
/// [`crate::schema::StairsDef`]'s ramp. See [`ground_height_at`] for how these become an actual
/// walkable ground height.
pub struct GroundCandidates {
    box_tops: Vec<Collider2D>,
    stairs: Vec<StairsRamp>,
}

pub fn collect_ground_candidates(scene: &Scene) -> GroundCandidates {
    fn walk(objects: &[Object], parent: Mat4, box_tops: &mut Vec<Collider2D>, stairs: &mut Vec<StairsRamp>) {
        for o in objects {
            let local = crate::render::trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
            let world = parent * local;
            match &o.kind {
                ObjectKind::Prim(PrimKind::Box { size }) => push_box_collider(world, *size * 0.5, box_tops),
                ObjectKind::Prim(_) => {}
                ObjectKind::Group(children) => walk(children, world, box_tops, stairs),
                ObjectKind::Humanoid(_) => {}
                ObjectKind::Prop(p) => {
                    for part in prop_parts(p.kind) {
                        if let PrimKind::Box { size } = part.shape {
                            push_box_collider(world * part.local_transform, size * 0.5, box_tops);
                        }
                    }
                }
                ObjectKind::Stairs(s) => stairs.push(StairsRamp {
                    world_to_local: world.inverse(),
                    half_width: s.width * 0.5,
                    half_run: s.run * 0.5,
                    base_y: world.transform_point3(Vec3::ZERO).y,
                    rise: s.rise,
                }),
            }
        }
    }
    let mut box_tops = Vec::new();
    let mut stairs = Vec::new();
    walk(&scene.objects, Mat4::IDENTITY, &mut box_tops, &mut stairs);
    GroundCandidates { box_tops, stairs }
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

/// Conservative local-space half-extent for a primitive, used only for the interaction bounding
/// sphere above (not for rendering) — doesn't need to be exact, just big enough to cover the
/// mesh.
fn prim_half_extent(p: &PrimKind) -> Vec3 {
    match p {
        PrimKind::Box { size } => *size * 0.5,
        PrimKind::Sphere { radius } => Vec3::splat(*radius),
        PrimKind::Cylinder { radius, height } | PrimKind::Cone { radius, height } | PrimKind::Capsule { radius, height } => {
            Vec3::new(*radius, height * 0.5, *radius)
        }
        PrimKind::Plane { size } => Vec3::new(size.0 * 0.5, 0.02, size.1 * 0.5),
    }
}

fn accumulate_world_bounds(o: &Object, parent: Mat4, min: &mut Vec3, max: &mut Vec3) {
    let local = crate::render::trs(o.position.sample(0.0), o.rotation.sample(0.0), o.scale.sample(0.0));
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
    let mut out = Vec::with_capacity(scene.objects.len());
    for (object_index, o) in scene.objects.iter().enumerate() {
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

/// The six clip-space frustum planes of `view_proj`, each packed as `(A, B, C, D)` such that a
/// world-space point `p` is inside that plane's half-space when `A*p.x + B*p.y + C*p.z + D >=
/// 0`. Standard Gribb/Hartmann extraction directly from the combined view-projection matrix —
/// works identically for the camera's perspective frustum and the shadow light's orthographic
/// one, so both the main pass and the shadow pass can cull against it with the same code.
fn frustum_planes(view_proj: Mat4) -> [Vec4; 6] {
    let (c0, c1, c2, c3) = (view_proj.x_axis, view_proj.y_axis, view_proj.z_axis, view_proj.w_axis);
    let row0 = Vec4::new(c0.x, c1.x, c2.x, c3.x);
    let row1 = Vec4::new(c0.y, c1.y, c2.y, c3.y);
    let row2 = Vec4::new(c0.z, c1.z, c2.z, c3.z);
    let row3 = Vec4::new(c0.w, c1.w, c2.w, c3.w);
    [row3 + row0, row3 - row0, row3 + row1, row3 - row1, row2, row3 - row2]
}

/// World-space AABB (center, half-extent) of a local-space box after `transform` — exact
/// center, and a conservative half-extent computed from the transform's basis vectors (Ericson,
/// *Real-Time Collision Detection* §4.2.6) rather than transforming and re-bounding all 8
/// corners, since this is recomputed for every mesh every frame.
fn world_aabb(transform: Mat4, local_min: Vec3, local_max: Vec3) -> (Vec3, Vec3) {
    let local_center = (local_min + local_max) * 0.5;
    let local_half = (local_max - local_min) * 0.5;
    let world_center = transform.transform_point3(local_center);
    let bx = transform.x_axis.truncate().abs();
    let by = transform.y_axis.truncate().abs();
    let bz = transform.z_axis.truncate().abs();
    let world_half = bx * local_half.x + by * local_half.y + bz * local_half.z;
    (world_center, world_half)
}

/// True if the AABB (`center`, `half`) is entirely outside at least one of `planes` — the
/// standard "positive vertex" test: for each plane, the corner most in the box's favor is
/// `center + half` projected along the plane normal's sign, so if even that corner is outside,
/// the whole box is.
fn aabb_outside_frustum(center: Vec3, half: Vec3, planes: &[Vec4; 6]) -> bool {
    for p in planes {
        let normal = Vec3::new(p.x, p.y, p.z);
        let radius = half.x * normal.x.abs() + half.y * normal.y.abs() + half.z * normal.z.abs();
        if normal.dot(center) + p.w + radius < 0.0 {
            return true;
        }
    }
    false
}

struct LiveTargets {
    width: u32,
    height: u32,
    /// MSAA-resolved into the swapchain view at the end of the viewmodel pass (see
    /// [`LiveRenderer::render`]) — the swapchain itself can't be a multisampled texture, so the
    /// background/main/viewmodel passes all draw into this instead and only the last of them
    /// resolves.
    multisampled_color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    shadow_view: wgpu::TextureView,
    viewmodel_depth_view: wgpu::TextureView,
}

impl LiveTargets {
    fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let extent = wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 };
        let make_depth = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: MSAA_SAMPLES,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let multisampled_color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live-msaa-color-target"),
            size: extent,
            mip_level_count: 1,
            sample_count: MSAA_SAMPLES,
            dimension: wgpu::TextureDimension::D2,
            format: color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("live-shadow-map"),
            size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        LiveTargets {
            width,
            height,
            multisampled_color_view: multisampled_color_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: make_depth("live-depth-target"),
            shadow_view: shadow_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            // A separate depth target cleared fresh right before the viewmodel pass, so the held
            // crowbar always draws on top of the world instead of clipping into a nearby wall —
            // the standard first-person "weapon in its own depth space" trick.
            viewmodel_depth_view: make_depth("live-viewmodel-depth-target"),
        }
    }
}

/// Everything needed to draw one scene, live, into a window surface every frame.
pub struct LiveRenderer {
    color_format: wgpu::TextureFormat,
    pipelines: Pipelines,
    global_buf: wgpu::Buffer,
    global_bind_group_uniform: wgpu::BindGroup,
    global_bind_group_full: wgpu::BindGroup,
    object_buf: wgpu::Buffer,
    object_stride: u64,
    object_bind_group: wgpu::BindGroup,
    targets: LiveTargets,
    meshes: Vec<GpuMesh>,
    viewmodel_mesh: GpuMesh,
    crosshair: CrosshairPipeline,
    crosshair_buf: wgpu::Buffer,
    crosshair_bind_group: wgpu::BindGroup,
}

impl LiveRenderer {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, scene: &Scene, width: u32, height: u32) -> Self {
        let pipelines = create_pipelines(device, color_format, MSAA_SAMPLES);
        let shadow_sampler = make_shadow_sampler(device);
        let targets = LiveTargets::new(device, color_format, width, height);

        let mut raw_meshes = Vec::new();
        collect_leaf_meshes(&scene.objects, &mut raw_meshes);
        let meshes: Vec<GpuMesh> = raw_meshes.iter().map(|m| GpuMesh::upload(device, m)).collect();
        let viewmodel_mesh = GpuMesh::upload(device, &build_crowbar_mesh());
        // Two extra slots in the shared object-uniform buffer: one for the camera-attached
        // first-person viewmodel (drawn in its own always-on-top pass), one for a second
        // instance of the same crowbar mesh rigidly attached to the third-person body's hand
        // bone (drawn as an ordinary world object, shadowed/occluded like any prop). Both are
        // written and bound (via a dynamic offset) alongside the scene meshes each frame.
        let draw_count = meshes.len() as u64 + 2;

        let global_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live-global-uniform"),
            size: std::mem::size_of::<GlobalUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let object_stride = align_up(std::mem::size_of::<ObjectUniform>() as u64, alignment);
        let object_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live-object-uniforms"),
            size: object_stride * draw_count,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let global_bind_group_uniform = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-global-bind-group-uniform"),
            layout: &pipelines.layouts.global_uniform,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: global_buf.as_entire_binding() }],
        });
        let global_bind_group_full = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-global-bind-group-full"),
            layout: &pipelines.layouts.global_full,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: global_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&targets.shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            ],
        });
        let object_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-object-bind-group"),
            layout: &pipelines.layouts.object,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &object_buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
                }),
            }],
        });

        let crosshair = create_crosshair_pipeline(device, color_format);
        let crosshair_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("live-crosshair-uniform"),
            size: std::mem::size_of::<CrosshairUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let crosshair_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-crosshair-bind-group"),
            layout: &crosshair.bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: crosshair_buf.as_entire_binding() }],
        });

        LiveRenderer {
            color_format,
            pipelines,
            global_buf,
            global_bind_group_uniform,
            global_bind_group_full,
            object_buf,
            object_stride,
            object_bind_group,
            targets,
            meshes,
            viewmodel_mesh,
            crosshair,
            crosshair_buf,
            crosshair_bind_group,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == self.targets.width && height == self.targets.height {
            return;
        }
        self.targets = LiveTargets::new(device, self.color_format, width, height);
    }

    /// Renders one frame: `t` is the scene animation time (seconds, for any keyframed objects
    /// in the room — the camera itself is not part of the scene here), `camera` is the player's
    /// current view, `target_view` is the swapchain frame to draw into. `crosshair_highlighted`
    /// switches the aim reticle to its "something's in reach" color. `weapon_transform` is the
    /// camera-attached first-person crowbar's world transform (see [`viewmodel_transform`]),
    /// drawn last in its own depth space so it never clips into world geometry.
    /// `hand_prop_transform` is a second instance of the same crowbar mesh, rigidly attached to
    /// the third-person body's hand bone instead of the camera — drawn as an ordinary world
    /// object (shadowed, depth-tested against the world) alongside the scene meshes. Callers
    /// hide whichever one doesn't apply to the current view mode by scaling it to ~0.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        t: f32,
        camera: &FpsCamera,
        target_view: &wgpu::TextureView,
        crosshair_highlighted: bool,
        weapon_transform: Mat4,
        hand_prop_transform: Mat4,
    ) {
        let aspect = self.targets.width.max(1) as f32 / self.targets.height.max(1) as f32;
        let view_proj = camera.view_proj(aspect);
        let globals = build_globals_common(scene, t, camera.position, view_proj);
        queue.write_buffer(&self.global_buf, 0, bytemuck::bytes_of(&globals));

        let mut transforms = Vec::with_capacity(self.meshes.len());
        collect_leaf_transforms(&scene.objects, t, Mat4::IDENTITY, &mut transforms);
        debug_assert_eq!(transforms.len(), self.meshes.len());

        // Per-mesh frustum culling: which scene meshes are worth a draw call this frame, tested
        // against the camera's frustum (main pass) and, when a shadow-casting light is active,
        // the light's own ortho frustum (shadow pass) — skips both the vertex/fragment work and
        // the draw call for anything off-screen, which starts to matter once a prop-hunt map has
        // a few dozen props instead of a handful of room furniture.
        let cam_planes = frustum_planes(view_proj);
        let shadow_active = globals.counts[1] >= 0.0;
        let light_planes =
            shadow_active.then(|| frustum_planes(Mat4::from_cols_array_2d(&globals.light_view_proj)));
        let mut main_visible = Vec::with_capacity(self.meshes.len());
        let mut shadow_visible = Vec::with_capacity(self.meshes.len());
        for (i, mesh) in self.meshes.iter().enumerate() {
            let (center, half) = world_aabb(transforms[i].0, mesh.local_min, mesh.local_max);
            main_visible.push(!aabb_outside_frustum(center, half, &cam_planes));
            shadow_visible.push(match &light_planes {
                Some(planes) => !aabb_outside_frustum(center, half, planes),
                None => false,
            });
        }

        // Every object's uniform data is staged into one contiguous byte buffer and uploaded
        // with a single `write_buffer` call instead of one call per mesh — object_stride is
        // alignment-padded past ObjectUniform's own size, so the staging buffer is built at full
        // stride width and each uniform's bytes are copied into its slot, padding left as-is.
        let weapon_slot = self.meshes.len() as u64;
        let hand_prop_slot = weapon_slot + 1;
        let total_slots = hand_prop_slot + 1;
        let mut object_data = vec![0u8; (self.object_stride * total_slots) as usize];
        let stage = |data: &mut [u8], slot: u64, stride: u64, uniform: &ObjectUniform| {
            let start = (slot * stride) as usize;
            let bytes = bytemuck::bytes_of(uniform);
            data[start..start + bytes.len()].copy_from_slice(bytes);
        };

        for (i, (world, mat)) in transforms.iter().enumerate() {
            let normal_mat = world.inverse().transpose();
            let obj_uniform = ObjectUniform {
                model: world.to_cols_array_2d(),
                normal_mat: normal_mat.to_cols_array_2d(),
                base_color: [mat.color.x, mat.color.y, mat.color.z, 1.0],
                material: [mat.metallic, mat.roughness, 0.0, 0.0],
                emissive: [mat.emissive.x, mat.emissive.y, mat.emissive.z, 0.0],
            };
            stage(&mut object_data, i as u64, self.object_stride, &obj_uniform);
        }

        let crowbar_uniform = |world: Mat4| {
            let normal_mat = world.inverse().transpose();
            ObjectUniform {
                model: world.to_cols_array_2d(),
                normal_mat: normal_mat.to_cols_array_2d(),
                base_color: [CROWBAR_COLOR.x, CROWBAR_COLOR.y, CROWBAR_COLOR.z, 1.0],
                material: [CROWBAR_METALLIC, CROWBAR_ROUGHNESS, 0.0, 0.0],
                emissive: [0.0, 0.0, 0.0, 0.0],
            }
        };
        stage(&mut object_data, weapon_slot, self.object_stride, &crowbar_uniform(weapon_transform));
        stage(&mut object_data, hand_prop_slot, self.object_stride, &crowbar_uniform(hand_prop_transform));
        queue.write_buffer(&self.object_buf, 0, &object_data);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("live-frame-encoder") });

        if globals.counts[1] >= 0.0 {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-shadow-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.shadow_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            shadow_pass.set_pipeline(&self.pipelines.shadow);
            shadow_pass.set_bind_group(0, &self.global_bind_group_uniform, &[]);
            for (i, mesh) in self.meshes.iter().enumerate() {
                if !shadow_visible[i] {
                    continue;
                }
                shadow_pass.set_bind_group(1, &self.object_bind_group, &[(i as u64 * self.object_stride) as u32]);
                shadow_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                shadow_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            // The hand-held (third-person) crowbar is an ordinary world object, so it casts a
            // shadow like any other prop — unlike the always-on-top first-person viewmodel.
            shadow_pass.set_bind_group(1, &self.object_bind_group, &[(hand_prop_slot * self.object_stride) as u32]);
            shadow_pass.set_vertex_buffer(0, self.viewmodel_mesh.vertex_buf.slice(..));
            shadow_pass.set_index_buffer(self.viewmodel_mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            shadow_pass.draw_indexed(0..self.viewmodel_mesh.index_count, 0, 0..1);
        }

        {
            let mut bg_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-background-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.multisampled_color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            bg_pass.set_pipeline(&self.pipelines.background);
            bg_pass.set_bind_group(0, &self.global_bind_group_uniform, &[]);
            bg_pass.draw(0..3, 0..1);
        }

        {
            let mut main_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.multisampled_color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            main_pass.set_pipeline(&self.pipelines.main);
            main_pass.set_bind_group(0, &self.global_bind_group_full, &[]);
            for (i, mesh) in self.meshes.iter().enumerate() {
                if !main_visible[i] {
                    continue;
                }
                main_pass.set_bind_group(1, &self.object_bind_group, &[(i as u64 * self.object_stride) as u32]);
                main_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                main_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                main_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            // Hand-held (third-person) crowbar instance: normal depth test against the world,
            // so a wall between the camera and the player correctly occludes it like any prop.
            main_pass.set_bind_group(1, &self.object_bind_group, &[(hand_prop_slot * self.object_stride) as u32]);
            main_pass.set_vertex_buffer(0, self.viewmodel_mesh.vertex_buf.slice(..));
            main_pass.set_index_buffer(self.viewmodel_mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            main_pass.draw_indexed(0..self.viewmodel_mesh.index_count, 0, 0..1);
        }

        {
            // Fresh depth clear (not `self.targets.depth_view`, which still holds the world's
            // depth) so the crowbar always draws over the world, matching how a first-person
            // weapon is expected to behave rather than clipping into a wall the player is close to.
            // This is also the last of the three multisampled passes, so it's the one that
            // resolves into the actual (single-sampled) swapchain view.
            let mut vm_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-viewmodel-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.multisampled_color_view,
                    depth_slice: None,
                    resolve_target: Some(target_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.viewmodel_depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            vm_pass.set_pipeline(&self.pipelines.main);
            vm_pass.set_bind_group(0, &self.global_bind_group_full, &[]);
            vm_pass.set_bind_group(1, &self.object_bind_group, &[(weapon_slot * self.object_stride) as u32]);
            vm_pass.set_vertex_buffer(0, self.viewmodel_mesh.vertex_buf.slice(..));
            vm_pass.set_index_buffer(self.viewmodel_mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            vm_pass.draw_indexed(0..self.viewmodel_mesh.index_count, 0, 0..1);
        }

        let crosshair_color =
            if crosshair_highlighted { [1.0, 0.85, 0.2, 1.0] } else { [1.0, 1.0, 1.0, 0.85] };
        let crosshair_uniform = CrosshairUniform {
            color: crosshair_color,
            to_ndc: [2.0 / self.targets.width.max(1) as f32, 2.0 / self.targets.height.max(1) as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.crosshair_buf, 0, bytemuck::bytes_of(&crosshair_uniform));
        {
            let mut crosshair_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-crosshair-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            crosshair_pass.set_pipeline(&self.crosshair.pipeline);
            crosshair_pass.set_bind_group(0, &self.crosshair_bind_group, &[]);
            crosshair_pass.draw(0..12, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }
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
        let ramp = StairsRamp { world_to_local: Mat4::IDENTITY, half_width: 1.0, half_run: 2.0, base_y: 0.0, rise: 3.0 };
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp] };
        // ~WALK_SPEED (3.2 m/s) at FIXED_DT (1/60s) — the real per-tick horizontal step size.
        let foot_y = walk(&candidates, straight_line(glam::Vec2::new(0.0, -2.0), glam::Vec2::new(0.0, 2.0), 3.2 / 60.0));
        assert!(foot_y > 2.9, "expected to reach near the top of a rise-3.0 ramp, got {foot_y}");
    }

    /// The same ramp walked in reverse (top to bottom) should descend smoothly back to ~0,
    /// not get stuck partway — a player should be able to walk back down a staircase.
    #[test]
    fn stairs_ramp_descends_smoothly_top_to_bottom() {
        let ramp = StairsRamp { world_to_local: Mat4::IDENTITY, half_width: 1.0, half_run: 2.0, base_y: 0.0, rise: 3.0 };
        let candidates = GroundCandidates { box_tops: vec![], stairs: vec![ramp] };
        let mut foot_y = 3.0; // start already on top, as if having just climbed up
        for xz in straight_line(glam::Vec2::new(0.0, 2.0), glam::Vec2::new(0.0, -2.0), 3.2 / 60.0) {
            let ground = ground_height_at(&candidates, xz, foot_y);
            // Gravity pulls it down between ticks in the real game; here just track the ground
            // height directly, since a descending ramp is always "reachable" from above (you
            // fall onto it, you don't need to climb up to it).
            foot_y = ground;
        }
        assert!(foot_y < 0.1, "expected to have descended back to ~0, got {foot_y}");
    }

    /// Approaching a ramp from its *tall* end while standing at ground level must not teleport
    /// the player straight up to full rise — only once they're close enough (within
    /// `GROUND_SNAP_EPS`) should the ramp's height become a valid candidate at all.
    #[test]
    fn stairs_ramp_tall_end_is_unreachable_from_ground_level() {
        let ramp = StairsRamp { world_to_local: Mat4::IDENTITY, half_width: 1.0, half_run: 2.0, base_y: 0.0, rise: 3.0 };
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
        let deck = Collider2D {
            min: glam::Vec2::new(-5.0, -5.0),
            max: glam::Vec2::new(5.0, 5.0),
            min_y: 2.8,
            max_y: 3.0,
        };
        let candidates = GroundCandidates { box_tops: vec![deck], stairs: vec![] };
        let xz = glam::Vec2::new(0.0, 0.0);
        assert_eq!(ground_height_at(&candidates, xz, 0.0), 0.0, "deck must be unreachable from ground level");
        assert_eq!(
            ground_height_at(&candidates, xz, 2.9),
            3.0,
            "deck must become reachable once already close to its height"
        );
    }
}
