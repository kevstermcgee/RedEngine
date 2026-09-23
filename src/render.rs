use crate::gpu::{
    create_pipelines, make_shadow_sampler, FrameTargets, Gpu, GlobalUniform, GpuMesh, ObjectUniform, Pipelines,
    MAX_LIGHTS,
};
use crate::mesh::Mesh;
use crate::props::prop_parts;
use crate::schema::{Background, LightKind, Material, Object, ObjectKind, PrimKind, Scene, StairsDef};
use crate::skeleton::{pose_to_parts, BoneKind, HumanoidRig, PoseSample};
use anyhow::Result;
use glam::{Mat4, Quat, Vec3};

#[derive(Clone, Copy)]
pub(crate) struct SampledMaterial {
    pub color: Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
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
    SampledMaterial { color: mat.color.sample(t), metallic: mat.metallic, roughness: mat.roughness, emissive: mat.emissive }
}

pub(crate) fn trs(pos: Vec3, rot_deg: Vec3, scale: Vec3) -> Mat4 {
    let rot = Quat::from_euler(glam::EulerRot::XYZ, rot_deg.x.to_radians(), rot_deg.y.to_radians(), rot_deg.z.to_radians());
    Mat4::from_scale_rotation_translation(scale, rot, pos)
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

/// `steps` solid stacked box treads: step `i` owns its own depth slice of the run
/// (`[-run/2 + i*step_d, -run/2 + (i+1)*step_d]`) and spans height `[0, (i+1)*step_h]` — each
/// box is a self-contained solid block, so the whole thing reads as a real staircase silhouette
/// rather than floating slabs. Purely visual; `viewer::ground_height_at` uses a separate smooth
/// ramp formula for actually walking on it (see that function's doc comment for why).
pub(crate) fn build_stairs_parts(s: &StairsDef) -> Vec<(PrimKind, Mat4)> {
    let step_h = s.rise / s.steps as f32;
    let step_d = s.run / s.steps as f32;
    (0..s.steps)
        .map(|i| {
            let z_start = -s.run * 0.5 + step_d * i as f32;
            let y_height = step_h * (i + 1) as f32;
            let shape = PrimKind::Box { size: Vec3::new(s.width, y_height, step_d) };
            let center = Vec3::new(0.0, y_height * 0.5, z_start + step_d * 0.5);
            (shape, Mat4::from_translation(center))
        })
        .collect()
}

pub(crate) fn collect_leaf_meshes(objects: &[Object], out: &mut Vec<Mesh>) {
    for o in objects {
        match &o.kind {
            ObjectKind::Prim(p) => out.push(build_prim_mesh(p)),
            ObjectKind::Group(children) => collect_leaf_meshes(children, out),
            ObjectKind::Humanoid(h) => {
                let rig = HumanoidRig::new(h.height, h.build);
                let parts = pose_to_parts(&rig, &PoseSample::default());
                for part in &parts {
                    let mesh = match part.kind {
                        BoneKind::Sphere => Mesh::uv_sphere(part.radius, 16, 20),
                        BoneKind::Capsule => Mesh::capsule(part.radius, part.length, 14, 6),
                    };
                    out.push(mesh);
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
        }
    }
}

pub(crate) fn collect_leaf_transforms(objects: &[Object], t: f32, parent: Mat4, out: &mut Vec<(Mat4, SampledMaterial)>) {
    for o in objects {
        let local = trs(o.position.sample(t), o.rotation.sample(t), o.scale.sample(t));
        let world = parent * local;
        match &o.kind {
            ObjectKind::Prim(_) => {
                let mat = sample_material(o.material.as_ref().expect("primitive always has a material"), t);
                out.push((world, mat));
            }
            ObjectKind::Group(children) => collect_leaf_transforms(children, t, world, out),
            ObjectKind::Humanoid(h) => {
                let rig = HumanoidRig::new(h.height, h.build);
                let pose = sample_pose(&h.pose, t);
                let parts = pose_to_parts(&rig, &pose);
                let mat = sample_material(&h.material, t);
                for part in &parts {
                    let bone_local = Mat4::from_rotation_translation(part.rotation, part.center);
                    out.push((world * bone_local, mat));
                }
            }
            ObjectKind::Prop(p) => {
                let base = sample_material(&p.material, t);
                for part in prop_parts(p.kind) {
                    let mat = SampledMaterial {
                        color: part.color_override.unwrap_or(base.color),
                        metallic: (base.metallic + part.metallic_delta).clamp(0.0, 1.0),
                        roughness: (base.roughness + part.roughness_delta).clamp(0.04, 1.0),
                        emissive: base.emissive,
                    };
                    out.push((world * part.local_transform, mat));
                }
            }
            ObjectKind::Stairs(s) => {
                let mat = sample_material(&s.material, t);
                for (_, local_transform) in build_stairs_parts(s) {
                    out.push((world * local_transform, mat));
                }
            }
        }
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
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
    meshes: Vec<GpuMesh>,
    out_width: u32,
    out_height: u32,
}

impl Renderer {
    pub fn new(scene: &Scene) -> Result<Self> {
        let gpu = Gpu::new()?;
        let pipelines = create_pipelines(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, 1);
        let shadow_sampler = make_shadow_sampler(&gpu.device);
        let targets = FrameTargets::new(&gpu.device, scene.width, scene.height);

        let mut raw_meshes = Vec::new();
        collect_leaf_meshes(&scene.objects, &mut raw_meshes);
        let meshes: Vec<GpuMesh> = raw_meshes.iter().map(|m| GpuMesh::upload(&gpu.device, m)).collect();
        let draw_count = meshes.len().max(1) as u64;

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
            meshes,
            out_width: scene.width,
            out_height: scene.height,
        })
    }

    fn build_globals(&self, scene: &Scene, t: f32) -> GlobalUniform {
        let cam_pos = scene.camera.position.sample(t);
        let cam_target = scene.camera.target.sample(t);
        let fov = scene.camera.fov.sample(t).max(1.0).to_radians();
        let aspect = self.targets.width as f32 / self.targets.height as f32;
        let proj = glam::camera::rh::proj::directx::perspective(fov, aspect, scene.camera.near, scene.camera.far);

        let fwd = (cam_target - cam_pos).normalize_or_zero();
        let roll = scene.camera.roll.sample(t).to_radians();
        let up = if roll.abs() > 1e-6 { Quat::from_axis_angle(fwd, roll) * Vec3::Y } else { Vec3::Y };
        let up = if up.length_squared() < 1e-6 { Vec3::Z } else { up };
        let view = glam::camera::rh::view::look_at_mat4(cam_pos, cam_target, up);
        let view_proj = proj * view;

        build_globals_common(scene, t, cam_pos, view_proj)
    }

    /// Renders one frame at time `t` (seconds) and returns tightly-packed RGB8 pixels,
    /// `out_width * out_height * 3` bytes, top row first.
    pub fn render_frame(&mut self, scene: &Scene, t: f32) -> Vec<u8> {
        let globals = self.build_globals(scene, t);
        self.gpu.queue.write_buffer(&self.global_buf, 0, bytemuck::bytes_of(&globals));

        let mut transforms = Vec::with_capacity(self.meshes.len());
        collect_leaf_transforms(&scene.objects, t, Mat4::IDENTITY, &mut transforms);
        debug_assert_eq!(transforms.len(), self.meshes.len());

        for (i, (world, mat)) in transforms.iter().enumerate() {
            let normal_mat = world.inverse().transpose();
            let obj_uniform = ObjectUniform {
                model: world.to_cols_array_2d(),
                normal_mat: normal_mat.to_cols_array_2d(),
                base_color: [mat.color.x, mat.color.y, mat.color.z, 1.0],
                material: [mat.metallic, mat.roughness, 0.0, 0.0],
                emissive: [mat.emissive.x, mat.emissive.y, mat.emissive.z, 0.0],
            };
            self.gpu.queue.write_buffer(
                &self.object_buf,
                i as u64 * self.object_stride,
                bytemuck::bytes_of(&obj_uniform),
            );
        }

        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame-encoder"),
        });

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
            for (i, mesh) in self.meshes.iter().enumerate() {
                shadow_pass.set_bind_group(1, &self.object_bind_group, &[(i as u64 * self.object_stride) as u32]);
                shadow_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                shadow_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
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
            for (i, mesh) in self.meshes.iter().enumerate() {
                main_pass.set_bind_group(1, &self.object_bind_group, &[(i as u64 * self.object_stride) as u32]);
                main_pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
                main_pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                main_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.targets.color_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
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
pub(crate) fn build_globals_common(scene: &Scene, t: f32, cam_pos: Vec3, view_proj: Mat4) -> GlobalUniform {
    let mut light_pos_or_dir = [[0f32; 4]; MAX_LIGHTS];
    let mut light_color_intensity = [[0f32; 4]; MAX_LIGHTS];
    let mut shadow_idx: i32 = -1;
    let mut light_view_proj = Mat4::IDENTITY;
    let n = scene.lights.len().min(MAX_LIGHTS);
    for i in 0..n {
        let light = &scene.lights[i];
        let intensity = light.intensity.sample(t);
        let color = light.color.sample(t) * intensity;
        match &light.kind {
            LightKind::Directional { direction } => {
                let d = direction.sample(t).normalize_or_zero();
                light_pos_or_dir[i] = [d.x, d.y, d.z, 0.0];
                light_color_intensity[i] = [color.x, color.y, color.z, 0.0];
                if light.cast_shadows {
                    shadow_idx = i as i32;
                    let r = light.shadow_radius.max(0.5);
                    let light_pos = -d * (r * 1.6);
                    let up = if d.y.abs() > 0.98 { Vec3::Z } else { Vec3::Y };
                    let view_l = glam::camera::rh::view::look_at_mat4(light_pos, Vec3::ZERO, up);
                    let proj_l = glam::camera::rh::proj::directx::orthographic(-r, r, -r, r, 0.05, r * 3.5);
                    light_view_proj = proj_l * view_l;
                }
            }
            LightKind::Point { position, range } => {
                let p = position.sample(t);
                light_pos_or_dir[i] = [p.x, p.y, p.z, 1.0];
                light_color_intensity[i] = [color.x, color.y, color.z, *range];
            }
        }
    }

    let ambient = scene.ambient_color * scene.ambient_intensity;
    let (bg_top, bg_bottom, bg_mode) = match &scene.background {
        Background::Flat(c) => (*c, Vec3::ZERO, 0.0f32),
        Background::Gradient { top, bottom } => (*top, *bottom, 1.0f32),
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
        let material = Material { color: crate::track::Track::constant(Vec3::splat(0.7)), metallic: 0.0, roughness: 0.6, emissive: Vec3::ZERO };
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
}
