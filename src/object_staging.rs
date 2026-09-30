//! Per-frame object-uniform preparation for the live renderer, without rebuilding what did not change.
//!
//! [`SceneStaging`] keeps one persistent byte buffer laid out exactly like the GPU object-uniform buffer (one `stride`-wide slot per
//! draw) and, for each slot, the inputs that produced it (world matrix + material). A frame samples the scene's tracks as before
//! ([`crate::render::collect_leaf_transforms`], into reused storage), but a slot whose inputs are bit-identical to last frame's is left
//! alone: no inverse-transpose, no re-staging, no world-bounds recompute, no upload. Changed slots are coalesced into a few contiguous
//! byte ranges ([`SceneStaging::dirty_ranges`]) for `queue.write_buffer`.
//!
//! Invalidation is by value, not by flag: `Scene` fields are public and gameplay overwrites tracks (physics writes `Track::constant`
//! poses, rules hide or scale pooled objects, parents move whole subtrees), so nothing can be trusted to say "unchanged". Comparing the
//! sampled result is correct for every mutation a scene supports, including a parent transform (it changes every descendant's world
//! matrix) and world-wrap offsets (they are part of the slot's matrix). Invariant: after the caller uploads [`dirty_ranges`], the GPU
//! buffer equals [`SceneStaging::bytes`] for every slot that was staged.
//!
//! Also here: the frustum-culling helpers ([`frustum_planes`], [`world_aabb`], [`aabb_outside_frustum`]) and [`uncached_uniforms`], the
//! old rebuild-everything path, kept as the reference the tests compare against and the "before" the benchmarks measure.

use crate::gpu::ObjectUniform;
use crate::render::{collect_leaf_meshes, collect_leaf_transforms, collect_leaf_transforms_cached, PartsCache, SampledMaterial};
use crate::schema::Scene;
use glam::{Mat4, Vec3, Vec4};
use std::ops::Range;

/// Everything a slot's bytes are derived from. Equal keys give equal [`ObjectUniform`]s.
#[derive(Clone, Copy, PartialEq)]
struct SlotKey {
    model: [f32; 16],
    base_color: [f32; 4],
    material: [f32; 4],
    emissive: [f32; 4],
}

impl SlotKey {
    fn scene(world: Mat4, mat: &SampledMaterial) -> Self {
        SlotKey {
            model: world.to_cols_array(),
            base_color: [mat.color.x, mat.color.y, mat.color.z, 1.0],
            material: [mat.metallic, mat.roughness, 0.0, 0.0],
            emissive: [mat.emissive.x, mat.emissive.y, mat.emissive.z, 0.0],
        }
    }

    fn uniform(&self) -> ObjectUniform {
        let world = Mat4::from_cols_array(&self.model);
        ObjectUniform {
            model: world.to_cols_array_2d(),
            normal_mat: world.inverse().transpose().to_cols_array_2d(),
            base_color: self.base_color,
            material: self.material,
            emissive: self.emissive,
        }
    }
}

/// Slots this far apart (or closer) share one upload: re-sending a few clean slots is cheaper than another `write_buffer` call.
const MERGE_GAP_SLOTS: u64 = 8;

/// The persistent staging copy of the object-uniform buffer, the per-slot inputs it was built from, and the change list for the upload.
pub struct SceneStaging {
    stride: u64,
    n_meshes: usize,
    /// One entry per slot (`image * n_meshes + mesh`, then the held-weapon slots); `None` until first staged.
    keys: Vec<Option<SlotKey>>,
    data: Vec<u8>,
    /// World centre and half-extent of each scene slot's mesh, refreshed only when the slot changes.
    bounds: Vec<(Vec3, Vec3)>,
    /// Reused per-frame sampling storage.
    transforms: Vec<(Mat4, SampledMaterial)>,
    parts: PartsCache,
    main_visible: Vec<bool>,
    shadow_visible: Vec<bool>,
    dirty: Vec<Range<usize>>,
    /// Slots rewritten last frame (statistics for tests and benchmarks).
    last_rewritten: usize,
}

