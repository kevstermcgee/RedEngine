//! Offline renderer: builds GPU meshes from a `Scene`, renders frames headlessly (used by frame/tour/catalog/verify).

use crate::characters::{human_parts, rat_parts, CharPart, HumanLook, RatPose};
use crate::geometry::{build_stairs_parts, trs};
use crate::gpu::{
    create_pipelines, create_post_pipeline, make_shadow_sampler, post_uniform, FrameTargets, GlobalUniform, Gpu, GpuMesh, ObjectUniform, Pipelines, PostFx,
    MAX_LIGHTS,
};
use crate::mesh::Mesh;
use crate::props::{prop_parts, PropKind, PropPart};
use crate::schema::{Background, LightKind, Material, Object, ObjectKind, PrimKind, Scene};
use crate::skeleton::{HumanoidRig, PoseSample};
use anyhow::Result;
use glam::{Mat4, Quat, Vec3};

#[derive(Clone, Copy)]
pub(crate) struct SampledMaterial {
    pub color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    /// 1 = solid; below 1 the leaf is drawn in the blended pass.
    pub opacity: f32,
}

fn sample_pose(pose: &crate::schema::Pose, t: f32) -> PoseSample {
    PoseSample {
        spine: pose.spine.sample(t),
        head: pose.head.sample(t),
        l_shoulder: pose.l_shoulder.sample(t),
        r_shoulder: pose.r_shoulder.sample(t),
        l_elbow: pose.l_elbow.sample(t),
        r_elbow: pose.r_elbow.sample(t),
        l_hip: pose.l_hip.sample(t),
        r_hip: pose.r_hip.sample(t),
        l_knee: pose.l_knee.sample(t),
        r_knee: pose.r_knee.sample(t),
    }
}

fn sample_material(mat: &Material, t: f32) -> SampledMaterial {
    SampledMaterial { color: mat.color.sample(t), metallic: mat.metallic, roughness: mat.roughness, emissive: mat.emissive, opacity: mat.opacity }
}

pub(crate) fn build_prim_mesh(p: &PrimKind) -> Mesh {
    match p {
        PrimKind::Box { size } => Mesh::cuboid(*size),
        PrimKind::Sphere { radius } => Mesh::uv_sphere(*radius, 22, 30),
        PrimKind::Cylinder { radius, height } => Mesh::cylinder(*radius, *height, 28),
        PrimKind::Cone { radius, height } => Mesh::cone(*radius, *height, 28),
        PrimKind::Capsule { radius, height } => Mesh::capsule(*radius, *height, 22, 8),
        PrimKind::Plane { size } => Mesh::plane(size.0, size.1),
    }
}

/// A character part's final material: its own colour if it has one, else the object's.
fn char_material(base: &SampledMaterial, part: &crate::characters::CharPart) -> SampledMaterial {
    SampledMaterial {
        color: part.color.unwrap_or(base.color),
        metallic: part.metallic,
        roughness: part.roughness,
        emissive: base.emissive,
        opacity: base.opacity,
    }
}

pub(crate) fn collect_leaf_meshes(objects: &[Object], out: &mut Vec<Mesh>) {
    for o in objects {
        match &o.kind {
            ObjectKind::Prim(p) => out.push(build_prim_mesh(p)),
            ObjectKind::Group(children) => collect_leaf_meshes(children, out),
            ObjectKind::Humanoid(h) => {
                let rig = HumanoidRig::new(h.height, h.build);
                for part in human_parts(&rig, &PoseSample::default(), &h.look) {
                    out.push(build_prim_mesh(&part.shape));
                }
            }
            ObjectKind::Rat(_) => {
                for part in rat_parts(&RatPose::default()) {
                    out.push(build_prim_mesh(&part.shape));
                }
            }
            ObjectKind::Prop(p) => {
                for part in prop_parts(p.kind) {
                    out.push(build_prim_mesh(&part.shape));
                }
            }
            ObjectKind::Stairs(s) => {
                for (shape, _) in build_stairs_parts(s) {
                    out.push(build_prim_mesh(&shape));
                }
            }
            ObjectKind::Terrain(td) => {
                let at = o.position.sample(0.0);
                let tm = td.terrain.build_mesh(glam::Vec2::new(at.x, at.z), at.y);
                out.push(Mesh { vertices: tm.vertices.iter().map(|v| crate::mesh::Vertex::colored(v.pos, v.normal, v.color)).collect(), indices: tm.indices });
            }
        }
    }
}

