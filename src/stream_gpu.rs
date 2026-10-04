//! The streamed world on the GPU: every chunk the [`Streamer`] has baked, uploaded once, culled and drawn.
//!
//! A scene with a `procgen` block gets one of these in each renderer. It owns its own object-uniform buffer (one slot per draw, written when the chunk
//! arrives and never again, because chunks do not move), so it needs nothing from the renderer's object slots: the renderer hands it the render pass and the
//! frustum planes and it issues the draws. A chunk is up to three draws (ground, solid plants, flora). The ground is not drawn in the shadow pass. Which plants are
//! depends on the cascade ([`ShadowDetail`]): the far map holds only trees, the middle one trees and shrubs, the nearest also the squares of flowers and grass it covers.
//!
//! Live, [`StreamLayer::update`] is called every frame: it asks the streamer (which builds on worker threads) for what changed and uploads a few chunks.
//! Offline, [`StreamLayer::fill`] builds everything around the camera first, because a still frame cannot wait.

use crate::gpu::{GpuMesh, ObjectUniform};
use crate::mesh::{Mesh, Vertex};
use crate::object_staging::{aabb_outside_frustum, frustum_planes};
use crate::procgen::chunk::Chunk;
use crate::procgen::geo::Geo;
use crate::procgen::motes::{motes, MoteKind, MAX_MOTES};
use crate::procgen::stream::{Stats, Update};
use crate::procgen::{ChunkId, Config, Streamer, View};
use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;

/// Draw slots in the uniform buffer: three per chunk, for more chunks than any view distance reaches.
const SLOTS: u32 = 3 * 400;

/// One uploaded part of a chunk.
struct Part {
    mesh: GpuMesh,
    slot: u32,
    /// Runs of the index buffer that matter on their own: the trees of a solid part (one run), the squares of a flora part (one each, in grid order). Empty otherwise.
    runs: Vec<(u32, u32)>,
}

/// What a shadow cascade draws (see `shadow.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowDetail {
    /// Only the trees: the far map, whose texels are too coarse for anything smaller.
    Trees,
    /// Trees and shrubs.
    Solid,
    /// Trees, shrubs, and the squares of flowers and grass that touch the map's box: the nearest map.
    SolidAndFlora,
}

/// A chunk on the GPU.
struct Resident {
    ground: Option<Part>,
    solid: Option<Part>,
    flora: Option<Part>,
    /// The centre (relative to the chunk's corner) and half-extent of everything in it, for culling.
    centre: Vec3,
    half: Vec3,
    tris: u32,
    /// The chunk's corner in the world (x, z), for placing it relative to the renderer's origin.
    corner: Vec3,
}

/// What the layer drew last frame, for tests and the debug overlay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// Chunks resident on the GPU.
    pub resident: usize,
    /// Draw calls issued in the last main pass.
    pub draws: u32,
    /// Triangles those draws hold.
    pub tris: u64,
    /// Triangles drawn into the shadow cascades for that frame.
    pub shadow_tris: u64,
}

/// The streamed world.
pub struct StreamLayer {
    streamer: Streamer,
    resident: HashMap<ChunkId, Resident>,
    free: Vec<u32>,
    object_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    stride: u64,
    /// The point the renderer measures from; chunks are placed relative to it.
    origin: Vec3,
    drawn: std::cell::Cell<DrawStats>,
    /// Triangles drawn into the shadow cascades since the last main pass (reset when the main pass starts).
    shadow_tris: std::cell::Cell<u64>,
    lights: MoteLights,
}

/// The fireflies and pollen: one small glowing mesh drawn once per mote, billboarded to the camera, each with its own uniform (so its own brightness).
struct MoteLights {
    mesh: GpuMesh,
    buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Slots in use this frame, far to near.
    live: u32,
}

