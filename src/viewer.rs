//! Real-time rendering: the "Red Engine 2" viewer.
//!
//! This reuses the offline engine's scene schema, mesh generation, and shader pipelines
//! (see [`crate::render`] / [`crate::gpu`]) but draws directly into a window's swapchain
//! surface every frame instead of an offscreen texture read back to PNG/MP4, and the camera
//! is driven by the application instead of the scene's `camera` track.
//!
//! Two layers: [`LiveRenderer::render_view`] draws the world (shadows, lighting, clarity pass, rule-hidden objects, the
//! 2-D overlay) from any [`ViewCamera`] — what a custom client uses (`crate::app`). The first-person game adds its
//! optional [`FpsLayers`] on top (held weapon viewmodel, third-person hand copy, crosshair) through
//! [`LiveRenderer::render_ex`]; a renderer built with [`LiveRenderer::world`] never loads the weapon meshes at all.

use crate::avatar::RemoteHand;
use crate::feel::FxParams;
use crate::fx::FxPipeline;
use crate::gpu::{
    create_crosshair_pipeline, create_pipelines, create_post_pipeline, make_shadow_sampler, post_uniform, CrosshairPipeline, CrosshairUniform, GlobalUniform,
    GpuMesh, ObjectUniform, Pipelines, PostFx, MSAA_SAMPLES, SHADOW_SIZE,
};
use crate::mesh::{Mesh, Vertex};
use crate::object_staging::{frustum_planes, SceneStaging};
use crate::overlay::Overlay;
use crate::render::{build_globals_common, collect_leaf_meshes, collect_leaf_object_paths};
use crate::schema::Scene;
use crate::weapons::Weapon;
use glam::{Mat4, Quat, Vec3, Vec4};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// Other players' weapons drawn at once (one per possible remote player).
pub const REMOTE_HANDS: usize = crate::sim::match_sim::MAX_PLAYERS;

