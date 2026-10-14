//! The museum on the GPU: every model the visit has built ([`crate::museum::state::Museum::models`]) uploaded once, its animated pieces placed every frame.
//!
//! A scene with a `museum` block gets one of these in each renderer. Like the streamed world (`stream_gpu`) it owns its object-uniform buffer: the client
//! hands it the visit each frame ([`MuseumLayer::sync`]: new models are uploaded, gone ones dropped, every piece's world matrix and material worked out,
//! sculptures moved by their [`crate::museum::sculpt::Motion`]s, doors swung by how open they are), and the renderer writes the uniforms relative to its
//! origin, culls, and draws ([`MuseumLayer::prepare`], [`MuseumLayer::draw_main`]). The renderers hold it as a [`WorldLayer`] ([`LAYER`]); a still
//! picture's carries a visit of its own ([`StillMuseum`]).

use crate::gpu::{GpuMesh, ObjectUniform};
use crate::mesh::{Mesh, Vertex};
use crate::museum::build::{key_geo, Anim, Look, Model};
use crate::museum::sculpt::{part_transform, Stir};
use crate::museum::state::{ModelKey, Museum};
use crate::object_staging::{aabb_outside_frustum, frustum_planes};
use crate::procgen::geo::Geo;
use crate::world_layer::{LayerDef, LayerFrame, StillLight, WorldLayer};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

/// Draw slots in the uniform buffer.
const SLOTS: usize = 6000;

struct Resident {
    model: Arc<Model>,
    meshes: Vec<Option<GpuMesh>>,
}

/// One draw this frame: which mesh, where (world space, before the origin shift), how it looks.
struct Item {
    key: usize,
    piece: usize,
    world: Mat4,
    look: Look,
    /// World-space bounds for culling.
    centre: Vec3,
    half: Vec3,
}

/// The museum's draws.
pub struct MuseumLayer {
    resident: Vec<(ModelKey, Resident)>,
    key_mesh: GpuMesh,
    items: Vec<Item>,
    /// The floating key, when one floats in the halls (rising out of a sculpture).
    key_item: Option<(Mat4, Look)>,
    object_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    stride: u64,
    staged: Vec<u8>,
    visible: Vec<u32>,
    generation: u64,
}

fn to_mesh(g: &Geo) -> Mesh {
    Mesh { vertices: (0..g.pos.len()).map(|i| Vertex { pos: g.pos[i], normal: g.nrm[i], color: g.col[i], sway: g.sway[i] }).collect(), indices: g.idx.clone() }
}

fn uniform(world: Mat4, look: &Look) -> ObjectUniform {
    ObjectUniform {
        model: world.to_cols_array_2d(),
        normal_mat: world.inverse().transpose().to_cols_array_2d(),
        base_color: [1.0, 1.0, 1.0, 1.0],
        material: [look.metallic, look.roughness, 0.0, 0.0],
        emissive: [look.emissive.x, look.emissive.y, look.emissive.z, 0.0],
        surface: look.surface,
        palette: look.palette,
    }
}