impl MoteLights {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, stride: u64) -> MoteLights {
        // Concentric twelve-sided discs facing +Z (radii 1 to 5.6): drawn additively at a low strength each they stack into a bright core and a soft halo.
        let mut mesh = Mesh::default();
        for radius in [1.0f32, 1.5, 2.2, 3.1, 4.2, 5.6] {
            let first = mesh.vertices.len() as u32;
            mesh.vertices.push(Vertex::colored([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 3]));
            for k in 0..12 {
                let a = k as f32 / 12.0 * std::f32::consts::TAU;
                mesh.vertices.push(Vertex::colored([a.cos() * radius, a.sin() * radius, 0.0], [0.0, 0.0, 1.0], [1.0; 3]));
            }
            for k in 0..12 {
                mesh.indices.extend_from_slice(&[first, first + 1 + k, first + 1 + (k + 1) % 12]);
            }
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mote-object-uniforms"),
            size: stride * MAX_MOTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mote-object-bind-group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
                }),
            }],
        });
        MoteLights { mesh: GpuMesh::upload(device, &mesh), buf, bind_group, live: 0 }
    }
}

/// A geometry as an engine mesh.
fn to_mesh(g: &Geo) -> Mesh {
    Mesh { vertices: (0..g.pos.len()).map(|i| Vertex { pos: g.pos[i], normal: g.nrm[i], color: g.col[i], sway: g.sway[i] }).collect(), indices: g.idx.clone() }
}

