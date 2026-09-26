//! Real-time first-person rendering: the "Red Engine 2" viewer.
//!
//! This reuses the offline engine's scene schema, mesh generation, and shader pipelines
//! (see [`crate::render`] / [`crate::gpu`]) but draws directly into a window's swapchain
//! surface every frame instead of an offscreen texture read back to PNG/MP4, and the camera
//! is driven by player input ([`FpsCamera`]) instead of the scene's `camera` track.

use crate::gpu::{
    create_crosshair_pipeline, create_pipelines, create_post_pipeline, make_shadow_sampler, post_uniform, CrosshairPipeline, CrosshairUniform, GlobalUniform,
    GpuMesh, ObjectUniform, Pipelines, PostFx, MSAA_SAMPLES, SHADOW_SIZE,
};
use crate::mesh::{Mesh, Vertex};
use crate::overlay::Overlay;
use crate::render::{build_globals_common, collect_leaf_meshes, collect_leaf_object_paths, collect_leaf_transforms};
use crate::schema::Scene;
use crate::weapons::Weapon;
use glam::{Mat4, Quat, Vec3, Vec4};
use std::collections::HashSet;

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
        FpsCamera { position, yaw: yaw_deg.to_radians(), pitch: 0.0, fov_deg: 90.0, near: 0.05, far: 200.0 }
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
        let proj = glam::camera::rh::proj::directx::perspective(self.fov_deg.to_radians(), aspect, self.near, self.far);
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
    let basis = Mat4::from_cols(camera.right().extend(0.0), camera.up().extend(0.0), camera.forward().extend(0.0), Vec4::new(0.0, 0.0, 0.0, 1.0));
    Mat4::from_translation(camera.position) * basis * Mat4::from_translation(local_offset) * local_rotation
}

/// Idle held pose of the bat viewmodel (a pitch about the camera's right axis, then a roll about
/// the view axis) — shared with `re2` (swing/third-person poses) and with [`build_held_parts`],
/// which aims the forearm so it leaves the bottom-right of the screen from this exact pose.
pub const IDLE_PITCH_DEG: f32 = -66.0;
pub const IDLE_ROLL_DEG: f32 = -20.0;

/// Ash-wood bat (linear RGB of ~#b98a52), the player's skin-tone hand, and the slate sleeve that
/// matches the third-person body (`#4a5568`).
const BAT_COLOR: Vec3 = Vec3::new(0.90, 0.50, 0.17);
/// Bats are chunkier than real ones on screen: a viewmodel has to read at a glance.
const BAT_GIRTH: f32 = 1.35;
pub(crate) const HAND_COLOR: Vec3 = Vec3::new(0.86, 0.42, 0.30);
pub(crate) const SLEEVE_COLOR: Vec3 = Vec3::new(0.069, 0.091, 0.138);

/// One separately-coloured piece of a held weapon (the renderer draws one mesh per material).
pub struct HeldPart {
    pub mesh: Mesh,
    pub color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    /// The forearm sleeve only exists in first person (in third person the body's own arm is there).
    pub first_person_only: bool,
    /// Which weapon this piece belongs to; only the active weapon's pieces are drawn.
    pub weapon: Weapon,
    /// Self-lit colour (the muzzle flash); zero for everything else.
    pub emissive: Vec3,
    /// A muzzle-flash piece: drawn only while the flash is up, and casts no shadow.
    pub muzzle_flash: bool,
}

impl HeldPart {
    /// An ordinary lit piece of `weapon` (no glow, not a flash).
    pub fn lit(weapon: Weapon, mesh: Mesh, color: Vec3, metallic: f32, roughness: f32, first_person_only: bool) -> HeldPart {
        HeldPart { mesh, color, metallic, roughness, first_person_only, weapon, emissive: Vec3::ZERO, muzzle_flash: false }
    }
}

/// The held items are placed with a basis of (right, up, forward) — a *mirror* of the right-handed
/// world (see `viewmodel_transform` and the third-person hand transform in `re2`) — which flips
/// every triangle's winding on screen. Backface culling would then throw away the outside of the
/// meshes and draw their insides, so each weapon's parts are pre-flipped by this to cancel the mirror.
pub fn flip_winding_for_viewmodel(parts: &mut [HeldPart]) {
    for part in parts {
        for tri in part.mesh.indices.as_chunks_mut::<3>().0 {
            tri.swap(1, 2);
        }
    }
}

