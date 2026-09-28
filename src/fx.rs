//! The screen-effects pass: [`FxParams`] (what [`crate::feel::Feel`] decided to show this frame) drawn as one fullscreen triangle over the
//! finished frame (`shaders/fx.wgsl`): a vignette, a damage arc, flashes and the hit marker. It owns its uniform buffer, so a caller only
//! hands it the parameters each frame.

use crate::feel::FxParams;
use bytemuck::{Pod, Zeroable};

/// The uniform the shader reads. Field for field the same as `struct Fx` in `fx.wgsl` (a test checks the size).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct FxUniform {
    /// Viewport pixels (x, y), pixel scale (height / 1080), unused.
    pub res: [f32; 4],
    /// Vignette rgb and strength.
    pub vignette: [f32; 4],
    /// Flash rgb and alpha.
    pub flash: [f32; 4],
    /// Attacker direction relative to the view, strength, unused, unused.
    pub hurt: [f32; 4],
    /// Marker age, kind, unused, unused.
    pub marker: [f32; 4],
}

impl FxUniform {
    /// The uniform for `params` on a `width` x `height` target.
    pub fn new(params: &FxParams, width: u32, height: u32) -> FxUniform {
        FxUniform {
            res: [width as f32, height as f32, height as f32 / 1080.0, 0.0],
            vignette: params.vignette,
            flash: params.flash,
            hurt: [params.hurt_angle, params.hurt_strength, 0.0, 0.0],
            marker: [params.marker_age, params.marker_kind, 0.0, 0.0],
        }
    }
}

/// The pipeline and its uniform.
pub struct FxPipeline {
    pipeline: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl FxPipeline {
    /// Builds the pipeline for a single-sampled target of `color_format` (the swapchain).
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> FxPipeline {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fx-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<FxUniform>() as u64),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fx-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fx-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fx.wgsl").into()),
        });
        // The shader writes premultiplied colour: result = src + dst * (1 - src.a).
        let premultiplied_over = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fx-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_fx"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_fx"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(premultiplied_over), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fx-uniform"),
            size: std::mem::size_of::<FxUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fx-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        FxPipeline { pipeline, buffer, bind_group }
    }

    /// Uploads this frame's parameters.
    pub fn update(&self, queue: &wgpu::Queue, params: &FxParams, width: u32, height: u32) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&FxUniform::new(params, width, height)));
    }

    /// Draws the effects over whatever is in the pass's target.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniform_has_one_vec4_per_field_of_the_shader_struct() {
        let src = include_str!("shaders/fx.wgsl");
        let start = src.find("struct Fx {").expect("struct Fx");
        let body = &src[start..start + src[start..].find("};").expect("end of struct")];
        let fields = body.lines().filter(|l| l.trim_start().split("//").next().is_some_and(|c| c.contains("vec4<f32>"))).count();
        assert_eq!(fields * 16, std::mem::size_of::<FxUniform>(), "shaders/fx.wgsl declares {fields} vec4 fields");
    }

    #[test]
    fn the_uniform_carries_the_parameters_and_scales_with_the_window_height() {
        let p =
            FxParams { vignette: [1.0, 0.0, 0.0, 0.5], flash: [0.0, 1.0, 0.0, 0.25], hurt_angle: 0.5, hurt_strength: 0.75, marker_age: 0.2, marker_kind: 1.0 };
        let u = FxUniform::new(&p, 1920, 1080);
        assert_eq!(u.res, [1920.0, 1080.0, 1.0, 0.0]);
        assert_eq!((u.vignette, u.flash, u.hurt[..2].to_vec(), u.marker[..2].to_vec()), (p.vignette, p.flash, vec![0.5, 0.75], vec![0.2, 1.0]));
        assert_eq!(FxUniform::new(&p, 960, 540).res[2], 0.5, "half-size window, half-size marker");
    }
}