impl SceneStaging {
    /// Storage for `scene_slots` scene draws (`meshes * images`) plus `extra_slots` held-weapon draws, `stride` bytes apart.
    pub fn new(n_meshes: usize, images: usize, extra_slots: usize, stride: u64) -> Self {
        let scene_slots = n_meshes * images;
        let total = scene_slots + extra_slots;
        SceneStaging {
            stride,
            n_meshes,
            keys: vec![None; total],
            data: vec![0u8; stride as usize * total],
            bounds: vec![(Vec3::ZERO, Vec3::ZERO); scene_slots],
            transforms: Vec::with_capacity(n_meshes),
            parts: PartsCache::default(),
            main_visible: vec![false; scene_slots],
            shadow_visible: vec![false; scene_slots],
            dirty: Vec::new(),
            last_rewritten: 0,
        }
    }

    /// Samples the scene at `t`, restages every slot whose world matrix or material changed, and returns how many did.
    /// `offsets` are the world-wrap image translations (`[ZERO]` for an ordinary scene); `local_bounds` is each mesh's local AABB.
    pub fn update_scene(&mut self, scene: &Scene, t: f32, offsets: &[Vec3], local_bounds: &[(Vec3, Vec3)]) -> usize {
        self.transforms.clear();
        collect_leaf_transforms_cached(&scene.objects, t, Mat4::IDENTITY, &mut self.transforms, &mut self.parts);
        debug_assert_eq!(self.transforms.len(), self.n_meshes);
        let mut rewritten = 0;
        for (image, offset) in offsets.iter().enumerate() {
            let shift = Mat4::from_translation(*offset);
            for i in 0..self.n_meshes {
                let (world, mat) = &self.transforms[i];
                let world = if *offset == Vec3::ZERO { *world } else { shift * *world };
                let key = SlotKey::scene(world, mat);
                let slot = image * self.n_meshes + i;
                if self.keys[slot].as_ref() == Some(&key) {
                    continue;
                }
                self.write_slot(slot, key);
                self.bounds[slot] = world_aabb(world, local_bounds[i].0, local_bounds[i].1);
                rewritten += 1;
            }
        }
        self.last_rewritten = rewritten;
        rewritten
    }

    /// Restages a held-weapon (or any extra) slot from its inputs when they changed; returns whether they did. `slot` counts from 0 over
    /// the *whole* buffer, so extras start at `n_meshes * images`.
    pub fn update_extra(&mut self, slot: usize, world: Mat4, color: Vec3, metallic: f32, roughness: f32, emissive: Vec3) -> bool {
        let key = SlotKey::scene(world, &SampledMaterial { color, metallic, roughness, emissive });
        if self.keys[slot].as_ref() == Some(&key) {
            return false;
        }
        self.write_slot(slot, key);
        true
    }

    fn write_slot(&mut self, slot: usize, key: SlotKey) {
        let start = slot * self.stride as usize;
        let uniform = key.uniform();
        let bytes = bytemuck::bytes_of(&uniform);
        self.data[start..start + bytes.len()].copy_from_slice(bytes);
        self.keys[slot] = Some(key);
        let (s, e) = (start, start + self.stride as usize);
        let gap = (MERGE_GAP_SLOTS * self.stride) as usize;
        match self.dirty.last_mut() {
            Some(last) if s >= last.start && s <= last.end + gap => last.end = last.end.max(e),
            _ => self.dirty.push(s..e),
        }
    }

    /// Culls every scene slot against the camera (and, when a shadow light is active, the light's) frustum, honouring `hidden` (one flag per mesh).
    pub fn cull(&mut self, cam: &[Vec4; 6], light: Option<&[Vec4; 6]>, hidden: &[bool]) {
        for slot in 0..self.bounds.len() {
            let shown = !hidden[slot % self.n_meshes];
            let (center, half) = self.bounds[slot];
            self.main_visible[slot] = shown && !aabb_outside_frustum(center, half, cam);
            self.shadow_visible[slot] = match light {
                Some(planes) => shown && !aabb_outside_frustum(center, half, planes),
                None => false,
            };
        }
    }

