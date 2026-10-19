//! The block world on the GPU: one mesh per section and pass (solid, water, glass), each with a uniform slot of its own placed relative to the
//! renderer's origin. Both renderers hold it as a [`WorldLayer`]: the live client's game mode hands it the meshes the streamer made
//! ([`VoxelLayer::apply`]) and the things that move ([`VoxelLayer::set_models`]); a still picture's ([`StillCraft`]) streams a world of its own.

use crate::chunk::ChunkPos;
use crate::mesh::SectionMesh;
use crate::stream::{MeshUpdates, Terrain};
use glam::{Mat4, Vec3, Vec4};
use red_engine2::gpu::{GpuMesh, ObjectUniform};
use red_engine2::mesh::Mesh;
use red_engine2::object_staging::{aabb_outside_frustum, frustum_planes};
use red_engine2::stream_gpu::ShadowDetail;
use red_engine2::world_layer::{LayerDef, LayerFrame, StillLight, WorldLayer};
use std::collections::HashMap;
use std::sync::Arc;

/// Uniform slots for sections.
const SLOTS: usize = 16384;
/// Uniform slots for moving models.
const MODEL_SLOTS: usize = 512;

/// How a section's passes look.
#[derive(Clone, Copy)]
enum Look {
    Solid,
    Water,
    Glass,
    Cloud,
}

struct Part {
    mesh: GpuMesh,
    slot: u32,
}

struct Section {
    solid: Option<Part>,
    water: Option<Part>,
    glass: Option<Part>,
    corner: Vec3,
    lo: Vec3,
    hi: Vec3,
}

/// Something that moves (a mob, a dropped item, the block in hand): a mesh drawn with its own transform.
#[derive(Clone)]
pub struct Model {
    pub mesh: Arc<GpuMesh>,
    /// World transform.
    pub world: Mat4,
    /// Painted with the block shader (its vertex colours are texture numbers and light) or plain vertex colours.
    pub painted: bool,
    /// Glow added to it (a hurt flash, a lit fuse).
    pub emissive: Vec3,
}

/// The block world's draws.
pub struct VoxelLayer {
    sections: HashMap<(ChunkPos, usize), Section>,
    free: Vec<u32>,
    object_buf: wgpu::Buffer,
    model_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    model_group: wgpu::BindGroup,
    stride: u64,
    origin: Vec3,
    planes: [Vec4; 6],
    eye: Vec3,
    models: Vec<Model>,
    /// Meshes handed over since the last frame, uploaded when it is prepared (where the queue is).
    pending: Vec<MeshUpdates>,
    /// Drop every section at the next frame (another world).
    clear: bool,
    /// The clouds: their noise, the cell their mesh was built around, the mesh and where it is.
    cloud_noise: crate::noise::Noise,
    cloud_cell: Option<(i32, i32)>,
    cloud_mesh: Option<GpuMesh>,
    /// Triangles drawn by the last main pass, sections drawn.
    pub drawn: std::cell::Cell<(u64, u32)>,
}

