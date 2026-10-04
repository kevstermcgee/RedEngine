//! Split-screen on the GPU: every local player's view drawn by the one [`LiveRenderer`] into a viewport-sized texture and composed into its rectangle of the
//! window, each with its own HUD on top.
//!
//! The renderer is sized for *one* view (every view is the same size, see [`crate::splitscreen::layout`]) and renders the players in turn into a single
//! intermediate texture; after each, a blit puts that texture into the player's rectangle of the target. That keeps every pass the engine already has
//! unchanged (shadows, haze, streaming, the post pass), a single-player window is simply the case of one rectangle covering it, and the cost is what a split screen
//! honestly costs: N scene draws of 1/N of the pixels. HUDs are painted per player at viewport size and only re-uploaded when they change.

use crate::app::camera::ViewCamera;
use crate::overlay::Overlay;
use crate::schema::Scene;
use crate::splitscreen::{layout, Layout, Rect};
use crate::viewer::{FpsLayers, LiveRenderer};

/// The colour in the gutters between views (and the empty place with three players).
const GUTTER_COLOR: wgpu::Color = wgpu::Color { r: 0.004, g: 0.005, b: 0.008, a: 1.0 };

/// One player's frame: what they see and what is laid over it.
pub struct PlayerView<'a> {
    /// Where they look from.
    pub camera: ViewCamera,
    /// The first-person layers (crosshair, viewmodel), if this player has them.
    pub layers: Option<FpsLayers>,
    /// Objects not to draw for this player (their own body in first person, what the rules hide).
    pub hidden: &'a [String],
}

/// An opaque textured copy of one texture into a rectangle of another.
struct Blit {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Blit {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Blit {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("split-blit-bgl"),
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
            label: Some("split-blit-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("split-blit-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/overlay.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("split-blit-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_overlay"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_overlay"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("split-blit-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Blit { pipeline, layout, sampler }
    }
}

/// The split-screen compositor for one window.
pub struct SplitScreen {
    blit: Blit,
    format: wgpu::TextureFormat,
    window: (u32, u32),
    gutter: u32,
    layout: Layout,
    players: usize,
    view_texture: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    huds: Vec<Overlay>,
    /// A window-sized overlay on top of everything (the pause menu, a card).
    pub global: Overlay,
}

impl SplitScreen {
    /// A compositor for `players` on a `window` of `format` (the swapchain's), with `gutter` pixels between views.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, window: (u32, u32), players: usize, gutter: u32) -> SplitScreen {
        let blit = Blit::new(device, format);
        let l = layout(players, window.0, window.1, gutter);
        let (view_texture, view, bind_group) = Self::make_view(device, &blit, format, l.size);
        SplitScreen {
            huds: (0..l.views.len()).map(|_| Overlay::new(device, format)).collect(),
            blit,
            format,
            window,
            gutter,
            players: l.views.len(),
            layout: l,
            view_texture,
            view,
            bind_group,
            global: Overlay::new(device, format),
        }
    }

    fn make_view(device: &wgpu::Device, blit: &Blit, format: wgpu::TextureFormat, size: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("split-view"),
            size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("split-blit-bind-group"),
            layout: &blit.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&blit.sampler) },
            ],
        });
        (texture, view, bind_group)
    }

    /// The layout of the views on the window.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The size the renderer must be built or resized to (one view).
    pub fn view_size(&self) -> (u32, u32) {
        self.layout.size
    }

    /// The window changed size: recomputes the layout and, if a view is a different size now, its texture. Returns whether the renderer needs [`LiveRenderer::resize`].
    pub fn resize(&mut self, device: &wgpu::Device, window: (u32, u32)) -> bool {
        self.window = window;
        let l = layout(self.players, window.0, window.1, self.gutter);
        let changed = l.size != self.layout.size;
        if changed {
            let (t, v, b) = Self::make_view(device, &self.blit, self.format, l.size);
            self.view_texture = t;
            self.view = v;
            self.bind_group = b;
            // The HUDs were painted for the old size.
            for h in &mut self.huds {
                h.hide();
            }
        }
        self.layout = l;
        changed
    }

    /// Shows player `i`'s HUD (`rgba` straight alpha, viewport sized) or hides it.
    pub fn set_hud(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, i: usize, rgba: Option<&[u8]>) {
        let (w, h) = self.layout.size;
        if let Some(overlay) = self.huds.get_mut(i) {
            match rgba {
                Some(px) => overlay.set(device, queue, w, h, px),
                None => overlay.hide(),
            }
        }
    }

    /// Renders every player's view with `renderer` (which must be sized to [`Self::view_size`]) and composes them, with their HUDs, into `target`.
    pub fn render(
        &mut self,
        renderer: &mut LiveRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        t: f32,
        views: &[PlayerView<'_>],
        target: &wgpu::TextureView,
    ) {
        // Streaming follows everyone at once: a view's ground must not be dropped because another view was drawn after it.
        let eyes: Vec<glam::Vec3> = views.iter().map(|v| v.camera.eye).collect();
        renderer.set_stream_eyes(&eyes);
        renderer.overlay.hide();
        for (i, view) in views.iter().enumerate().take(self.layout.views.len()) {
            renderer.set_hidden_objects(view.hidden.iter().map(String::as_str));
            renderer.render_view(device, queue, scene, t, &view.camera, &self.view, view.layers);
            let rect = self.layout.views[i];
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("split-compose") });
            {
                let clear = if i == 0 { wgpu::LoadOp::Clear(GUTTER_COLOR) } else { wgpu::LoadOp::Load };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("split-compose-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: clear, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                set_rect(&mut pass, rect);
                pass.set_pipeline(&self.blit.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
                self.huds[i].draw(&mut pass);
            }
            queue.submit(Some(encoder.finish()));
        }
        if self.global.visible() {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("split-global-overlay") });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("split-global-overlay-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                self.global.draw(&mut pass);
            }
            queue.submit(Some(encoder.finish()));
        }
    }
}

fn set_rect(pass: &mut wgpu::RenderPass<'_>, r: Rect) {
    pass.set_viewport(r.x as f32, r.y as f32, r.w as f32, r.h as f32, 0.0, 1.0);
    pass.set_scissor_rect(r.x, r.y, r.w, r.h);
}