/// For each mesh produced by [`collect_leaf_meshes`], records the object-id path that owns it.
/// A group id remains in the path of every descendant, so hiding either a whole prefab/group or
/// one nested object can be implemented without changing scene transforms or rebuilding GPU data.
pub(crate) fn collect_leaf_object_paths(objects: &[Object], parents: &[String], out: &mut Vec<Vec<String>>) {
    for o in objects {
        let mut path = parents.to_vec();
        path.push(o.id.clone());
        let count = match &o.kind {
            ObjectKind::Prim(_) => 1,
            ObjectKind::Group(children) => {
                collect_leaf_object_paths(children, &path, out);
                0
            }
            ObjectKind::Humanoid(h) => human_parts(&HumanoidRig::new(h.height, h.build), &PoseSample::default(), &h.look).len(),
            ObjectKind::Rat(_) => rat_parts(&RatPose::default()).len(),
            ObjectKind::Prop(p) => prop_parts(p.kind).len(),
            ObjectKind::Stairs(s) => build_stairs_parts(s).len(),
            ObjectKind::Terrain(_) => 1,
        };
        out.extend(std::iter::repeat_n(path, count));
    }
}

/// Invariant part lists compiled once and reused every frame: a prop kind's parts never change, and a stairs' treads depend only on its four
/// dimensions. Keyed by value, so an object whose kind or dimensions were edited simply gets (and caches) the new expansion.
#[derive(Default)]
pub(crate) struct PartsCache {
    props: Vec<(PropKind, Vec<PropPart>)>,
    stairs: Vec<(StairsKey, Vec<(PrimKind, Mat4)>)>,
    /// Posed characters in traversal order; a character is re-posed only when its sampled inputs changed.
    chars: Vec<(CharKey, Vec<CharPart>)>,
    /// Which entry of `chars` the traversal is at.
    next_char: usize,
}

/// Everything a posed humanoid's or rat's parts are derived from.
#[derive(Clone, Copy, PartialEq)]
enum CharKey {
    Human { height: f32, build: f32, pose: PoseSample, look: HumanLook },
    Rat(RatPose),
}

/// The fields of a `StairsDef` that shape its treads (not its material).
#[derive(Clone, Copy, PartialEq)]
struct StairsKey {
    width: f32,
    run: f32,
    rise: f32,
    steps: u32,
}

impl PartsCache {
    fn prop(&mut self, kind: PropKind) -> &[PropPart] {
        let i = match self.props.iter().position(|(k, _)| *k == kind) {
            Some(i) => i,
            None => {
                self.props.push((kind, prop_parts(kind)));
                self.props.len() - 1
            }
        };
        &self.props[i].1
    }

    /// The parts for the next character in traversal order, re-posing them only if `key` differs from last frame's. Keyed by value
    /// (not identity), so inserting, removing or editing objects can only cause a recompute, never stale parts.
    fn character(&mut self, key: CharKey, build: impl FnOnce() -> Vec<CharPart>) -> &[CharPart] {
        let i = self.next_char;
        self.next_char += 1;
        match self.chars.get_mut(i) {
            Some(entry) if entry.0 == key => {}
            Some(entry) => *entry = (key, build()),
            None => self.chars.push((key, build())),
        }
        &self.chars[i].1
    }

    fn stairs(&mut self, s: &crate::schema::StairsDef) -> &[(PrimKind, Mat4)] {
        let key = StairsKey { width: s.width, run: s.run, rise: s.rise, steps: s.steps };
        let i = match self.stairs.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                self.stairs.push((key, build_stairs_parts(s)));
                self.stairs.len() - 1
            }
        };
        &self.stairs[i].1
    }
}

/// [`collect_leaf_transforms_cached`] with a throwaway cache: the offline renderer samples a handful of frames, so it does not keep one.
pub(crate) fn collect_leaf_transforms(objects: &[Object], t: f32, parent: Mat4, out: &mut Vec<(Mat4, SampledMaterial)>) {
    collect_leaf_transforms_cached(objects, t, parent, out, &mut PartsCache::default());
}

/// Samples every leaf's world matrix and material at `t` into `out` (draw order, see [`collect_leaf_meshes`]), reusing `cache`'s compiled parts.
pub(crate) fn collect_leaf_transforms_cached(objects: &[Object], t: f32, parent: Mat4, out: &mut Vec<(Mat4, SampledMaterial)>, cache: &mut PartsCache) {
    cache.next_char = 0;
    collect_leaf_transforms_inner(objects, t, parent, out, cache);
}