/// Every held weapon's parts (bat first, then the revolver), ready to upload.
pub fn build_all_held_parts() -> Vec<HeldPart> {
    let mut parts = build_held_parts();
    parts.extend(crate::revolver::build_revolver_parts());
    for weapon in Weapon::FIREARMS.into_iter().filter(|w| *w != Weapon::Revolver) {
        parts.extend(crate::firearms::build_firearm_parts(weapon));
    }
    parts
}

/// Appends `src`'s vertices/indices into `dst`, transformed by `transform` — the same
/// "combine primitives placed by local transforms" approach `humanoid` uses for its capsule rig,
/// but done directly (a viewmodel isn't a scene object, so it has no `schema`/`skeleton` node of
/// its own to hang a `group` off of).
pub(crate) fn append_transformed(dst: &mut Mesh, src: &Mesh, transform: Mat4) {
    let normal_mat = transform.inverse().transpose();
    let base = dst.vertices.len() as u32;
    for v in &src.vertices {
        let p = transform.transform_point3(Vec3::from_array(v.pos));
        let n = normal_mat.transform_vector3(Vec3::from_array(v.normal)).normalize_or_zero();
        dst.vertices.push(Vertex { pos: p.to_array(), normal: n.to_array() });
    }
    dst.indices.extend(src.indices.iter().map(|&i| base + i));
}