impl StreamLayer {
    /// A layer for a scene's `procgen` settings. `threads` workers build chunks (0 builds them inside `update`).
    pub fn new(device: &wgpu::Device, object_layout: &wgpu::BindGroupLayout, cfg: Config, view: View, threads: usize) -> StreamLayer {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let stride = (std::mem::size_of::<ObjectUniform>() as u64).div_ceil(alignment) * alignment;
        let object_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("stream-object-uniforms"),
            size: stride * SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("stream-object-bind-group"),
            layout: object_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &object_buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
                }),
            }],
        });
        let lights = MoteLights::new(device, object_layout, stride);
        StreamLayer {
            lights,
            streamer: Streamer::new(cfg, view, threads),
            resident: HashMap::new(),
            free: (0..SLOTS).rev().collect(),
            object_buf,
            bind_group,
            stride,
            origin: Vec3::ZERO,
            drawn: Default::default(),
            shadow_tris: Default::default(),
        }
    }

    /// How far the world reaches, metres.
    pub fn set_view_distance(&mut self, metres: f32) {
        self.streamer.set_view(View { distance: metres.clamp(60.0, 600.0) });
    }

    /// The streamer's counters.
    pub fn stream_stats(&self) -> Stats {
        self.streamer.stats()
    }

    /// Lights this frame's fireflies and pollen for a camera at `eye` (world position) looking along `forward`: only in a scene with a `clock` (they follow the
    /// height of the sun), and not at all when the sun is high at night's end or low at noon, so a daylight frame stays clean.
    pub fn update_motes(&mut self, queue: &wgpu::Queue, eye: Vec3, forward: Vec3, t: f32, clock: Option<&crate::daycycle::Clock>) {
        let Some(clock) = clock else {
            self.lights.live = 0;
            return;
        };
        let sun = clock.state(t, 0).sun_elev_deg;
        let mut list = motes(self.streamer.world(), [eye.x as f64, eye.y as f64, eye.z as f64], t, sun);
        // Far to near, so the glows blend over one another correctly.
        let d2 = |m: &crate::procgen::motes::Mote| (m.pos[0] - eye.x as f64).powi(2) + (m.pos[1] - eye.y as f64).powi(2) + (m.pos[2] - eye.z as f64).powi(2);
        list.sort_by(|a, b| d2(b).total_cmp(&d2(a)));
        let right = forward.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward);
        for (slot, m) in list.iter().enumerate() {
            let pos = Vec3::new((m.pos[0] - self.origin.x as f64) as f32, m.pos[1] as f32, (m.pos[2] - self.origin.z as f64) as f32);
            let (scale, colour, alpha) = match m.kind {
                MoteKind::Firefly => (m.size * 0.75, Vec3::new(0.95, 1.0, 0.3) * 0.7, 1.0),
                MoteKind::Pollen => (m.size * 1.0, Vec3::new(1.0, 0.93, 0.7) * 0.35, 1.0),
            };
            let model = Mat4::from_cols(right.extend(0.0) * scale, up.extend(0.0) * scale, (-forward).extend(0.0) * scale, pos.extend(1.0));
            let uniform = ObjectUniform {
                model: model.to_cols_array_2d(),
                normal_mat: Mat4::IDENTITY.to_cols_array_2d(),
                base_color: [0.0, 0.0, 0.0, alpha],
                material: [0.0, 1.0, 0.0, 0.0],
                emissive: [colour.x * m.glow, colour.y * m.glow, colour.z * m.glow, 0.0],
            };
            queue.write_buffer(&self.lights.buf, slot as u64 * self.stride, bytemuck::bytes_of(&uniform));
        }
        self.lights.live = list.len() as u32;
    }

    /// Draws the lights (`pass` has the main bindings; `alpha` is the blended pipeline, which does not write depth).
    pub fn draw_motes(&self, pass: &mut wgpu::RenderPass<'_>, alpha: &wgpu::RenderPipeline) {
        if self.lights.live == 0 {
            return;
        }
        pass.set_pipeline(alpha);
        pass.set_vertex_buffer(0, self.lights.mesh.vertex_buf.slice(..));
        pass.set_index_buffer(self.lights.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        for slot in 0..self.lights.live {
            pass.set_bind_group(1, &self.lights.bind_group, &[(slot as u64 * self.stride) as u32]);
            pass.draw_indexed(0..self.lights.mesh.index_count, 0, 0..1);
        }
    }

    /// What the last main pass drew.
    pub fn draw_stats(&self) -> DrawStats {
        DrawStats { resident: self.resident.len(), ..self.drawn.get() }
    }

    /// Streams for a viewer at `eye`: uploads what the workers have finished and forgets what is behind.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, eyes: &[Vec3]) {
        let eyes: Vec<(f64, f64)> = eyes.iter().map(|e| (e.x as f64, e.z as f64)).collect();
        let u = self.streamer.update_many(&eyes, 4);
        self.apply(device, queue, u);
    }

    /// Builds and uploads everything wanted around `eye` before returning (a still frame).
    pub fn fill(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, eyes: &[Vec3]) {
        let eyes: Vec<(f64, f64)> = eyes.iter().map(|e| (e.x as f64, e.z as f64)).collect();
        let u = self.streamer.fill_many(&eyes);
        self.apply(device, queue, u);
    }

    fn release(&mut self, id: ChunkId) {
        if let Some(r) = self.resident.remove(&id) {
            for p in [r.ground, r.solid, r.flora].into_iter().flatten() {
                self.free.push(p.slot);
            }
        }
    }

    fn write_uniform(&self, queue: &wgpu::Queue, slot: u32, corner: Vec3) {
        // Relative to the renderer's origin, so far out the numbers on the GPU stay small.
        let model = Mat4::from_translation(corner - self.origin);
        let uniform = ObjectUniform {
            model: model.to_cols_array_2d(),
            normal_mat: Mat4::IDENTITY.to_cols_array_2d(),
            base_color: [1.0, 1.0, 1.0, 1.0],
            material: [0.0, 0.93, 0.0, 0.0],
            emissive: [0.0; 4],
        };
        queue.write_buffer(&self.object_buf, slot as u64 * self.stride, bytemuck::bytes_of(&uniform));
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, g: &Geo, corner: Vec3, runs: Vec<(u32, u32)>) -> Option<Part> {
        if g.idx.is_empty() {
            return None;
        }
        let slot = self.free.pop()?;
        self.write_uniform(queue, slot, corner);
        Some(Part { mesh: GpuMesh::upload(device, &to_mesh(g)), slot, runs })
    }

    /// Tells the layer where the renderer measures from; when that moves (every couple of kilometres of walking) every chunk is re-placed.
    pub fn set_origin(&mut self, queue: &wgpu::Queue, origin: Vec3) {
        if origin == self.origin {
            return;
        }
        self.origin = origin;
        for r in self.resident.values() {
            for p in [&r.ground, &r.solid, &r.flora].into_iter().flatten() {
                self.write_uniform(queue, p.slot, r.corner);
            }
        }
    }

    fn apply(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, update: Update) {
        for id in update.removed {
            self.release(id);
        }
        for c in update.added {
            self.release(c.id);
            let r = self.make(device, queue, &c);
            self.resident.insert(c.id, r);
        }
    }

    fn make(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, c: &Chunk) -> Resident {
        let (ox, oz) = c.id.origin();
        let origin = Vec3::new(ox as f32, 0.0, oz as f32);
        let (lo, hi) = c.bounds;
        // Plants sway a little: pad the box by their largest reach so a swaying crown is never culled while visible.
        let pad = Vec3::splat(0.5);
        Resident {
            ground: self.upload(device, queue, &c.ground, origin, Vec::new()),
            solid: self.upload(device, queue, &c.solid, origin, vec![(0, c.solid_trees as u32)]),
            flora: self.upload(device, queue, &c.flora, origin, c.flora_cells.clone()),
            centre: (lo + hi) * 0.5,
            half: (hi - lo) * 0.5 + pad,
            tris: c.tris() as u32,
            corner: origin,
        }
    }

    /// Draws into the shadow pass for the cascade `instance` (`pass` has the shadow pipeline and the global uniform set): what `detail` says, in the chunks (and flora squares)
    /// that touch the cascade's box. Returns the triangles drawn.
    pub fn draw_shadow(&self, pass: &mut wgpu::RenderPass<'_>, light_planes: &[Vec4; 6], detail: ShadowDetail, instance: std::ops::Range<u32>) -> u64 {
        let mut tris = 0u64;
        for r in self.resident.values() {
            let centre = r.corner - self.origin + r.centre;
            if aabb_outside_frustum(centre, r.half, light_planes) {
                continue;
            }
            if let Some(p) = &r.solid {
                let count = if detail == ShadowDetail::Trees { p.runs.first().map_or(0, |t| t.1) } else { p.mesh.index_count };
                if count > 0 {
                    self.draw_range(pass, p, 0, count, instance.clone());
                    tris += count as u64 / 3;
                }
            }
            if let (ShadowDetail::SolidAndFlora, Some(p)) = (detail, &r.flora) {
                let corner = r.corner - self.origin;
                let half_cell = crate::procgen::chunk::FLORA_CELL * 0.5;
                for (k, &(start, count)) in p.runs.iter().enumerate() {
                    if count == 0 {
                        continue;
                    }
                    let (cx, cz) = ((k % crate::procgen::chunk::FLORA_CELLS) as f32, (k / crate::procgen::chunk::FLORA_CELLS) as f32);
                    let cell_centre = Vec3::new(corner.x + (cx + 0.5) * half_cell * 2.0, centre.y, corner.z + (cz + 0.5) * half_cell * 2.0);
                    // A square of flowers and grass overhangs its edges by a plant's width at most.
                    if aabb_outside_frustum(cell_centre, Vec3::new(half_cell + 1.0, r.half.y, half_cell + 1.0), light_planes) {
                        continue;
                    }
                    self.draw_range(pass, p, start, count, instance.clone());
                    tris += count as u64 / 3;
                }
            }
        }
        self.shadow_tris.set(self.shadow_tris.get() + tris);
        tris
    }

    /// Draws the ground, trees, bushes, flowers and grass into the main pass (`pass` has the main pipeline and the global bindings set).
    pub fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>, view_proj: Mat4) {
        let planes = frustum_planes(view_proj);
        let (mut draws, mut tris) = (0u32, 0u64);
        for r in self.resident.values() {
            if aabb_outside_frustum(r.corner - self.origin + r.centre, r.half, &planes) {
                continue;
            }
            for p in [&r.ground, &r.solid, &r.flora].into_iter().flatten() {
                self.draw(pass, p);
                draws += 1;
            }
            tris += r.tris as u64;
        }
        self.drawn.set(DrawStats { resident: self.resident.len(), draws, tris, shadow_tris: self.shadow_tris.replace(0) });
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, p: &Part) {
        self.draw_range(pass, p, 0, p.mesh.index_count, 0..1);
    }

    fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, p: &Part, start: u32, count: u32, instance: std::ops::Range<u32>) {
        pass.set_bind_group(1, &self.bind_group, &[(p.slot as u64 * self.stride) as u32]);
        pass.set_vertex_buffer(0, p.mesh.vertex_buf.slice(..));
        pass.set_index_buffer(p.mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(start..start + count, 0, instance);
    }
}