/// The world transform of the weapon a remote player holds: at their wrist, pointing the way they look (a firearm's grip sits in the hand and
/// kicks up with recoil), or the bat at the angle of its swing. The same construction as the local player's third-person hand prop.
pub fn remote_hand_transform(h: &RemoteHand) -> Mat4 {
    let (sy, cy) = h.yaw.sin_cos();
    let basis = Mat4::from_cols(Vec4::new(cy, 0.0, sy, 0.0), Vec4::Y, Vec4::new(sy, 0.0, -cy, 0.0), Vec4::new(0.0, 0.0, 0.0, 1.0));
    if h.weapon.is_firearm() {
        Mat4::from_translation(h.wrist)
            * basis
            * Mat4::from_rotation_x(-h.pitch - 0.30 * h.kick)
            * Mat4::from_translation(-crate::firearms::grip_anchor(h.weapon))
    } else if h.weapon == Weapon::Knife {
        Mat4::from_translation(h.wrist)
            * basis
            * Mat4::from_rotation_z(18.0f32.to_radians())
            * Mat4::from_rotation_x(h.bat_pitch_deg.to_radians())
            * Mat4::from_translation(-crate::firearms::grip_anchor(h.weapon))
    } else {
        Mat4::from_translation(h.wrist) * basis * Mat4::from_rotation_z(IDLE_ROLL_DEG.to_radians()) * Mat4::from_rotation_x(h.bat_pitch_deg.to_radians())
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

/// The first-person camera policy lives with the other cameras in [`crate::app::camera`] (graphics-free); re-exported
/// here where `re2` and the examples have always found it.
pub use crate::app::camera::{FpsCamera, ViewCamera};

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
pub const IDLE_PITCH_DEG: f32 = crate::avatar::BAT_IDLE_PITCH_DEG;
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
    /// Which team look this piece is for (`uniforms::by_skin`): [`ANY_SKIN`] is drawn for everyone; `0` (no team), `1` and `2` only while the
    /// wearer has that look, so a player's own hands and sleeves match their uniform.
    pub skin: u8,
}

/// [`HeldPart::skin`] for pieces every wearer sees (the gun itself, the muzzle flash).
pub const ANY_SKIN: u8 = u8::MAX;

impl HeldPart {
    /// An ordinary lit piece of `weapon` (no glow, not a flash).
    pub fn lit(weapon: Weapon, mesh: Mesh, color: Vec3, metallic: f32, roughness: f32, first_person_only: bool) -> HeldPart {
        HeldPart { mesh, color, metallic, roughness, first_person_only, weapon, emissive: Vec3::ZERO, muzzle_flash: false, skin: ANY_SKIN }
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

/// The hand and forearm pieces of a held weapon once per hand look (see [`HeldPart::skin`]): the same meshes in each team's glove and sleeve
/// colours, so the arms a player sees on themselves are the arms everyone else sees on them.
pub fn skinned_hands(weapon: Weapon, hand: &Mesh, sleeve: &Mesh) -> Vec<HeldPart> {
    let mut out = Vec::new();
    for skin in crate::firearms::SKINS {
        let (sleeve_color, hand_color) = crate::firearms::skin_colors(skin);
        out.push(HeldPart { skin, ..HeldPart::lit(weapon, hand.clone(), hand_color, 0.0, 0.55, false) });
        out.push(HeldPart { skin, ..HeldPart::lit(weapon, sleeve.clone(), sleeve_color, 0.0, 0.85, true) });
    }
    out
}

/// Every held weapon's parts (the bat first, then each firearm of [`Weapon::FIREARMS`]), ready to upload.
pub fn build_all_held_parts() -> Vec<HeldPart> {
    let mut parts = build_held_parts();
    for weapon in Weapon::FIREARMS {
        parts.extend(crate::firearms::build_firearm_parts(weapon));
    }
    parts
}

/// The parts of every weapon of the loadout arsenal ([`Weapon::ROSTER`]) — what a loadout match's renderer uploads.
pub fn build_all_roster_parts() -> Vec<HeldPart> {
    let mut parts = build_held_parts();
    for weapon in Weapon::ROSTER {
        match weapon {
            Weapon::Bat => {}
            w if w.is_gun() => parts.extend(crate::firearms::build_firearm_parts(w)),
            w => parts.extend(crate::firearms::build_thrown_and_melee_parts(w)),
        }
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
        dst.vertices.push(Vertex::new(p.to_array(), n.to_array()));
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
            m.vertices.push(Vertex::new([r * cs, r * sn, z], Vec3::new(cs * nr, sn * nr, nz).normalize_or_zero().to_array()));
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

    let mut parts = vec![HeldPart::lit(Weapon::Bat, bat, BAT_COLOR, 0.0, 0.32, false)];
    parts.extend(skinned_hands(Weapon::Bat, &hand, &sleeve));
    // Pre-flip the winding to cancel the viewmodel basis' mirror (see `flip_winding_for_viewmodel`).
    flip_winding_for_viewmodel(&mut parts);
    parts
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
    /// Each mesh's local AABB (from `meshes`, laid out flat for the per-frame bounds refresh).
    local_bounds: Vec<(Vec3, Vec3)>,
    /// The persistent object-uniform staging copy: only slots whose inputs changed are rewritten and uploaded.
    staging: SceneStaging,
    /// Scratch for the coalesced upload ranges.
    upload_ranges: Vec<std::ops::Range<usize>>,
    /// Object ancestry for each scene mesh, in exactly the same order as `meshes`.
    mesh_object_paths: Vec<Vec<String>>,
    /// The scene's ocean, when it has one (drawn last in the main pass; see [`crate::ocean_pass`]).
    ocean: Option<crate::ocean_pass::OceanPass>,
    /// The scene's endless generated world, when it has a `procgen` block: chunks stream in around the camera.
    stream: Option<crate::stream_gpu::StreamLayer>,
    /// Every local viewer's eye, when several views are drawn in a frame (split-screen): the streamed world follows all of them.
    stream_eyes: Vec<Vec3>,
    /// Drives the water's animation (it must not loop with the scene's `duration`).
    clock: std::time::Instant,
    /// Scene object ids suppressed by a game rule or application.
    hidden_objects: HashSet<String>,
    /// Whether each scene mesh is under a hidden object (same order as `meshes`; recomputed only when the hidden set changes).
    mesh_hidden: Vec<bool>,
    held: Vec<HeldGpu>,
    /// The weapons other players hold this frame (set with [`LiveRenderer::set_remote_hands`]).
    remote_hands: Vec<RemoteHand>,
    crosshair: CrosshairPipeline,
    crosshair_buf: wgpu::Buffer,
    crosshair_bind_group: wgpu::BindGroup,
    post: PostFx,
    post_bind_group: wgpu::BindGroup,
    /// Damage vignette, flashes and the hit marker, drawn over the finished frame.
    fx: FxPipeline,
    /// A 2-D image drawn over the finished frame (the connect form, the pause menu, the lobby); hidden during play.
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
    skin: u8,
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
    /// Screen effects over the frame: damage vignette and arc, flashes, the hit marker (nothing by default).
    pub fx: FxParams,
    /// An enemy is under the crosshair: it turns red (over the pick-up green and the gold "in reach").
    pub enemy: bool,
    /// The hand look of the local player (`uniforms::by_skin`): `0` when they have no team.
    pub skin: u8,
}

impl Default for FrameOptions {
    fn default() -> Self {
        FrameOptions { crosshair: true, viewmodel: true, pickup: false, weapon: Weapon::Bat, muzzle_flash: 0.0, fx: FxParams::default(), enemy: false, skin: 0 }
    }
}

/// The first-person game's layers over the world (see [`LiveRenderer::render_ex`] for what each one is).
#[derive(Clone, Copy)]
pub struct FpsLayers {
    /// The crosshair shows its gold "something is in reach" state.
    pub crosshair_highlighted: bool,
    /// World transform of the camera-attached viewmodel ([`viewmodel_transform`]).
    pub weapon_transform: Mat4,
    /// World transform of the weapon copy held by the third-person body's hand.
    pub hand_prop_transform: Mat4,
    /// Which optional layers to draw.
    pub opts: FrameOptions,
}

impl LiveRenderer {
    /// A renderer for the first-person game: the world plus every held weapon's meshes (see [`FpsLayers`]).
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, scene: &Scene, width: u32, height: u32) -> Self {
        Self::build(device, color_format, scene, width, height, build_all_held_parts())
    }

    /// [`new`](Self::new) for a loadout match: every weapon of [`Weapon::ROSTER`] (and both teams' hands) is uploaded, not just the prototype's ten guns.
    pub fn new_loadout(device: &wgpu::Device, color_format: wgpu::TextureFormat, scene: &Scene, width: u32, height: u32) -> Self {
        Self::build(device, color_format, scene, width, height, build_all_roster_parts())
    }

    /// A renderer for the world alone (any camera, no weapons, no crosshair): what a custom client draws with through
    /// [`Self::render_view`]. The scene's leaf objects are uploaded once, here; afterwards move, re-colour or hide them
    /// (their tracks are sampled every frame), but adding or removing objects needs a new renderer.
    pub fn world(device: &wgpu::Device, color_format: wgpu::TextureFormat, scene: &Scene, width: u32, height: u32) -> Self {
        Self::build(device, color_format, scene, width, height, Vec::new())
    }

    fn build(device: &wgpu::Device, color_format: wgpu::TextureFormat, scene: &Scene, width: u32, height: u32, held_parts: Vec<HeldPart>) -> Self {
        let pipelines = create_pipelines(device, color_format, MSAA_SAMPLES);
        let shadow_sampler = make_shadow_sampler(device);
        let targets = LiveTargets::new(device, color_format, width, height);

        let mut raw_meshes = Vec::new();
        collect_leaf_meshes(&scene.objects, &mut raw_meshes);
        let mut mesh_object_paths = Vec::new();
        collect_leaf_object_paths(&scene.objects, &[], &mut mesh_object_paths);
        debug_assert_eq!(raw_meshes.len(), mesh_object_paths.len());
        // Identical geometry (every crate of a kind, every same-size box) shares one pair of GPU buffers: matched by exact vertex/index bytes,
        // so a shared mesh is indistinguishable from a separate one. Draw calls, per-object uniforms and culling are unchanged.
        let mut seen: HashMap<u64, Vec<(usize, GpuMesh)>> = HashMap::new();
        let mut meshes: Vec<GpuMesh> = Vec::with_capacity(raw_meshes.len());
        for (i, m) in raw_meshes.iter().enumerate() {
            let (vb, ib): (&[u8], &[u8]) = (bytemuck::cast_slice(&m.vertices), bytemuck::cast_slice(&m.indices));
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (vb, ib).hash(&mut h);
            let bucket = seen.entry(h.finish()).or_default();
            let same = bucket.iter().find(|(j, _)| {
                let o = &raw_meshes[*j];
                bytemuck::cast_slice::<_, u8>(&o.vertices) == vb && bytemuck::cast_slice::<_, u8>(&o.indices) == ib
            });
            match same {
                Some((_, g)) => meshes.push(g.clone()),
                None => {
                    let g = GpuMesh::upload(device, m);
                    bucket.push((i, g.clone()));
                    meshes.push(g);
                }
            }
        }
        let held: Vec<HeldGpu> = held_parts
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
                skin: p.skin,
            })
            .collect();
        // Two extra slots in the shared object-uniform buffer: one for the camera-attached
        // first-person viewmodel (drawn in its own always-on-top pass), one for a second
        // instance of the same bat mesh rigidly attached to the third-person body's hand
        // bone (drawn as an ordinary world object, shadowed/occluded like any prop). Both are
        // written and bound (via a dynamic offset) alongside the scene meshes each frame.
        let images = crate::render::wrap_offsets(scene).len();
        let draw_count = ((meshes.len() * images) as u64 + (2 + REMOTE_HANDS as u64) * held.len() as u64).max(1);

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
        let ocean = crate::ocean_pass::OceanPass::new(device, color_format, MSAA_SAMPLES, scene, &pipelines.layouts.global_uniform);
        let stream = scene.procgen.clone().map(|cfg| {
            let workers = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 3));
            crate::stream_gpu::StreamLayer::new(device, &pipelines.layouts.object, cfg, crate::procgen::View::default(), workers)
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

        let local_bounds = meshes.iter().map(|m| (m.local_min, m.local_max)).collect();
        let staging = SceneStaging::new(meshes.len(), images, (2 + REMOTE_HANDS) * held.len(), object_stride);
        LiveRenderer {
            local_bounds,
            staging,
            upload_ranges: Vec::new(),
            color_format,
            pipelines,
            global_buf,
            global_bind_group_uniform,
            global_bind_group_full,
            object_buf,
            object_stride,
            object_bind_group,
            targets,
            mesh_hidden: vec![false; meshes.len()],
            meshes,
            mesh_object_paths,
            ocean,
            stream,
            stream_eyes: Vec::new(),
            clock: std::time::Instant::now(),
            hidden_objects: HashSet::new(),
            held,
            remote_hands: Vec::new(),
            crosshair,
            crosshair_buf,
            crosshair_bind_group,
            post,
            post_bind_group,
            fx: FxPipeline::new(device, color_format),
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

    /// Sets the weapons other players hold this frame (at most [`REMOTE_HANDS`] are drawn): each is drawn like the third-person copy of ours,
    /// with its muzzle flash while `flash` is above zero.
    pub fn set_remote_hands(&mut self, hands: &[RemoteHand]) {
        self.remote_hands.clear();
        self.remote_hands.extend(hands.iter().copied().take(REMOTE_HANDS));
    }

    /// Replaces the set of scene object ids omitted from both the colour and shadow passes.
    /// Hiding a group also hides every descendant mesh. This is intentionally renderer state:
    /// it does not mutate authored transforms or collision, and accepts the output of
    /// [`crate::sim::rules_run::RulesEngine::hidden`] directly. It is the way to switch a pooled object off ([`crate::scene_pool::ScenePool::hidden_ids`]): a mesh
    /// scaled to a speck is still a draw call in both passes, a hidden one is none. Cheap to call every frame: an unchanged set costs no allocation and no
    /// recomputation.
    /// Builds and uploads every chunk of a streamed (`procgen`) world around `eye` before returning, so a picture shows all of it. A no-op without one.
    pub fn settle_stream(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, eye: Vec3) {
        if let Some(stream) = &mut self.stream {
            if self.stream_eyes.is_empty() {
                stream.fill(device, queue, &[eye]);
            } else {
                stream.fill(device, queue, &self.stream_eyes);
            }
        }
    }

    /// Tells the streamed world where every viewer of this frame is (split-screen draws several views; each needs its ground). Empty means one viewer, the camera.
    pub fn set_stream_eyes(&mut self, eyes: &[Vec3]) {
        self.stream_eyes.clear();
        self.stream_eyes.extend_from_slice(eyes);
    }

    /// What the streamed world drew in the last frame (chunks resident, draw calls, triangles), if the scene has one.
    pub fn stream_draw_stats(&self) -> Option<crate::stream_gpu::DrawStats> {
        self.stream.as_ref().map(|s| s.draw_stats())
    }

    /// How far the streamed world reaches, metres (split-screen draws several views per frame, so each sees a little less far).
    pub fn set_view_distance(&mut self, metres: f32) {
        if let Some(s) = &mut self.stream {
            s.set_view_distance(metres);
        }
    }

    pub fn set_hidden_objects<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        let mut ids: Vec<&str> = ids.into_iter().collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() == self.hidden_objects.len() && ids.iter().all(|id| self.hidden_objects.contains(*id)) {
            return;
        }
        self.hidden_objects.clear();
        self.hidden_objects.extend(ids.into_iter().map(str::to_owned));
        let hidden = &self.hidden_objects;
        self.mesh_hidden = self.mesh_object_paths.iter().map(|path| path.iter().any(|id| hidden.contains(id))).collect();
    }

    /// How many scene meshes are skipped because their object is hidden (for statistics and tests).
    pub fn hidden_mesh_count(&self) -> usize {
        self.mesh_hidden.iter().filter(|h| **h).count()
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
        let layers = FpsLayers { crosshair_highlighted, weapon_transform, hand_prop_transform, opts };
        self.render_view(device, queue, scene, t, &camera.view(), target_view, Some(layers));
    }

    /// Draws one frame of `scene` at animation time `t` from `camera` into `target_view`: shadows, the lit world
    /// without objects hidden by [`Self::set_hidden_objects`], the clarity pass, then `self.overlay` (a HUD) on top.
    /// `fps` adds the first-person game's layers; a custom client passes `None`.
    pub fn render_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        t: f32,
        camera: &ViewCamera,
        target_view: &wgpu::TextureView,
        fps: Option<FpsLayers>,
    ) {
        let (crosshair_highlighted, weapon_transform, hand_prop_transform, opts) = match fps {
            Some(l) => (l.crosshair_highlighted, l.weapon_transform, l.hand_prop_transform, l.opts),
            None => (false, Mat4::IDENTITY, Mat4::IDENTITY, FrameOptions { crosshair: false, viewmodel: false, ..FrameOptions::default() }),
        };
        let draw_held = fps.is_some();
        let aspect = self.targets.width.max(1) as f32 / self.targets.height.max(1) as f32;
        // Far from the world's origin everything is measured from a nearby point (see `render_origin`), so the f32 maths stays accurate.
        let origin = crate::render::render_origin(camera.eye);
        let shift = Mat4::from_translation(-origin);
        let (weapon_transform, hand_prop_transform) =
            if origin == Vec3::ZERO { (weapon_transform, hand_prop_transform) } else { (shift * weapon_transform, shift * hand_prop_transform) };
        let camera = &ViewCamera { eye: camera.eye - origin, target: camera.target - origin, ..*camera };
        let view_proj = camera.view_proj(aspect);
        let globals = build_globals_common(scene, t, camera.eye, view_proj, origin);
        queue.write_buffer(&self.global_buf, 0, bytemuck::bytes_of(&globals));

        // Per-mesh frustum culling: which scene meshes are worth a draw call this frame, tested
        // against the camera's frustum (main pass) and, when a shadow-casting light is active,
        // the light's own ortho frustum (shadow pass) — skips both the vertex/fragment work and
        // the draw call for anything off-screen, which starts to matter once a prop-hunt map has
        // a few dozen props instead of a handful of room furniture.
        let cam_planes = frustum_planes(view_proj);
        let shadow_active = globals.counts[1] >= 0.0;
        let light_planes = shadow_active.then(|| frustum_planes(Mat4::from_cols_array_2d(&globals.light_view_proj)));
        // One entry per (image, mesh): slot `image * meshes + mesh`. Image 0 is the scene itself; on a looping world the others are its
        // neighbours one period away along the loop axis.
        let offsets: Vec<Vec3> = crate::render::wrap_offsets(scene).into_iter().map(|o| o - origin).collect();
        let n_meshes = self.meshes.len();

        // Object uniforms live in a persistent staging copy of the GPU buffer (see `object_staging`): the scene is sampled as before, but
        // only slots whose world matrix or material changed are rebuilt, and only their (coalesced) byte ranges are uploaded. Bounds are
        // cached per slot, so a moving camera over a static scene costs the sampling plus the plane tests and no uploads.
        self.staging.update_scene(scene, t, &offsets, &self.local_bounds);
        self.staging.cull(&cam_planes, light_planes.as_ref(), &self.mesh_hidden);
        let scene_slots = (n_meshes * offsets.len()) as u64;
        let held_slot = |k: usize, third: bool| scene_slots + 2 * k as u64 + third as u64;
        // Other players' weapons follow the local ones: one group of slots per player.
        let remote_slot = |r: usize, k: usize| scene_slots + 2 * self.held.len() as u64 + (r * self.held.len() + k) as u64;

        let held_uniform = |world: Mat4, h: &HeldGpu, glow: f32| (world, h.color, h.metallic, h.roughness, h.emissive * glow);
        // Only the active weapon's pieces are drawn (a muzzle flash only while it is up), and only drawn pieces are staged.
        let held_visible: Vec<bool> = self
            .held
            .iter()
            .map(|h| draw_held && h.weapon == opts.weapon && (h.skin == ANY_SKIN || h.skin == opts.skin) && (!h.flash || opts.muzzle_flash > 0.0))
            .collect();
        for (k, h) in self.held.iter().enumerate() {
            if !held_visible[k] {
                continue;
            }
            let glow = if h.flash { opts.muzzle_flash } else { 1.0 };
            if opts.viewmodel {
                let (w, c, m, r, e) = held_uniform(weapon_transform, h, glow);
                self.staging.update_extra(held_slot(k, false) as usize, w, c, m, r, e);
            }
            if !h.fp_only {
                let (w, c, m, r, e) = held_uniform(hand_prop_transform, h, glow);
                self.staging.update_extra(held_slot(k, true) as usize, w, c, m, r, e);
            }
        }
        for (r, hand) in self.remote_hands.iter().enumerate() {
            let world = shift * remote_hand_transform(hand);
            for (k, h) in self.held.iter().enumerate() {
                if h.weapon != hand.weapon || h.fp_only || !(h.skin == ANY_SKIN || h.skin == hand.skin) || (h.flash && hand.flash <= 0.0) {
                    continue;
                }
                let glow = if h.flash { hand.flash } else { 1.0 };
                let (w, c, m, rough, e) = held_uniform(world, h, glow);
                self.staging.update_extra(remote_slot(r, k) as usize, w, c, m, rough, e);
            }
        }
        self.staging.drain_dirty_into(&mut self.upload_ranges);
        for range in &self.upload_ranges {
            queue.write_buffer(&self.object_buf, range.start as u64, &self.staging.bytes()[range.clone()]);
        }

        if let Some(ocean) = &self.ocean {
            ocean.update(queue, self.clock.elapsed().as_secs_f32());
        }
        if let Some(stream) = &mut self.stream {
            if self.stream_eyes.is_empty() {
                stream.update(device, queue, &[camera.eye + origin]);
            } else {
                stream.update(device, queue, &self.stream_eyes);
            }
            stream.set_origin(queue, origin);
            stream.update_motes(queue, camera.eye + origin, camera.forward(), t, scene.clock.as_ref());
        }
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
            for image in 0..offsets.len() {
                for (i, mesh) in self.meshes.iter().enumerate() {
                    let slot = image * n_meshes + i;
                    if !self.staging.shadow_visible(slot) {
                        continue;
                    }
                    shadow_pass.set_bind_group(1, &self.object_bind_group, &[(slot as u64 * self.object_stride) as u32]);
                    shadow_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                    shadow_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
            if let (Some(stream), Some(planes)) = (&self.stream, &light_planes) {
                stream.draw_shadow(&mut shadow_pass, planes);
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
            for (r, hand) in self.remote_hands.iter().enumerate() {
                for (k, h) in self.held.iter().enumerate() {
                    if h.weapon != hand.weapon || h.fp_only || h.flash {
                        continue;
                    }
                    shadow_pass.set_bind_group(1, &self.object_bind_group, &[(remote_slot(r, k) * self.object_stride) as u32]);
                    shadow_pass.set_vertex_buffer(0, h.mesh.vertex_buf.slice(..));
                    shadow_pass.set_index_buffer(h.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    shadow_pass.draw_indexed(0..h.mesh.index_count, 0, 0..1);
                }
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
            for image in 0..offsets.len() {
                for (i, mesh) in self.meshes.iter().enumerate() {
                    let slot = image * n_meshes + i;
                    if !self.staging.main_visible(slot) {
                        continue;
                    }
                    main_pass.set_bind_group(1, &self.object_bind_group, &[(slot as u64 * self.object_stride) as u32]);
                    main_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                    main_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    main_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
            if let Some(stream) = &self.stream {
                stream.draw_main(&mut main_pass, view_proj);
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
            // Other players' weapons, with a muzzle flash only while it is up.
            for (r, hand) in self.remote_hands.iter().enumerate() {
                for (k, h) in self.held.iter().enumerate() {
                    if h.weapon != hand.weapon || h.fp_only || !(h.skin == ANY_SKIN || h.skin == hand.skin) || (h.flash && hand.flash <= 0.0) {
                        continue;
                    }
                    main_pass.set_bind_group(1, &self.object_bind_group, &[(remote_slot(r, k) * self.object_stride) as u32]);
                    main_pass.set_vertex_buffer(0, h.mesh.vertex_buf.slice(..));
                    main_pass.set_index_buffer(h.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    main_pass.draw_indexed(0..h.mesh.index_count, 0, 0..1);
                }
            }
            // See-through surfaces (`material.opacity` < 1): after every solid one, far to near, blended and not depth-writing.
            let blended = self.staging.blended(camera.eye);
            if !blended.is_empty() {
                main_pass.set_pipeline(&self.pipelines.main_alpha);
                for &slot in blended {
                    let mesh = &self.meshes[slot % n_meshes];
                    main_pass.set_bind_group(1, &self.object_bind_group, &[(slot as u64 * self.object_stride) as u32]);
                    main_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                    main_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    main_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
                main_pass.set_pipeline(&self.pipelines.main);
            }
            if let Some(stream) = &self.stream {
                stream.draw_motes(&mut main_pass, &self.pipelines.main_glow);
            }
            // The water goes last: it is depth-tested against everything opaque above and blends over the seabed under it.
            if let Some(ocean) = &self.ocean {
                ocean.draw(&mut main_pass, &self.global_bind_group_uniform);
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

        let crosshair_color = if opts.enemy {
            [1.0, 0.22, 0.18, 1.0]
        } else if opts.pickup {
            [0.35, 1.0, 0.45, 1.0]
        } else if crosshair_highlighted {
            [1.0, 0.85, 0.2, 1.0]
        } else {
            [1.0, 1.0, 1.0, 0.85]
        };
        let crosshair_uniform = CrosshairUniform {
            color: crosshair_color,
            to_ndc: [2.0 / self.targets.width.max(1) as f32, 2.0 / self.targets.height.max(1) as f32, (self.targets.height as f32 / 720.0).max(1.0), 0.0],
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
            if opts.fx.active() {
                self.fx.update(queue, &opts.fx, self.targets.width, self.targets.height);
                self.fx.draw(&mut crosshair_pass);
            }
            if opts.crosshair {
                crosshair_pass.set_pipeline(&self.crosshair.pipeline);
                crosshair_pass.set_bind_group(0, &self.crosshair_bind_group, &[]);
                crosshair_pass.draw(0..48, 0..1);
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