/// Surface of revolution about local `+Z`: `profile` is `(z, radius)` pairs from one end to the
/// other (radius 0 at both ends closes the shape). Smooth normals come from the profile's slope.
/// Winding is CCW seen from outside (`mesh::tests`-style check in `held_tests`).
pub(crate) fn lathe(profile: &[(f32, f32)], segments: u32) -> Mesh {
    let mut m = Mesh::default();
    let n = profile.len();
    for (i, &(z, r)) in profile.iter().enumerate() {
        let (p0, p1) = (profile[i.saturating_sub(1)], profile[(i + 1).min(n - 1)]);
        let (dz, dr) = (p1.0 - p0.0, p1.1 - p0.1);
        let len = (dz * dz + dr * dr).sqrt().max(1e-6);
        let (nr, nz) = (dz / len, -dr / len);
        for s in 0..=segments {
            let a = s as f32 / segments as f32 * std::f32::consts::TAU;
            let (sn, cs) = a.sin_cos();
            m.vertices.push(Vertex { pos: [r * cs, r * sn, z], normal: Vec3::new(cs * nr, sn * nr, nz).normalize_or_zero().to_array() });
        }
    }
    let row = segments + 1;
    for i in 0..(n as u32 - 1) {
        for s in 0..segments {
            let (a, b) = (i * row + s, i * row + s + 1);
            let (c, d) = ((i + 1) * row + s, (i + 1) * row + s + 1);
            m.indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    m
}

/// The held item: a wooden baseball bat gripped by a fist, with a sleeve/forearm leaving toward
/// the player (first person only). Everything is in the viewmodel's local frame: the grip at the
/// origin, the bat extending toward `+Z` (knob at `-Z`), matching [`viewmodel_transform`].
pub fn build_held_parts() -> Vec<HeldPart> {
    // Bat: knob, thin handle, gradual taper to the barrel, rounded end cap (about 0.81 m long,
    // roughly a 32" bat at this scale).
    let bat_profile: Vec<(f32, f32)> = [
        (-0.100, 0.0),
        (-0.098, 0.017),
        (-0.090, 0.0235),
        (-0.080, 0.0255),
        (-0.068, 0.0215),
        (-0.056, 0.0160),
        (-0.040, 0.0138),
        (0.000, 0.0135),
        (0.100, 0.0142),
        (0.200, 0.0168),
        (0.300, 0.0215),
        (0.400, 0.0282),
        (0.480, 0.0330),
        (0.560, 0.0355),
        (0.630, 0.0352),
        (0.680, 0.0330),
        (0.705, 0.0270),
        (0.718, 0.0180),
        (0.723, 0.0),
    ]
    .iter()
    .map(|&(z, r)| (z, r * BAT_GIRTH))
    .collect();
    let bat = lathe(&bat_profile, 24);

    // ---- Hand -------------------------------------------------------------------------------
    // Built in a *hand frame* whose Z is the bat's axis: a fist blob around the handle, a thumb
    // bump, and the wrist leaving the -Y side. The whole hand is then rolled about the bat's axis
    // (`roll`) so the forearm leaves toward the bottom of the screen.
    let handle_r = 0.0135_f32 * BAT_GIRTH;
    let rot_z = |deg: f32| Mat4::from_rotation_z(deg.to_radians());
    let wrist_dir = Vec3::new(0.25, -0.50, -0.83).normalize();

    // Idle pose: bat-local -> camera-local. Pick the roll that sends the forearm to the lower right.
    let idle = Mat4::from_rotation_z(IDLE_ROLL_DEG.to_radians()) * Mat4::from_rotation_x(IDLE_PITCH_DEG.to_radians());
    let target = Vec3::new(0.35, -0.90, -0.40).normalize();
    let mut roll = 0.0_f32;
    let mut best = f32::MIN;
    for d in 0..360 {
        let dir_cam = idle.transform_vector3(rot_z(d as f32).transform_vector3(wrist_dir));
        let score = dir_cam.dot(target);
        if score > best {
            best = score;
            roll = d as f32;
        }
    }
    // Hand sits low on the handle, its little finger just above the knob.
    let to_bat = Mat4::from_translation(Vec3::new(0.0, 0.0, -0.030)) * rot_z(roll);

    let mut hand = Mesh::default();
    let mut sleeve = Mesh::default();
    let capsule_between = |mesh: &mut Mesh, a: Vec3, b: Vec3, r: f32| {
        let d = b - a;
        let len = d.length();
        if len < 1e-5 {
            return;
        }
        let cap = Mesh::capsule(r, len + 2.0 * r, 10, 4);
        append_transformed(mesh, &cap, to_bat * Mat4::from_translation((a + b) * 0.5) * Mat4::from_quat(Quat::from_rotation_arc(Vec3::Y, d / len)));
    };
    let ellipsoid = |mesh: &mut Mesh, c: Vec3, radii: Vec3| {
        append_transformed(mesh, &Mesh::uv_sphere(1.0, 8, 12), to_bat * Mat4::from_translation(c) * Mat4::from_scale(radii));
    };

    // Fist: one soft, slightly flattened blob wrapped around the handle, with a thumb bump. Kept
    // deliberately simple: at viewmodel size a plain rounded fist reads better than fiddly fingers.
    ellipsoid(&mut hand, Vec3::new(0.004, 0.0, 0.008), Vec3::new(handle_r + 0.019, handle_r + 0.017, 0.054));
    ellipsoid(&mut hand, Vec3::new(-0.021, -0.021, 0.040), Vec3::new(0.013, 0.013, 0.022));

    // Wrist (skin) and the sleeve/forearm leaving along `wrist_dir` (first person only).
    let w0 = Vec3::new(0.012, -0.030, 0.0);
    capsule_between(&mut hand, w0, w0 + wrist_dir * 0.035, 0.0195);
    let arm_dir = to_bat.transform_vector3(wrist_dir).normalize();
    let arm_rot = Quat::from_rotation_arc(Vec3::Y, arm_dir);
    let start = to_bat.transform_point3(w0);
    let along = |from: f32, len: f32| Mat4::from_translation(start + arm_dir * (from + len * 0.5)) * Mat4::from_quat(arm_rot);
    append_transformed(&mut sleeve, &Mesh::cylinder(0.038, 0.035, 14), along(0.040, 0.035));
    append_transformed(&mut sleeve, &Mesh::cylinder(0.030, 0.60, 14), along(0.070, 0.60));

    let mut parts = vec![
        HeldPart::lit(Weapon::Bat, bat, BAT_COLOR, 0.0, 0.32, false),
        HeldPart::lit(Weapon::Bat, hand, HAND_COLOR, 0.0, 0.5, false),
        HeldPart::lit(Weapon::Bat, sleeve, SLEEVE_COLOR, 0.0, 0.85, true),
    ];
    // Pre-flip the winding to cancel the viewmodel basis' mirror (see `flip_winding_for_viewmodel`).
    flip_winding_for_viewmodel(&mut parts);
    parts
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
        // The world depth buffer is also sampled by the clarity post pass, hence TEXTURE_BINDING.
        let make_depth = |label| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: MSAA_SAMPLES,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Depth32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
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
            // bat always draws on top of the world instead of clipping into a nearby wall —
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
    /// Object ancestry for each scene mesh, in exactly the same order as `meshes`.
    mesh_object_paths: Vec<Vec<String>>,
    /// Scene object ids suppressed by a game rule or application.
    hidden_objects: HashSet<String>,
    held: Vec<HeldGpu>,
    crosshair: CrosshairPipeline,
    crosshair_buf: wgpu::Buffer,
    crosshair_bind_group: wgpu::BindGroup,
    post: PostFx,
    post_bind_group: wgpu::BindGroup,
    /// A 2-D image drawn over the finished frame (the launch menu); hidden during play.
    pub overlay: Overlay,
}

/// A held-weapon piece on the GPU.
struct HeldGpu {
    mesh: GpuMesh,
    color: Vec3,
    metallic: f32,
    roughness: f32,
    fp_only: bool,
    weapon: Weapon,
    emissive: Vec3,
    flash: bool,
}

/// Which optional layers [`LiveRenderer::render_ex`] draws on top of the world.
#[derive(Clone, Copy)]
pub struct FrameOptions {
    /// The aim reticle.
    pub crosshair: bool,
    /// The first-person held bat/fist (the third-person copy is always drawn).
    pub viewmodel: bool,
    /// The crosshair shows the green "you can pick that up" state (wins over the gold bat-target one).
    pub pickup: bool,
    /// Which weapon's pieces are drawn (first-person viewmodel and third-person hand copy).
    pub weapon: Weapon,
    /// Muzzle-flash brightness, 0 (off) .. 1 (full); the flash piece is drawn only when > 0.
    pub muzzle_flash: f32,
}

impl Default for FrameOptions {
    fn default() -> Self {
        FrameOptions { crosshair: true, viewmodel: true, pickup: false, weapon: Weapon::Bat, muzzle_flash: 0.0 }
    }
}

impl LiveRenderer {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, scene: &Scene, width: u32, height: u32) -> Self {
        let pipelines = create_pipelines(device, color_format, MSAA_SAMPLES);
        let shadow_sampler = make_shadow_sampler(device);
        let targets = LiveTargets::new(device, color_format, width, height);

        let mut raw_meshes = Vec::new();
        collect_leaf_meshes(&scene.objects, &mut raw_meshes);
        let mut mesh_object_paths = Vec::new();
        collect_leaf_object_paths(&scene.objects, &[], &mut mesh_object_paths);
        debug_assert_eq!(raw_meshes.len(), mesh_object_paths.len());
        let meshes: Vec<GpuMesh> = raw_meshes.iter().map(|m| GpuMesh::upload(device, m)).collect();
        let held: Vec<HeldGpu> = build_all_held_parts()
            .into_iter()
            .map(|p| HeldGpu {
                mesh: GpuMesh::upload(device, &p.mesh),
                color: p.color,
                metallic: p.metallic,
                roughness: p.roughness,
                fp_only: p.first_person_only,
                weapon: p.weapon,
                emissive: p.emissive,
                flash: p.muzzle_flash,
            })
            .collect();
        // Two extra slots in the shared object-uniform buffer: one for the camera-attached
        // first-person viewmodel (drawn in its own always-on-top pass), one for a second
        // instance of the same bat mesh rigidly attached to the third-person body's hand
        // bone (drawn as an ordinary world object, shadowed/occluded like any prop). Both are
        // written and bound (via a dynamic offset) alongside the scene meshes each frame.
        let draw_count = meshes.len() as u64 + 2 * held.len() as u64;

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

        let post = create_post_pipeline(device, color_format, MSAA_SAMPLES);
        let post_bind_group = post.bind(device, &targets.depth_view);
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
            mesh_object_paths,
            hidden_objects: HashSet::new(),
            held,
            crosshair,
            crosshair_buf,
            crosshair_bind_group,
            post,
            post_bind_group,
            overlay: Overlay::new(device, color_format),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == self.targets.width && height == self.targets.height {
            return;
        }
        self.targets = LiveTargets::new(device, self.color_format, width, height);
        // The depth texture was recreated, so the post pass's bind group must point at the new one.
        self.post_bind_group = self.post.bind(device, &self.targets.depth_view);
        // So was the shadow map: the main pass must sample the texture the shadow pass now writes.
        // (Left pointing at the old one, it read a never-written all-zero map and every surface
        // was in the sun's shadow — the whole map went dark after the window's first resize.)
        self.global_bind_group_full = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("live-global-bind-group-full"),
            layout: &self.pipelines.layouts.global_full,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.global_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.targets.shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&make_shadow_sampler(device)) },
            ],
        });
    }

    /// Replaces the set of scene object ids omitted from both the colour and shadow passes.
    /// Hiding a group also hides every descendant mesh. This is intentionally renderer state:
    /// it does not mutate authored transforms or collision, and accepts the output of
    /// [`crate::sim::rules_run::RulesEngine::hidden`] directly.
    pub fn set_hidden_objects<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        self.hidden_objects.clear();
        self.hidden_objects.extend(ids.into_iter().map(str::to_owned));
    }

    /// Renders one frame: `t` is the scene animation time (seconds, for any keyframed objects
    /// in the room — the camera itself is not part of the scene here), `camera` is the player's
    /// current view, `target_view` is the swapchain frame to draw into. `crosshair_highlighted`
    /// switches the aim reticle to its "something's in reach" color. `weapon_transform` is the
    /// camera-attached first-person bat's world transform (see [`viewmodel_transform`]),
    /// drawn last in its own depth space so it never clips into world geometry.
    /// `hand_prop_transform` is a second instance of the same bat mesh, rigidly attached to
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
        self.render_ex(device, queue, scene, t, camera, target_view, crosshair_highlighted, weapon_transform, hand_prop_transform, FrameOptions::default());
    }

    /// [`render`](Self::render) with control over the optional layers (`opts`), and it also draws
    /// `self.overlay` last when one is set.
    pub fn render_ex(
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
        opts: FrameOptions,
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
        let light_planes = shadow_active.then(|| frustum_planes(Mat4::from_cols_array_2d(&globals.light_view_proj)));
        let mut main_visible = Vec::with_capacity(self.meshes.len());
        let mut shadow_visible = Vec::with_capacity(self.meshes.len());
        for (i, mesh) in self.meshes.iter().enumerate() {
            let shown = !self.mesh_object_paths[i].iter().any(|id| self.hidden_objects.contains(id));
            let (center, half) = world_aabb(transforms[i].0, mesh.local_min, mesh.local_max);
            main_visible.push(shown && !aabb_outside_frustum(center, half, &cam_planes));
            shadow_visible.push(match &light_planes {
                Some(planes) => shown && !aabb_outside_frustum(center, half, planes),
                None => false,
            });
        }

        // Every object's uniform data is staged into one contiguous byte buffer and uploaded
        // with a single `write_buffer` call instead of one call per mesh — object_stride is
        // alignment-padded past ObjectUniform's own size, so the staging buffer is built at full
        // stride width and each uniform's bytes are copied into its slot, padding left as-is.
        // Each held part owns two slots: `[first-person, third-person]`.
        let held_slot = |k: usize, third: bool| self.meshes.len() as u64 + 2 * k as u64 + third as u64;
        let mut object_data = vec![0u8; (self.object_stride * (self.meshes.len() as u64 + 2 * self.held.len() as u64)) as usize];
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

        let held_uniform = |world: Mat4, h: &HeldGpu, glow: f32| {
            let normal_mat = world.inverse().transpose();
            let e = h.emissive * glow;
            ObjectUniform {
                model: world.to_cols_array_2d(),
                normal_mat: normal_mat.to_cols_array_2d(),
                base_color: [h.color.x, h.color.y, h.color.z, 1.0],
                material: [h.metallic, h.roughness, 0.0, 0.0],
                emissive: [e.x, e.y, e.z, 0.0],
            }
        };
        // Only the active weapon's pieces are drawn (a muzzle flash only while it is up).
        let held_visible: Vec<bool> = self.held.iter().map(|h| h.weapon == opts.weapon && (!h.flash || opts.muzzle_flash > 0.0)).collect();
        for (k, h) in self.held.iter().enumerate() {
            let glow = if h.flash { opts.muzzle_flash } else { 1.0 };
            stage(&mut object_data, held_slot(k, false), self.object_stride, &held_uniform(weapon_transform, h, glow));
            stage(&mut object_data, held_slot(k, true), self.object_stride, &held_uniform(hand_prop_transform, h, glow));
        }
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
            // The hand-held (third-person) bat is an ordinary world object, so it casts a
            // shadow like any other prop — unlike the always-on-top first-person viewmodel.
            for (k, h) in self.held.iter().enumerate() {
                if h.fp_only || h.flash || !held_visible[k] {
                    continue;
                }
                shadow_pass.set_bind_group(1, &self.object_bind_group, &[(held_slot(k, true) * self.object_stride) as u32]);
                shadow_pass.set_vertex_buffer(0, h.mesh.vertex_buf.slice(..));
                shadow_pass.set_index_buffer(h.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.draw_indexed(0..h.mesh.index_count, 0, 0..1);
            }
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
            // Hand-held (third-person) bat instance: normal depth test against the world,
            // so a wall between the camera and the player correctly occludes it like any prop.
            for (k, h) in self.held.iter().enumerate() {
                if h.fp_only || !held_visible[k] {
                    continue;
                }
                main_pass.set_bind_group(1, &self.object_bind_group, &[(held_slot(k, true) * self.object_stride) as u32]);
                main_pass.set_vertex_buffer(0, h.mesh.vertex_buf.slice(..));
                main_pass.set_index_buffer(h.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                main_pass.draw_indexed(0..h.mesh.index_count, 0, 0..1);
            }
        }

        {
            // Clarity pass: contact AO + silhouette outlines from the world depth, multiplied
            // into the lit color. Runs after the world and *before* the viewmodel pass so the
            // held bat (its own depth space) is never darkened by world geometry.
            let (w, h) = (self.targets.width.max(1), self.targets.height.max(1));
            let edge_px = (1.5 * h as f32 / 1080.0).max(1.0);
            let uniform = post_uniform(&scene.post, camera.near, camera.far, camera.fov_deg, w, h, edge_px);
            queue.write_buffer(&self.post.uniform_buf, 0, bytemuck::bytes_of(&uniform));
            let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("live-post-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.multisampled_color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            post_pass.set_pipeline(&self.post.pipeline);
            post_pass.set_bind_group(0, &self.post_bind_group, &[]);
            post_pass.draw(0..3, 0..1);
        }

        {
            // Fresh depth clear (not `self.targets.depth_view`, which still holds the world's
            // depth) so the bat always draws over the world, matching how a first-person
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
            for (k, h) in self.held.iter().enumerate() {
                if !opts.viewmodel {
                    break;
                }
                if !held_visible[k] {
                    continue;
                }
                vm_pass.set_bind_group(1, &self.object_bind_group, &[(held_slot(k, false) * self.object_stride) as u32]);
                vm_pass.set_vertex_buffer(0, h.mesh.vertex_buf.slice(..));
                vm_pass.set_index_buffer(h.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                vm_pass.draw_indexed(0..h.mesh.index_count, 0, 0..1);
            }
        }

        let crosshair_color = if opts.pickup {
            [0.35, 1.0, 0.45, 1.0]
        } else if crosshair_highlighted {
            [1.0, 0.85, 0.2, 1.0]
        } else {
            [1.0, 1.0, 1.0, 0.85]
        };
        let crosshair_uniform =
            CrosshairUniform { color: crosshair_color, to_ndc: [2.0 / self.targets.width.max(1) as f32, 2.0 / self.targets.height.max(1) as f32, 0.0, 0.0] };
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
            if opts.crosshair {
                crosshair_pass.set_pipeline(&self.crosshair.pipeline);
                crosshair_pass.set_bind_group(0, &self.crosshair_bind_group, &[]);
                crosshair_pass.draw(0..12, 0..1);
            }
            self.overlay.draw(&mut crosshair_pass);
        }

        queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod held_tests {
    use super::*;

    /// The held meshes are drawn through a mirroring basis (see `build_held_parts`), so their
    /// winding must *disagree* with the stored (outward) vertex normals in mesh space — after the
    /// mirror they agree, and backface culling keeps the outside. If this fails the bat renders
    /// inside-out (its inner wall showing through the fist).
    #[test]
    fn held_parts_are_wound_for_the_mirrored_viewmodel_basis() {
        for (i, part) in build_all_held_parts().iter().enumerate() {
            let m = &part.mesh;
            let (mut ok, mut bad) = (0, 0);
            for t in m.indices.chunks(3) {
                let v: Vec<Vec3> = t.iter().map(|&i| Vec3::from_array(m.vertices[i as usize].pos)).collect();
                let wn = (v[1] - v[0]).cross(v[2] - v[0]);
                if wn.length() < 1e-9 {
                    continue; // degenerate pole triangles
                }
                let sn: Vec3 = t.iter().map(|&i| Vec3::from_array(m.vertices[i as usize].normal)).sum();
                if wn.dot(sn) < 0.0 {
                    ok += 1;
                } else {
                    bad += 1;
                }
            }
            assert!(ok > 0 && bad == 0, "held part {i}: {bad} triangles not pre-flipped for the mirrored basis ({ok} fine)");
        }
    }

    #[test]
    fn the_bat_is_a_believable_length_and_the_grip_is_at_the_origin() {
        let parts = build_held_parts();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for v in &parts[0].mesh.vertices {
            lo = lo.min(v.pos[2]);
            hi = hi.max(v.pos[2]);
        }
        assert!((0.75..0.9).contains(&(hi - lo)), "bat length {}", hi - lo);
        assert!(lo < -0.05 && lo > -0.15, "knob sits just behind the grip: {lo}");
    }
}