fn uniform(world: Mat4, look: Look, painted: bool, emissive: Vec3) -> ObjectUniform {
    let (alpha, rough) = match look {
        Look::Solid => (1.0, 0.92),
        Look::Water => (0.78, 0.35),
        Look::Glass => (0.42, 0.08),
        Look::Cloud => (0.82, 1.0),
    };
    ObjectUniform {
        model: world.to_cols_array_2d(),
        normal_mat: world.inverse().transpose().to_cols_array_2d(),
        base_color: [1.0, 1.0, 1.0, alpha],
        material: [0.0, rough, if painted { 1.0 } else { 0.0 }, 0.0],
        emissive: [emissive.x, emissive.y, emissive.z, 0.0],
        surface: [if painted { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        palette: [[0.0; 4]; 4],
    }
}

fn mesh_bounds(m: &Mesh) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for v in &m.vertices {
        let p = Vec3::from(v.pos);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo, hi)
}

impl VoxelLayer {
    pub fn new(device: &wgpu::Device, object_layout: &wgpu::BindGroupLayout) -> VoxelLayer {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let size = std::mem::size_of::<ObjectUniform>() as u64;
        let stride = size.div_ceil(alignment) * alignment;
        let make = |label: &str, slots: usize| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: stride * slots as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: object_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buf, offset: 0, size: wgpu::BufferSize::new(size) }),
                }],
            });
            (buf, group)
        };
        let (object_buf, bind_group) = make("redcraft-sections", SLOTS);
        let (model_buf, model_group) = make("redcraft-models", MODEL_SLOTS);
        VoxelLayer {
            sections: HashMap::new(),
            free: (0..SLOTS as u32).rev().collect(),
            object_buf,
            model_buf,
            bind_group,
            model_group,
            stride,
            origin: Vec3::ZERO,
            planes: [Vec4::ZERO; 6],
            eye: Vec3::ZERO,
            models: Vec::new(),
            pending: Vec::new(),
            clear: false,
            cloud_noise: crate::noise::Noise::new(0xc10d),
            cloud_cell: None,
            cloud_mesh: None,
            drawn: Default::default(),
        }
    }

    /// Hands over the streamer's news; it is uploaded with the next frame.
    pub fn push(&mut self, u: MeshUpdates) {
        if !u.fresh.is_empty() || !u.gone.is_empty() {
            self.pending.push(u);
        }
    }

    /// Forgets every section (another world is coming).
    pub fn clear(&mut self) {
        self.clear = true;
        self.pending.clear();
    }

    fn write(&self, queue: &wgpu::Queue, slot: u32, corner: Vec3, look: Look) {
        let u = uniform(Mat4::from_translation(corner - self.origin), look, true, Vec3::ZERO);
        queue.write_buffer(&self.object_buf, slot as u64 * self.stride, bytemuck::bytes_of(&u));
    }

    fn release(&mut self, s: Section) {
        for p in [s.solid, s.water, s.glass].into_iter().flatten() {
            self.free.push(p.slot);
        }
    }

    fn part(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, m: &Mesh, corner: Vec3, look: Look) -> Option<Part> {
        if m.indices.is_empty() {
            return None;
        }
        let slot = self.free.pop()?;
        self.write(queue, slot, corner, look);
        Some(Part { mesh: GpuMesh::upload(device, m), slot })
    }

    /// Takes the streamer's news: uploads fresh sections (an empty one clears its place), drops the chunks that are gone.
    pub fn apply(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, u: MeshUpdates) {
        for p in u.gone {
            let keys: Vec<(ChunkPos, usize)> = self.sections.keys().copied().filter(|(c, _)| *c == p).collect();
            for k in keys {
                if let Some(s) = self.sections.remove(&k) {
                    self.release(s);
                }
            }
        }
        for (p, s, m) in u.fresh {
            self.upload(device, queue, p, s, &m);
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, p: ChunkPos, s: usize, m: &SectionMesh) {
        if let Some(old) = self.sections.remove(&(p, s)) {
            self.release(old);
        }
        if m.is_empty() {
            return;
        }
        let corner = Terrain::section_origin(p, s);
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for mesh in [&m.solid, &m.water, &m.glass] {
            if !mesh.vertices.is_empty() {
                let (a, b) = mesh_bounds(mesh);
                lo = lo.min(a);
                hi = hi.max(b);
            }
        }
        let section = Section {
            solid: self.part(device, queue, &m.solid, corner, Look::Solid),
            water: self.part(device, queue, &m.water, corner, Look::Water),
            glass: self.part(device, queue, &m.glass, corner, Look::Glass),
            corner,
            // Swaying plants reach a little past their cells.
            lo: lo - Vec3::splat(0.2),
            hi: hi + Vec3::splat(0.2),
        };
        self.sections.insert((p, s), section);
    }

    /// Tells the layer where the renderer measures from; when that moves (every couple of kilometres) every section is placed again.
    pub fn set_origin(&mut self, queue: &wgpu::Queue, origin: Vec3) {
        if origin == self.origin {
            return;
        }
        self.origin = origin;
        for s in self.sections.values() {
            for (p, look) in [(&s.solid, Look::Solid), (&s.water, Look::Water), (&s.glass, Look::Glass)] {
                if let Some(p) = p {
                    self.write(queue, p.slot, s.corner, look);
                }
            }
        }
    }

    /// The moving models for the coming frames.
    pub fn set_models(&mut self, models: Vec<Model>) {
        self.models = models;
        // The last slot is the clouds'.
        self.models.truncate(MODEL_SLOTS - 1);
    }

    /// Sections resident.
    pub fn resident(&self) -> usize {
        self.sections.len()
    }

    fn outside(&self, s: &Section, planes: &[Vec4; 6]) -> bool {
        let c = s.corner - self.origin;
        aabb_outside_frustum(c + (s.lo + s.hi) * 0.5, (s.hi - s.lo) * 0.5, planes)
    }

    fn draw_part(&self, pass: &mut wgpu::RenderPass<'_>, p: &Part, instance: std::ops::Range<u32>) {
        pass.set_bind_group(1, &self.bind_group, &[(p.slot as u64 * self.stride) as u32]);
        pass.set_vertex_buffer(0, p.mesh.vertex_buf.slice(..));
        pass.set_index_buffer(p.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..p.mesh.index_count, 0, instance);
    }

    fn prepare_frame(&mut self, f: &LayerFrame) {
        if std::mem::take(&mut self.clear) {
            let all: Vec<(ChunkPos, usize)> = self.sections.keys().copied().collect();
            for k in all {
                if let Some(s) = self.sections.remove(&k) {
                    self.release(s);
                }
            }
        }
        for u in std::mem::take(&mut self.pending) {
            self.apply(f.device, f.queue, u);
        }
        self.set_origin(f.queue, f.origin);
        self.planes = frustum_planes(f.view_proj);
        self.eye = f.eye;
        for (i, m) in self.models.iter().enumerate() {
            let u = uniform(Mat4::from_translation(-f.origin) * m.world, Look::Solid, m.painted, m.emissive);
            f.queue.write_buffer(&self.model_buf, i as u64 * self.stride, bytemuck::bytes_of(&u));
        }
        // The clouds follow the eye, drifting with the scene's time; the mesh is built again when the field moves a cell.
        let (cell, corner) = crate::clouds::place(f.eye, f.t);
        if self.cloud_cell != Some(cell) {
            self.cloud_cell = Some(cell);
            let m = crate::clouds::mesh(&self.cloud_noise, cell.0, cell.1);
            self.cloud_mesh = (!m.indices.is_empty()).then(|| GpuMesh::upload(f.device, &m));
        }
        let u = uniform(Mat4::from_translation(corner - f.origin), Look::Cloud, false, Vec3::ZERO);
        f.queue.write_buffer(&self.model_buf, (MODEL_SLOTS - 1) as u64 * self.stride, bytemuck::bytes_of(&u));
    }
}