impl MuseumLayer {
    pub fn new(device: &wgpu::Device, object_layout: &wgpu::BindGroupLayout) -> MuseumLayer {
        let alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let size = std::mem::size_of::<ObjectUniform>() as u64;
        let stride = size.div_ceil(alignment) * alignment;
        let object_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("museum-object-uniforms"),
            size: stride * SLOTS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("museum-object-bind-group"),
            layout: object_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &object_buf, offset: 0, size: wgpu::BufferSize::new(size) }),
            }],
        });
        MuseumLayer {
            resident: Vec::new(),
            key_mesh: GpuMesh::upload(device, &to_mesh(&key_geo())),
            items: Vec::new(),
            key_item: None,
            object_buf,
            bind_group,
            stride,
            staged: Vec::new(),
            visible: Vec::new(),
            generation: u64::MAX,
        }
    }

    /// Takes this frame's visit: uploads what is new, drops what is gone, and places every piece for time `t` with the viewer's eye at `eye` (world).
    pub fn sync(&mut self, device: &wgpu::Device, museum: &Museum, t: f32, eye: Vec3) {
        if museum.generation != self.generation {
            self.generation = museum.generation;
            let wanted = museum.models();
            let mut kept: Vec<(ModelKey, Resident)> = Vec::new();
            let mut old: HashMap<ModelKey, Resident> = self.resident.drain(..).collect();
            for (key, model) in wanted {
                let res = match old.remove(&key) {
                    Some(r) if Arc::ptr_eq(&r.model, &model) => r,
                    _ => Resident {
                        meshes: model.pieces.iter().map(|p| (!p.geo.idx.is_empty()).then(|| GpuMesh::upload(device, &to_mesh(&p.geo)))).collect(),
                        model,
                    },
                };
                kept.push((key, res));
            }
            self.resident = kept;
        }
        self.items.clear();
        for (k, (key, res)) in self.resident.iter().enumerate() {
            let m = &res.model;
            let place = Mat4::from_translation(m.origin);
            for (i, p) in m.pieces.iter().enumerate() {
                let world = match p.anim {
                    Anim::Still => place * p.local,
                    Anim::Drift { phase } => place * Mat4::from_translation(Vec3::new(0.0, 0.08 * (t * 0.7 + phase).sin(), 0.0)) * p.local,
                    Anim::Key => continue,
                    Anim::Leaf { hinge, swing } => {
                        let open = match key {
                            ModelKey::Edge(e) => museum.door_openness(*e),
                            _ => 0.0,
                        };
                        // Eased, so a door starts and ends its swing gently.
                        let k = open * open * (3.0 - 2.0 * open);
                        place * Mat4::from_translation(hinge) * Mat4::from_rotation_y(swing * k) * Mat4::from_translation(-hinge) * p.local
                    }
                    Anim::Part { exhibit, part } => {
                        let Some(rig) = m.rig(exhibit) else { continue };
                        let room = match key {
                            ModelKey::Room(r) => *r,
                            _ => continue,
                        };
                        let base = place * rig.base;
                        let eye_local = base.inverse().transform_point3(eye);
                        let near = (1.0 - (Vec3::new(eye_local.x, 0.0, eye_local.z).length() - 1.2) / 4.5).clamp(0.0, 1.0);
                        let (since_touch, reveal) = museum.stir_of(room, exhibit);
                        let stir = Stir { near, eye: eye_local, since_touch, reveal };
                        base * part_transform(&rig.parts, part as usize, t, &stir)
                    }
                };
                let (lo, hi) = bounds(&res.meshes[i]);
                let (centre, half) = world_box(world, lo, hi);
                self.items.push(Item { key: k, piece: i, world, look: p.look, centre, half });
            }
        }
        self.key_item = museum.key_at().map(|at| {
            let world = Mat4::from_translation(at)
                * Mat4::from_rotation_y(t * 1.4)
                * Mat4::from_rotation_z(0.35 * (t * 0.9).sin())
                * Mat4::from_scale(Vec3::splat(1.6));
            (
                world,
                Look {
                    metallic: 0.9,
                    roughness: 0.25,
                    emissive: Vec3::new(0.95, 0.66, 0.2) * (0.55 + 0.25 * (t * 3.0).sin()),
                    surface: [0.0; 4],
                    palette: [[0.0; 4]; 4],
                },
            )
        });
    }

    /// Writes this frame's uniforms relative to the renderer's `origin` and works out what the camera (`view_proj`, origin-relative) can see.
    pub fn prepare(&mut self, queue: &wgpu::Queue, origin: Vec3, view_proj: Mat4) {
        let planes = frustum_planes(view_proj);
        let shift = Mat4::from_translation(-origin);
        self.visible.clear();
        self.staged.clear();
        let stride = self.stride as usize;
        let push = |staged: &mut Vec<u8>, u: ObjectUniform| {
            let start = staged.len();
            staged.extend_from_slice(bytemuck::bytes_of(&u));
            staged.resize(start + stride, 0);
        };
        for (n, item) in self.items.iter().enumerate() {
            if self.visible.len() + 1 >= SLOTS {
                break;
            }
            if aabb_outside_frustum(item.centre - origin, item.half, &planes) {
                continue;
            }
            push(&mut self.staged, uniform(shift * item.world, &item.look));
            self.visible.push(n as u32);
        }
        if let Some((world, look)) = &self.key_item {
            push(&mut self.staged, uniform(shift * *world, look));
        }
        if !self.staged.is_empty() {
            queue.write_buffer(&self.object_buf, 0, &self.staged);
        }
    }

    /// Draws what [`Self::prepare`] found visible (`pass` has the main pipeline and global bindings set).
    pub fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>) {
        for (slot, n) in self.visible.iter().enumerate() {
            let item = &self.items[*n as usize];
            let Some(mesh) = self.resident[item.key].1.meshes[item.piece].as_ref() else { continue };
            pass.set_bind_group(1, &self.bind_group, &[(slot as u64 * self.stride) as u32]);
            pass.set_vertex_buffer(0, mesh.vertex_buf.slice(..));
            pass.set_index_buffer(mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
        if self.key_item.is_some() {
            let slot = self.visible.len();
            pass.set_bind_group(1, &self.bind_group, &[(slot as u64 * self.stride) as u32]);
            pass.set_vertex_buffer(0, self.key_mesh.vertex_buf.slice(..));
            pass.set_index_buffer(self.key_mesh.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.key_mesh.index_count, 0, 0..1);
        }
    }

    /// Draws and models resident (for tests and the debug overlay).
    pub fn stats(&self) -> (usize, usize, usize) {
        (self.resident.len(), self.items.len(), self.visible.len())
    }
}

fn bounds(mesh: &Option<GpuMesh>) -> (Vec3, Vec3) {
    mesh.as_ref().map_or((Vec3::ZERO, Vec3::ZERO), |m| (m.local_min, m.local_max))
}

/// The world box of a local box under `m`.
fn world_box(m: Mat4, lo: Vec3, hi: Vec3) -> (Vec3, Vec3) {
    let c = m.transform_point3((lo + hi) * 0.5);
    let h = (hi - lo) * 0.5;
    let ax = m.x_axis.truncate().abs() * h.x;
    let ay = m.y_axis.truncate().abs() * h.y;
    let az = m.z_axis.truncate().abs() * h.z;
    // A little slack: animated pieces move within their boxes between frames.
    (c, ax + ay + az + Vec3::splat(0.3))
}

/// The museum as a world layer of a live renderer: the client's museum mode hands it the visit every frame ([`MuseumLayer::sync`], reached with
/// `LiveRenderer::layer_mut`).
impl WorldLayer for MuseumLayer {
    fn prepare(&mut self, f: &LayerFrame) {
        MuseumLayer::prepare(self, f.queue, f.origin, f.view_proj);
    }

    fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>, _view_proj: Mat4) {
        MuseumLayer::draw_main(self, pass);
    }
}