    /// Whether scene slot `slot` is drawn in the colour pass after [`Self::cull`].
    pub fn main_visible(&self, slot: usize) -> bool {
        self.main_visible[slot]
    }

    /// Whether scene slot `slot` is drawn in the shadow pass after [`Self::cull`].
    pub fn shadow_visible(&self, slot: usize) -> bool {
        self.shadow_visible[slot]
    }

    /// Takes the coalesced byte ranges to upload (`queue.write_buffer(buf, range.start, &bytes()[range])`), clearing the change list.
    pub fn dirty_ranges(&mut self) -> Vec<Range<usize>> {
        std::mem::take(&mut self.dirty)
    }

    /// Like [`Self::dirty_ranges`] but appends into `out` (no allocation once `out` has grown).
    pub fn drain_dirty_into(&mut self, out: &mut Vec<Range<usize>>) {
        out.clear();
        out.append(&mut self.dirty);
    }

    /// The staged bytes, laid out like the GPU buffer.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Slots rewritten by the last [`Self::update_scene`].
    pub fn last_rewritten(&self) -> usize {
        self.last_rewritten
    }
}

/// The local-space AABB of every leaf mesh of `scene`, in draw order (what `GpuMesh::local_min/max` hold). For tools and benchmarks that
/// have no GPU; the live renderer already has these on its uploaded meshes.
pub fn local_bounds(scene: &Scene) -> Vec<(Vec3, Vec3)> {
    let mut meshes = Vec::new();
    collect_leaf_meshes(&scene.objects, &mut meshes);
    meshes
        .iter()
        .map(|m| {
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for v in &m.vertices {
                let p = Vec3::from_array(v.pos);
                lo = lo.min(p);
                hi = hi.max(p);
            }
            (lo, hi)
        })
        .collect()
}

/// The pre-cache path: samples the scene, allocates fresh vectors and a zeroed staging buffer for `slots` slots, and rebuilds every scene
/// uniform. Returns the full staging buffer. Reference for equivalence tests and the "before" of the benchmarks; not used by the renderer.
pub fn uncached_uniforms(scene: &Scene, t: f32, offsets: &[Vec3], stride: u64, slots: usize) -> Vec<u8> {
    let mut transforms = Vec::new();
    collect_leaf_transforms(&scene.objects, t, Mat4::IDENTITY, &mut transforms);
    let n = transforms.len();
    let mut data = vec![0u8; stride as usize * slots];
    for (image, offset) in offsets.iter().enumerate() {
        for (i, (world, mat)) in transforms.iter().enumerate() {
            let world = if *offset == Vec3::ZERO { *world } else { Mat4::from_translation(*offset) * *world };
            let key = SlotKey::scene(world, mat);
            let bytes = bytemuck::bytes_of(&key.uniform()).to_vec();
            let start = (image * n + i) * stride as usize;
            data[start..start + bytes.len()].copy_from_slice(&bytes);
        }
    }
    data
}

/// The six clip-space frustum planes of `view_proj`, each packed as `(A, B, C, D)` such that a
/// world-space point `p` is inside that plane's half-space when `A*p.x + B*p.y + C*p.z + D >=
/// 0`. Standard Gribb/Hartmann extraction directly from the combined view-projection matrix —
/// works identically for the camera's perspective frustum and the shadow light's orthographic
/// one, so both the main pass and the shadow pass can cull against it with the same code.
pub fn frustum_planes(view_proj: Mat4) -> [Vec4; 6] {
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
/// corners. Cached per slot: recomputed only when the slot's transform changes.
pub fn world_aabb(transform: Mat4, local_min: Vec3, local_max: Vec3) -> (Vec3, Vec3) {
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
pub fn aabb_outside_frustum(center: Vec3, half: Vec3, planes: &[Vec4; 6]) -> bool {
    for p in planes {
        let normal = Vec3::new(p.x, p.y, p.z);
        let radius = half.x * normal.x.abs() + half.y * normal.y.abs() + half.z * normal.z.abs();
        if normal.dot(center) + p.w + radius < 0.0 {
            return true;
        }
    }
    false
}