fn collect_leaf_transforms_inner(objects: &[Object], t: f32, parent: Mat4, out: &mut Vec<(Mat4, SampledMaterial)>, cache: &mut PartsCache) {
    for o in objects {
        let local = trs(o.position.sample(t), o.rotation.sample(t), o.scale.sample(t));
        let world = parent * local;
        match &o.kind {
            ObjectKind::Prim(_) => {
                let mat = sample_material(o.material.as_ref().expect("primitive always has a material"), t);
                out.push((world, mat));
            }
            ObjectKind::Group(children) => collect_leaf_transforms_inner(children, t, world, out, cache),
            ObjectKind::Humanoid(h) => {
                let rig = HumanoidRig::new(h.height, h.build);
                let pose = sample_pose(&h.pose, t);
                let base = sample_material(&h.material, t);
                let key = CharKey::Human { height: h.height, build: h.build, pose, look: h.look };
                for part in cache.character(key, || human_parts(&rig, &pose, &h.look)) {
                    out.push((world * part.local, char_material(&base, part)));
                }
            }
            ObjectKind::Rat(r) => {
                let base = sample_material(&r.material, t);
                let pose = RatPose { gait: r.gait.sample(t), stride: r.stride.sample(t), sway: r.sway.sample(t) };
                for part in cache.character(CharKey::Rat(pose), || rat_parts(&pose)) {
                    out.push((world * part.local, char_material(&base, part)));
                }
            }
            ObjectKind::Prop(p) => {
                let base = sample_material(&p.material, t);
                for part in cache.prop(p.kind) {
                    let mat = SampledMaterial {
                        color: part.color_override.unwrap_or(base.color),
                        metallic: (base.metallic + part.metallic_delta).clamp(0.0, 1.0),
                        roughness: (base.roughness + part.roughness_delta).clamp(0.04, 1.0),
                        emissive: base.emissive,
                        opacity: base.opacity,
                    };
                    out.push((world * part.local_transform, mat));
                }
            }
            ObjectKind::Stairs(s) => {
                let mat = sample_material(&s.material, t);
                for (_, local_transform) in cache.stairs(s) {
                    out.push((world * *local_transform, mat));
                }
            }
            ObjectKind::Terrain(td) => out.push((world, sample_material(&td.material, t))),
        }
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

/// How far from the world's origin the camera may be before the renderer measures from somewhere nearer (m). f32 has about 7 digits: at 4 km a position is
/// good to half a millimetre, at 20 km to two, and the inverse of a view matrix (the sky, the fog) starts to fall apart. Scenes within this are drawn exactly
/// as before; an endless world is not.
pub const REBASE_DISTANCE: f32 = 4096.0;

/// The point the renderer measures from for a camera at `eye`: the world origin, or (far out) the multiple of 2048 m nearest to the camera. Everything the
/// GPU sees (camera, lights, objects, chunks) is shifted by it, so the numbers it works with stay small however far the player has walked.
pub fn render_origin(eye: Vec3) -> Vec3 {
    if eye.x.abs().max(eye.z.abs()) < REBASE_DISTANCE {
        return Vec3::ZERO;
    }
    Vec3::new((eye.x / 2048.0).round() * 2048.0, 0.0, (eye.z / 2048.0).round() * 2048.0)
}

/// The translations at which the scene is drawn: `[0]`, or on a looping world (`world.wrap`) the scene itself plus its two neighbours
/// one period away along the loop axis, so what lies past the seam is drawn where the player expects it and the seam cannot be seen.
/// Index 0 is always the scene as authored.
pub fn wrap_offsets(scene: &Scene) -> Vec<Vec3> {
    match scene.player.expanse.wrap {
        None => vec![Vec3::ZERO],
        Some(w) => {
            let p = w.period();
            let dir = match w.axis {
                crate::expanse::Axis::X => Vec3::X,
                crate::expanse::Axis::Z => Vec3::Z,
            };
            vec![Vec3::ZERO, -dir * p, dir * p]
        }
    }
}

/// Everything needed to render many frames of one scene without re-initializing the GPU.
pub struct Renderer {
    gpu: Gpu,
    pipelines: Pipelines,
    global_buf: wgpu::Buffer,
    global_bind_group_uniform: wgpu::BindGroup,
    global_bind_group_full: wgpu::BindGroup,
    object_buf: wgpu::Buffer,
    object_stride: u64,
    object_bind_group: wgpu::BindGroup,
    targets: FrameTargets,
    post: PostFx,
    post_bind_group: wgpu::BindGroup,
    meshes: Vec<GpuMesh>,
    /// The scene's ocean, when it has one.
    ocean: Option<crate::ocean_pass::OceanPass>,
    /// The scene's endless generated world, when it has a `procgen` block.
    stream: Option<crate::stream_gpu::StreamLayer>,
    out_width: u32,
    out_height: u32,
}

impl Renderer {
    /// Builds the offline renderer (device, pipelines, meshes) for `scene`.
    pub fn new(scene: &Scene) -> Result<Self> {
        let gpu = Gpu::new()?;
        let pipelines = create_pipelines(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
        let shadow_sampler = make_shadow_sampler(&gpu.device);
        let targets = FrameTargets::new(&gpu.device, scene.width, scene.height);
        let post = create_post_pipeline(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
        let post_bind_group = post.bind(&gpu.device, &targets.depth_view);
        let ocean = crate::ocean_pass::OceanPass::new(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, 1, scene, &pipelines.layouts.global_uniform);

        let mut raw_meshes = Vec::new();
        collect_leaf_meshes(&scene.objects, &mut raw_meshes);
        let meshes: Vec<GpuMesh> = raw_meshes.iter().map(|m| GpuMesh::upload(&gpu.device, m)).collect();
        let draw_count = (meshes.len() * wrap_offsets(scene).len()).max(1) as u64;

        let global_buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("global-uniform"),
            size: std::mem::size_of::<GlobalUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let alignment = gpu.device.limits().min_uniform_buffer_offset_alignment as u64;
        let object_stride = align_up(std::mem::size_of::<ObjectUniform>() as u64, alignment);
        let object_buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("object-uniforms"),
            size: object_stride * draw_count,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let global_bind_group_uniform = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("global-bind-group-uniform"),
            layout: &pipelines.layouts.global_uniform,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: global_buf.as_entire_binding() }],
        });
        let global_bind_group_full = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("global-bind-group-full"),
            layout: &pipelines.layouts.global_full,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: global_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&targets.shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            ],
        });
        let object_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("object-bind-group"),
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

        let stream = scene
            .procgen
            .clone()
            .map(|cfg| crate::stream_gpu::StreamLayer::new(&gpu.device, &pipelines.layouts.object, cfg, crate::procgen::View::default(), 0));
        Ok(Renderer {
            gpu,
            pipelines,
            global_buf,
            global_bind_group_uniform,
            global_bind_group_full,
            object_buf,
            object_stride,
            object_bind_group,
            targets,
            post,
            post_bind_group,
            meshes,
            ocean,
            stream,
            out_width: scene.width,
            out_height: scene.height,
        })
    }

    fn build_globals(&self, scene: &Scene, t: f32) -> GlobalUniform {
        let origin = render_origin(scene.camera.position.sample(t));
        let cam_pos = scene.camera.position.sample(t) - origin;
        let cam_target = scene.camera.target.sample(t) - origin;
        let fov = scene.camera.fov.sample(t).max(1.0).to_radians();
        let aspect = self.targets.width as f32 / self.targets.height as f32;
        let proj = glam::camera::rh::proj::directx::perspective(fov, aspect, scene.camera.near, scene.camera.far);

        let fwd = (cam_target - cam_pos).normalize_or_zero();
        let roll = scene.camera.roll.sample(t).to_radians();
        let up = if roll.abs() > 1e-6 { Quat::from_axis_angle(fwd, roll) * Vec3::Y } else { Vec3::Y };
        let up = if up.length_squared() < 1e-6 { Vec3::Z } else { up };
        let view = glam::camera::rh::view::look_at_mat4(cam_pos, cam_target, up);
        let view_proj = proj * view;

        build_globals_common(scene, t, cam_pos, view_proj, origin)
    }

    /// Renders one frame at time `t` (seconds) and returns tightly-packed RGB8 pixels,
    /// `out_width * out_height * 3` bytes, top row first.
    pub fn render_frame(&mut self, scene: &Scene, t: f32) -> Vec<u8> {
        let globals = self.build_globals(scene, t);
        self.gpu.queue.write_buffer(&self.global_buf, 0, bytemuck::bytes_of(&globals));
        // A still frame cannot wait for the world to stream in: build everything around the camera first.
        let origin = render_origin(scene.camera.position.sample(t));
        if let Some(stream) = &mut self.stream {
            stream.fill(&self.gpu.device, &self.gpu.queue, scene.camera.position.sample(t));
            stream.set_origin(&self.gpu.queue, origin);
        }

        let mut transforms = Vec::with_capacity(self.meshes.len());
        collect_leaf_transforms(&scene.objects, t, Mat4::IDENTITY, &mut transforms);
        debug_assert_eq!(transforms.len(), self.meshes.len());

        let offsets: Vec<Vec3> = wrap_offsets(scene).into_iter().map(|o| o - origin).collect();
        let n_meshes = self.meshes.len();
        // See-through leaves (`opacity` < 1), drawn after the solid ones, far to near: `(distance from the camera squared, slot)`.
        let eye = scene.camera.position.sample(t) - origin;
        let mut blended: Vec<(f32, usize)> = Vec::new();
        let mut is_blended = vec![false; n_meshes * offsets.len()];
        for (image, offset) in offsets.iter().enumerate() {
            for (i, (world, mat)) in transforms.iter().enumerate() {
                let world = Mat4::from_translation(*offset) * *world;
                if mat.opacity < 1.0 {
                    blended.push((world.w_axis.truncate().distance_squared(eye), image * n_meshes + i));
                    is_blended[image * n_meshes + i] = true;
                }
                let normal_mat = world.inverse().transpose();
                let obj_uniform = ObjectUniform {
                    model: world.to_cols_array_2d(),
                    normal_mat: normal_mat.to_cols_array_2d(),
                    base_color: [mat.color.x, mat.color.y, mat.color.z, mat.opacity],
                    material: [mat.metallic, mat.roughness, 0.0, 0.0],
                    emissive: [mat.emissive.x, mat.emissive.y, mat.emissive.z, 0.0],
                };
                self.gpu.queue.write_buffer(&self.object_buf, (image * n_meshes + i) as u64 * self.object_stride, bytemuck::bytes_of(&obj_uniform));
            }
        }

        if let Some(ocean) = &self.ocean {
            ocean.update(&self.gpu.queue, t);
        }
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame-encoder") });

        if globals.counts[1] >= 0.0 {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-pass"),
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
                    if is_blended[image * n_meshes + i] {
                        continue; // see-through surfaces cast no shadow
                    }
                    shadow_pass.set_bind_group(1, &self.object_bind_group, &[((image * n_meshes + i) as u64 * self.object_stride) as u32]);
                    shadow_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                    shadow_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
            if let Some(stream) = &self.stream {
                stream.draw_shadow(&mut shadow_pass, &crate::object_staging::frustum_planes(Mat4::from_cols_array_2d(&globals.light_view_proj)));
            }
        }

        {
            let mut bg_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.color_view,
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
                label: Some("main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.color_view,
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
                    if is_blended[image * n_meshes + i] {
                        continue;
                    }
                    main_pass.set_bind_group(1, &self.object_bind_group, &[((image * n_meshes + i) as u64 * self.object_stride) as u32]);
                    main_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                    main_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    main_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
            if let Some(stream) = &self.stream {
                stream.draw_main(&mut main_pass, Mat4::from_cols_array_2d(&globals.view_proj));
            }
            if !blended.is_empty() {
                blended.sort_by(|a, b| b.0.total_cmp(&a.0));
                main_pass.set_pipeline(&self.pipelines.main_alpha);
                for &(_, slot) in &blended {
                    let mesh = &self.meshes[slot % n_meshes];
                    main_pass.set_bind_group(1, &self.object_bind_group, &[(slot as u64 * self.object_stride) as u32]);
                    main_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                    main_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                    main_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
            if let Some(ocean) = &self.ocean {
                ocean.draw(&mut main_pass, &self.global_bind_group_uniform);
            }
        }

        // Clarity pass (contact AO + outlines) over the finished world, before readback.
        {
            let fov = scene.camera.fov.sample(t).max(1.0);
            // ~2 px of outline at 1080p, scaled with the (supersampled) target height.
            let edge_px = (2.0 * self.targets.height as f32 / 1080.0).max(1.0);
            let uniform = post_uniform(&scene.post, scene.camera.near, scene.camera.far, fov, self.targets.width, self.targets.height, edge_px);
            self.gpu.queue.write_buffer(&self.post.uniform_buf, 0, bytemuck::bytes_of(&uniform));
            let mut post_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.color_view,
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

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.targets.color_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.targets.staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.targets.padded_bytes_per_row),
                    rows_per_image: Some(self.targets.height),
                },
            },
            wgpu::Extent3d { width: self.targets.width, height: self.targets.height, depth_or_array_layers: 1 },
        );
        self.gpu.queue.submit(Some(encoder.finish()));

        let raw = self.read_staging_buffer();
        downsample_rgba_to_rgb(&raw, &self.targets, self.out_width, self.out_height)
    }

    fn read_staging_buffer(&self) -> Vec<u8> {
        let slice = self.targets.staging_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        self.gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll failed");
        rx.recv().expect("map_async callback dropped").expect("failed to map staging buffer");
        let data = slice.get_mapped_range().expect("buffer was just mapped successfully").to_vec();
        self.targets.staging_buffer.unmap();
        data
    }
}

