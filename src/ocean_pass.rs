//! The GPU half of the scene's `ocean` block: one fullscreen pass that draws the water plane after the opaque world, depth-tested against
//! it (`shaders/ocean.wgsl`). It owns the water's uniform and a buffer of the scene's first terrain's heights, which is how the water
//! knows how deep it is at every pixel (shallow and translucent over a shore, foam at the waterline, deep offshore).
//!
//! Both renderers (`viewer::LiveRenderer`, `render::Renderer`) build an [`OceanPass`] when the scene has an ocean and call
//! [`OceanPass::draw`] at the end of their main pass; a scene without an ocean pays nothing.

use crate::atmosphere::Ocean;
use crate::expanse::Axis;
use crate::schema::{Object, ObjectKind, Scene};
use crate::terrain::Terrain;
use bytemuck::{Pod, Zeroable};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// The water's numbers as the shader reads them (mirrors `OceanU` in `shaders/ocean.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct OceanUniform {
    /// Water level y, wave amplitude, wave frequency (rad/m), speed.
    pub wave: [f32; 4],
    /// Deep colour, roughness.
    pub deep: [f32; 4],
    /// Shallow colour, depth scale (m).
    pub shallow: [f32; 4],
    /// Foam colour, haze distance (m).
    pub foam: [f32; 4],
    /// Terrain origin x, origin z, cell x, cell z.
    pub terr: [f32; 4],
    /// Terrain samples x, samples z, periodic axis (0 none, 1 x, 2 z), has terrain.
    pub terr2: [f32; 4],
    /// Seconds.
    pub time: [f32; 4],
}

/// The first terrain in the scene (the ocean reads its heights), at any depth of grouping.
pub fn first_terrain(objects: &[Object]) -> Option<Arc<Terrain>> {
    for o in objects {
        match &o.kind {
            ObjectKind::Terrain(t) => return Some(t.terrain.clone()),
            ObjectKind::Group(children) => {
                if let Some(t) = first_terrain(children) {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

/// The uniform for `ocean` over `terrain` (if any) at `time` seconds.
pub fn uniform(ocean: &Ocean, terrain: Option<&Terrain>, time: f32) -> OceanUniform {
    let (terr, terr2) = match terrain {
        Some(t) => (
            [t.origin.x, t.origin.y, t.cell.x, t.cell.y],
            [
                t.nx as f32,
                t.nz as f32,
                match t.periodic {
                    None => 0.0,
                    Some(Axis::X) => 1.0,
                    Some(Axis::Z) => 2.0,
                },
                1.0,
            ],
        ),
        None => ([0.0; 4], [1.0, 1.0, 0.0, 0.0]),
    };
    OceanUniform {
        wave: [ocean.y, ocean.wave_amplitude, ocean.wave_frequency, ocean.wave_speed],
        deep: [ocean.deep.x, ocean.deep.y, ocean.deep.z, ocean.roughness],
        shallow: [ocean.shallow.x, ocean.shallow.y, ocean.shallow.z, ocean.depth_scale],
        foam: [ocean.foam.x, ocean.foam.y, ocean.foam.z, ocean.haze_distance],
        terr,
        terr2,
        time: [time, 0.0, 0.0, 0.0],
    }
}

/// The ocean's pipeline, uniform and terrain-height texture.
pub struct OceanPass {
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    ocean: Ocean,
    terrain: Option<Arc<Terrain>>,
}

impl OceanPass {
    /// Builds the pass for `scene`, or `None` when the scene has no `ocean`. `global_layout` is the uniform-only globals layout
    /// (`gpu::BindLayouts::global_uniform`); `sample_count` must match the main pass's colour target.
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        sample_count: u32,
        scene: &Scene,
        global_layout: &wgpu::BindGroupLayout,
    ) -> Option<Self> {
        let ocean = scene.ocean?;
        let terrain = first_terrain(&scene.objects);
        // The terrain's heights (one float per sample, row-major) in a read-only storage buffer; a scene without terrain gets one dummy float.
        let flat = [0.0f32];
        let heights: &[f32] = terrain.as_ref().map_or(&flat, |t| &t.heights);
        let height_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean-terrain-heights"),
            contents: bytemuck::cast_slice(heights),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean-uniform"),
            contents: bytemuck::bytes_of(&uniform(&ocean, terrain.as_deref(), 0.0)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<OceanUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean-bind-group"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: height_buf.as_entire_binding() },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean-shader"),
            source: wgpu::ShaderSource::Wgsl(
                concat!(include_str!("shaders/common.wgsl"), include_str!("shaders/sky.wgsl"), include_str!("shaders/ocean.wgsl")).into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean-pipeline-layout"),
            bind_group_layouts: &[Some(global_layout), Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ocean-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_ocean"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ocean"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // LessEqual: water beyond the far plane writes depth 1.0, exactly the cleared value.
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: sample_count, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        });
        Some(OceanPass { pipeline, uniform_buf, bind_group, ocean, terrain })
    }

    /// Writes the animation clock (seconds) for the next draw.
    pub fn update(&self, queue: &wgpu::Queue, time: f32) {
        queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniform(&self.ocean, self.terrain.as_deref(), time)));
    }

    /// Draws the water into a main pass that already holds the opaque world; `global` is the uniform-only globals bind group.
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, global: &'a wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, global, &[]);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rust_uniform_matches_the_wgsl_struct() {
        // 7 vec4s, no padding: the shader declares exactly these seven fields.
        assert_eq!(std::mem::size_of::<OceanUniform>(), 7 * 16);
        let src = include_str!("shaders/ocean.wgsl");
        let start = src.find("struct OceanU {").expect("OceanU in ocean.wgsl");
        let body = &src[start..src[start..].find("};").map_or(src.len(), |e| start + e)];
        assert_eq!(body.matches(": vec4<f32>").count(), 7, "OceanU fields");
    }
}
