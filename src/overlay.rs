//! A full-screen 2-D image drawn over the finished frame: the launch menu's text and panels.
//!
//! The engine has no text rendering, so 2-D screens are painted on the CPU (see `crate::menu`)
//! into an RGBA image the size of the window and shown by this one alpha-blended textured
//! triangle. The image is only re-uploaded when it changes (a hover, a resize), so a static
//! screen costs one texture sample per pixel and nothing else.

/// The overlay pipeline plus the current image, if any.
pub struct Overlay {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    slot: Option<Slot>,
}

struct Slot {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

impl Overlay {
    /// Builds the pipeline for a single-sampled target of `color_format` (the swapchain).
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("overlay-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/overlay.wgsl").into()),
        });
        let alpha_over = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_overlay"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_overlay"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(alpha_over), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("overlay-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Overlay { pipeline, layout, sampler, slot: None }
    }

    /// Shows `rgba` (straight alpha, sRGB-encoded, `width * height * 4` bytes, top row first).
    pub fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, rgba: &[u8]) {
        debug_assert_eq!(rgba.len() as u64, width as u64 * height as u64 * 4);
        if self.slot.as_ref().map(|s| s.size) != Some((width, height)) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("overlay-texture"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("overlay-bind-group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            self.slot = Some(Slot { texture, bind_group, size: (width, height) });
        }
        if let Some(slot) = &self.slot {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &slot.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
        }
    }

    /// Stops drawing the overlay (the texture is kept for reuse).
    pub fn hide(&mut self) {
        self.slot = None;
    }

    /// True while an image is set.
    pub fn visible(&self) -> bool {
        self.slot.is_some()
    }

    /// Draws the image over whatever is in the pass's target (no-op when nothing is set).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(slot) = &self.slot {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &slot.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