/// Packs lights/ambient/background into a [`GlobalUniform`] given an already-computed camera
/// (position + view-projection matrix). Shared by the offline [`Renderer`] (camera driven by the
/// scene's `camera` track) and the live viewer (camera driven by player input instead).
/// `cam_pos` and `view_proj` are already measured from `origin` (see [`render_origin`]); the scene's own light positions are shifted here.
pub(crate) fn build_globals_common(scene: &Scene, t: f32, cam_pos: Vec3, view_proj: Mat4, origin: Vec3) -> GlobalUniform {
    let mut light_pos_or_dir = [[0f32; 4]; MAX_LIGHTS];
    let mut light_color_intensity = [[0f32; 4]; MAX_LIGHTS];
    let mut shadow_idx: i32 = -1;
    let mut light_view_proj = Mat4::IDENTITY;
    // A scene with a `clock`: the sky, the sun's light, the ambient and the moon follow the time of day.
    let day = scene.clock.as_ref().map(|c| c.state(t, 0));
    let sun_index = scene.clock.as_ref().and_then(|c| {
        c.light
            .as_ref()
            .and_then(|id| scene.lights.iter().position(|l| &l.id == id))
            .or_else(|| scene.lights.iter().position(|l| matches!(l.kind, LightKind::Directional { .. })))
    });
    let mut selected: Vec<usize> =
        scene.lights.iter().enumerate().filter_map(|(i, light)| matches!(light.kind, LightKind::Directional { .. }).then_some(i)).collect();
    let mut points: Vec<(usize, f32)> = scene
        .lights
        .iter()
        .enumerate()
        .filter_map(|(i, light)| match &light.kind {
            LightKind::Point { position, .. } => Some((i, (position.sample(t) - origin).distance_squared(cam_pos))),
            LightKind::Directional { .. } => None,
        })
        .collect();
    points.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    selected.extend(points.into_iter().map(|(i, _)| i));
    selected.truncate(MAX_LIGHTS);
    // The shadow map for a directional light shining along `d`: centred on `center`, or under the camera (snapped to texels so shadows do not crawl) when it follows.
    let shadow_matrix = |d: Vec3, radius: f32, center: Vec3, follow: bool| -> Mat4 {
        let r = radius.max(0.5);
        let mut center = center;
        if follow {
            let texel = (2.0 * r) / crate::gpu::SHADOW_SIZE as f32;
            center = Vec3::new((cam_pos.x / texel).round() * texel, center.y, (cam_pos.z / texel).round() * texel);
        }
        let light_pos = center - d * (r * 1.6);
        let up = if d.y.abs() > 0.98 { Vec3::Z } else { Vec3::Y };
        let view_l = glam::camera::rh::view::look_at_mat4(light_pos, center, up);
        let proj_l = glam::camera::rh::proj::directx::orthographic(-r, r, -r, r, 0.05, r * 3.5);
        proj_l * view_l
    };
    let mut n = selected.len();
    for (i, source_index) in selected.into_iter().enumerate() {
        let light = &scene.lights[source_index];
        let intensity = light.intensity.sample(t);
        let mut color = light.color.sample(t) * intensity;
        match &light.kind {
            LightKind::Directional { direction } => {
                let mut d = direction.sample(t).normalize_or_zero();
                if let (Some(day), true) = (&day, Some(source_index) == sun_index) {
                    // The sun: its direction and colour from the clock, scaled by the light's own colour and intensity (so an author can tune it).
                    d = day.light_dir;
                    color = day.light_color * light.color.sample(t) * intensity;
                }
                light_pos_or_dir[i] = [d.x, d.y, d.z, 0.0];
                light_color_intensity[i] = [color.x, color.y, color.z, 0.0];
                if light.cast_shadows {
                    shadow_idx = i as i32;
                    light_view_proj = shadow_matrix(d, light.shadow_radius, light.shadow_center, light.shadow_follow);
                }
            }
            LightKind::Point { position, range } => {
                let p = position.sample(t) - origin;
                light_pos_or_dir[i] = [p.x, p.y, p.z, 1.0];
                light_color_intensity[i] = [color.x, color.y, color.z, *range];
            }
        }
    }
    if let Some(day) = &day {
        // No directional light authored: the clock supplies a sun that casts shadows around the camera.
        if sun_index.is_none() && n < MAX_LIGHTS {
            let d = day.light_dir;
            light_pos_or_dir[n] = [d.x, d.y, d.z, 0.0];
            light_color_intensity[n] = [day.light_color.x, day.light_color.y, day.light_color.z, 0.0];
            shadow_idx = n as i32;
            light_view_proj = shadow_matrix(d, 48.0, Vec3::ZERO, true);
            n += 1;
        }
        // Moonlight: a dim, cool fill from the moon's own place in the sky (no shadows).
        if n < MAX_LIGHTS && day.moon_light_color.max_element() > 1e-4 {
            let d = day.moon_light_dir;
            light_pos_or_dir[n] = [d.x, d.y, d.z, 0.0];
            light_color_intensity[n] = [day.moon_light_color.x, day.moon_light_color.y, day.moon_light_color.z, 0.0];
            n += 1;
        }
    }

    let ambient = match &day {
        Some(d) => d.ambient,
        None => scene.ambient_color * scene.ambient_intensity,
    };
    let (bg_top, bg_bottom, bg_mode) = match &scene.background {
        Background::Flat(c) => (*c, Vec3::ZERO, 0.0f32),
        Background::Gradient { top, bottom } => (*top, *bottom, 1.0f32),
    };

    // The sky: a view-direction gradient (zenith over horizon) and a sun at infinity, when the scene has a `sky` block (or a `clock`, which supplies one).
    let (bg_top, bg_bottom, bg_mode, sky_flags) = match (&day, &scene.sky) {
        (Some(d), sky) => (d.zenith, d.horizon, 1.0f32, [1.0, sky.map_or(0.6, |s| s.gradient_power), 1.0, 0.0]),
        (None, Some(sky)) => (sky.zenith, sky.horizon, 1.0f32, [1.0, sky.gradient_power, if sky.sun.is_some() { 1.0 } else { 0.0 }, 0.0]),
        (None, None) => (bg_top, bg_bottom, bg_mode, [0.0; 4]),
    };
    let sun = scene.sky.and_then(|s| s.sun);
    let (sun_dir, sun_color) = match &day {
        Some(d) => ([d.sun_dir.x, d.sun_dir.y, d.sun_dir.z, d.sun_radius_deg.to_radians()], [d.sun_color.x, d.sun_color.y, d.sun_color.z, 0.85]),
        None => (
            sun.map_or([0.0, 1.0, 0.0, 0.0], |s| [s.direction.x, s.direction.y, s.direction.z, s.radius_deg.to_radians()]),
            sun.map_or([0.0; 4], |s| [s.color.x, s.color.y, s.color.z, s.glow]),
        ),
    };
    let clock = scene.clock.as_ref();
    let (moon_dir, night, celestial, glow, fog) = match (&day, clock) {
        (Some(d), Some(c)) => (
            [d.moon_dir.x, d.moon_dir.y, d.moon_dir.z, if c.moon { 2.6f32.to_radians() } else { 0.0 }],
            [d.star_visibility, t, d.moon_phase, 1.0],
            d.celestial.map(|r| [r.x, r.y, r.z, 0.0]),
            [d.glow.x, d.glow.y, d.glow.z, d.glow_strength],
            [d.horizon.x, d.horizon.y, d.horizon.z, c.fog],
        ),
        // No clock: no stars, moon or glow, but the scene time still reaches the shaders (the wind blows in any scene with plants).
        _ => ([0.0, 1.0, 0.0, 0.0], [0.0, t, 0.0, 0.0], [[0.0; 4]; 3], [0.0; 4], [0.0; 4]),
    };

    GlobalUniform {
        view_proj: view_proj.to_cols_array_2d(),
        light_view_proj: light_view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        ambient: [ambient.x, ambient.y, ambient.z, 0.0],
        light_pos_or_dir,
        light_color_intensity,
        counts: [n as f32, shadow_idx as f32, 0.0, 0.0],
        bg_top: [bg_top.x, bg_top.y, bg_top.z, bg_mode],
        bg_bottom: [bg_bottom.x, bg_bottom.y, bg_bottom.z, 0.0],
        inv_view_proj: view_proj.inverse().to_cols_array_2d(),
        sun_dir,
        sun_color,
        sky: sky_flags,
        moon_dir,
        night,
        celestial,
        glow,
        fog,
    }
}