/// The museum of a still picture: a visit of its own, standing where the camera is (`museum` tool steps it into a painting first, reaching it with
/// `Renderer::layer_mut`).
pub struct StillMuseum {
    pub visit: Museum,
    layer: MuseumLayer,
}

impl WorldLayer for StillMuseum {
    /// A museum lights itself: its nearest lamps, and its shader numbers for the game's shaders.
    fn still_light(&mut self, eye: Vec3) -> Option<StillLight> {
        self.visit.settle_at(eye);
        let lamps = self.visit.lamps(eye, 12).iter().map(|l| (l.at, Vec3::from(l.color) * l.intensity, l.range)).collect();
        Some(StillLight { lamps, game_params: Some(self.visit.shader_params()) })
    }

    fn prepare(&mut self, f: &LayerFrame) {
        self.layer.sync(f.device, &self.visit, f.t, f.eye);
        self.layer.prepare(f.queue, f.origin, f.view_proj);
    }

    fn draw_main(&self, pass: &mut wgpu::RenderPass<'_>, _view_proj: Mat4) {
        self.layer.draw_main(pass);
    }
}

/// The museum's layer, for a scene with a `museum` block (`scene_blocks::register` defines it).
pub const LAYER: LayerDef = LayerDef {
    name: "museum",
    build: |scene, init| {
        let spec = crate::museum::spec_of(scene)?;
        let layer = MuseumLayer::new(init.device, init.object_layout);
        Some(if init.live {
            Box::new(layer) as Box<dyn WorldLayer>
        } else {
            Box::new(StillMuseum { visit: Museum::new(spec.clone(), spec.seed.value(None), None), layer })
        })
    },
    art: |scene| scene.blocks.get::<crate::museum::MuseumSpec>().and_then(|m| m.art.as_deref()),
};