impl WorldLayer for VoxelLayer {
    fn prepare(&mut self, f: &LayerFrame) {
        self.prepare_frame(f);
    }

    fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>, planes: &[Vec4; 6], _detail: ShadowDetail, instance: std::ops::Range<u32>) {
        for s in self.sections.values() {
            if let Some(p) = &s.solid {
                if !self.outside(s, planes) {
                    self.draw_part(pass, p, instance.clone());
                }
            }
        }
    }

    fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>, view_proj: Mat4) {
        let planes = frustum_planes(view_proj);
        let (mut tris, mut n) = (0u64, 0u32);
        for s in self.sections.values() {
            if let Some(p) = &s.solid {
                if !self.outside(s, &planes) {
                    self.draw_part(pass, p, 0..1);
                    tris += p.mesh.index_count as u64 / 3;
                    n += 1;
                }
            }
        }
        for (i, m) in self.models.iter().enumerate() {
            pass.set_bind_group(1, &self.model_group, &[(i as u64 * self.stride) as u32]);
            pass.set_vertex_buffer(0, m.mesh.vertex_buf.slice(..));
            pass.set_index_buffer(m.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..m.mesh.index_count, 0, 0..1);
        }
        self.drawn.set((tris, n));
    }

    fn draw_blended(&self, pass: &mut wgpu::RenderPass<'_>, alpha: &wgpu::RenderPipeline) {
        let mut list: Vec<(f32, &Part)> = Vec::new();
        for s in self.sections.values() {
            if (s.water.is_some() || s.glass.is_some()) && !self.outside(s, &self.planes) {
                let d = (s.corner + (s.lo + s.hi) * 0.5 - self.eye).length_squared();
                for p in [&s.water, &s.glass].into_iter().flatten() {
                    list.push((d, p));
                }
            }
        }
        pass.set_pipeline(alpha);
        // The clouds first: they are farther than anything else.
        if let Some(c) = &self.cloud_mesh {
            pass.set_bind_group(1, &self.model_group, &[((MODEL_SLOTS - 1) as u64 * self.stride) as u32]);
            pass.set_vertex_buffer(0, c.vertex_buf.slice(..));
            pass.set_index_buffer(c.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..c.index_count, 0, 0..1);
        }
        if list.is_empty() {
            return;
        }
        list.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, p) in list {
            self.draw_part(pass, p, 0..1);
        }
    }
}