fn downsample_rgba_to_rgb(raw: &[u8], targets: &FrameTargets, out_w: u32, out_h: u32) -> Vec<u8> {
    let factor = targets.width / out_w;
    let mut out = vec![0u8; (out_w * out_h * 3) as usize];
    for y in 0..out_h {
        for x in 0..out_w {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let samples = factor * factor;
            for sy in 0..factor {
                let src_y = y * factor + sy;
                let row_start = src_y as usize * targets.padded_bytes_per_row as usize;
                for sx in 0..factor {
                    let src_x = x * factor + sx;
                    let px = row_start + src_x as usize * 4;
                    r += raw[px] as u32;
                    g += raw[px + 1] as u32;
                    b += raw[px + 2] as u32;
                }
            }
            let out_idx = (y * out_w + x) as usize * 3;
            out[out_idx] = (r / samples) as u8;
            out[out_idx + 1] = (g / samples) as u8;
            out[out_idx + 2] = (b / samples) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::StairsDef;

    #[test]
    fn stairs_parts_are_valid_and_non_degenerate() {
        let material = Material { color: crate::track::Track::constant(Vec3::splat(0.7)), metallic: 0.0, roughness: 0.6, emissive: Vec3::ZERO, opacity: 1.0 };
        let s = StairsDef { width: 1.2, run: 4.0, rise: 3.0, steps: 16, material };
        let parts = build_stairs_parts(&s);
        assert_eq!(parts.len(), 16);
        for (shape, _) in &parts {
            let mesh = build_prim_mesh(shape);
            assert!(!mesh.vertices.is_empty());
            assert!(!mesh.indices.is_empty());
        }
        // Each step should be strictly taller than the last (a solid, climbing staircase, not
        // flat slabs at the same height).
        let mut last_height = 0.0f32;
        for (shape, _) in &parts {
            if let PrimKind::Box { size } = shape {
                assert!(size.y > last_height, "step height did not increase: {} <= {}", size.y, last_height);
                last_height = size.y;
            } else {
                panic!("stairs parts should all be boxes");
            }
        }
    }

    #[test]
    fn every_leaf_mesh_keeps_its_object_ancestry_for_visibility() {
        let scene = crate::schema::parse_scene(
            r##"{"camera":{"position":[0,2,5],"target":[0,0,0]},"objects":[
              {"id":"plain","type":"box","size":[1,1,1]},
              {"id":"prefab","type":"group","children":[
                {"id":"nested","type":"sphere","radius":1},
                {"id":"steps","type":"stairs","position":[0,0,0],"width":1,"run":2,"rise":1,"steps":3}
              ]}] }"##,
        )
        .unwrap();
        let mut meshes = Vec::new();
        let mut paths = Vec::new();
        collect_leaf_meshes(&scene.objects, &mut meshes);
        collect_leaf_object_paths(&scene.objects, &[], &mut paths);
        assert_eq!(paths.len(), meshes.len());
        assert_eq!(paths[0], ["plain"]);
        assert_eq!(paths[1], ["prefab", "nested"]);
        assert!(paths[2..].iter().all(|p| p == &["prefab", "steps"]));
    }

    #[test]
    fn authored_point_lights_are_culled_to_the_nearest_shader_budget() {
        let lights = (0..20)
            .map(|x| {
                serde_json::json!({
                    "id": format!("light_{x}"),
                    "type": "point",
                    "position": [x, 0, 0],
                    "range": 10
                })
            })
            .collect::<Vec<_>>();
        let text = serde_json::json!({"camera": {}, "lights": lights, "objects": []}).to_string();
        let scene = crate::schema::parse_scene(&text).expect("more than 16 authored lights should parse");
        let globals = build_globals_common(&scene, 0.0, Vec3::new(18.0, 0.0, 0.0), Mat4::IDENTITY, Vec3::ZERO);

        assert_eq!(globals.counts[0], MAX_LIGHTS as f32);
        assert_eq!(globals.light_pos_or_dir[0], [18.0, 0.0, 0.0, 1.0]);
        assert!(globals.light_pos_or_dir[..MAX_LIGHTS].iter().all(|light| light[0] != 0.0));
    }
}