/// The block world of a still picture: a world of its own, streamed around the camera before the picture is taken.
pub struct StillCraft {
    pub terrain: Terrain,
    pub layer: VoxelLayer,
    pub game_params: [[f32; 4]; 8],
}

impl WorldLayer for StillCraft {
    fn still_light(&mut self, eye: Vec3) -> Option<StillLight> {
        self.terrain.settle(eye, 20.0);
        Some(StillLight { lamps: Vec::new(), game_params: Some(self.game_params) })
    }

    fn prepare(&mut self, f: &LayerFrame) {
        if f.settle {
            self.terrain.settle(f.eye, 20.0);
        } else {
            self.terrain.update(f.eye);
        }
        let u = self.terrain.take_updates();
        self.layer.apply(f.device, f.queue, u);
        self.layer.prepare_frame(f);
    }

    fn settle(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, eyes: &[Vec3]) {
        if let Some(e) = eyes.first() {
            self.terrain.settle(*e, 20.0);
        }
        let u = self.terrain.take_updates();
        self.layer.apply(device, queue, u);
    }

    fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>, planes: &[Vec4; 6], detail: ShadowDetail, instance: std::ops::Range<u32>) {
        self.layer.draw_shadow(pass, planes, detail, instance);
    }

    fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>, view_proj: Mat4) {
        self.layer.draw_main(pass, view_proj);
    }

    fn draw_blended(&self, pass: &mut wgpu::RenderPass<'_>, alpha: &wgpu::RenderPipeline) {
        self.layer.draw_blended(pass, alpha);
    }
}

/// The block world's layer, for a scene with a `redcraft` block.
pub const LAYER: LayerDef = LayerDef {
    name: "redcraft",
    build: |scene, init| {
        let spec = crate::spec::spec_of(scene)?;
        let layer = VoxelLayer::new(init.device, init.object_layout);
        Some(if init.live {
            Box::new(layer) as Box<dyn WorldLayer>
        } else {
            let world = crate::world::World::new(spec.seed.value(None));
            let view = still_view();
            Box::new(StillCraft { terrain: Terrain::new(world, view, crate::stream::live_workers()), layer, game_params: [[0.0; 4]; 8] })
        })
    },
    art: |scene| scene.blocks.get::<crate::spec::CraftSpec>().and_then(|m| m.art.as_deref()),
};

/// How far a still picture's world reaches (`REDCRAFT_VIEW`, default 6 chunks).
fn still_view() -> i32 {
    std::env::var("REDCRAFT_VIEW").ok().and_then(|v| v.parse().ok()).unwrap_or(6).clamp(1, 16)
}
