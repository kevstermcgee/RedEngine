//! wgpu plumbing: device setup, GPU uniform structs (mirrored in `shaders/*.wgsl`), pipelines, shadow map, post-fx targets.

use crate::mesh::{Mesh, Vertex};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

/// Shadow-map resolution in texels (square).
pub const SHADOW_SIZE: u32 = 2048;
pub use crate::schema::MAX_LIGHTS;
/// Offline-render supersampling factor per axis (the live viewer uses MSAA instead).
pub const SUPERSAMPLE: u32 = 2;
/// MSAA sample count for the live viewer's color/depth targets. The offline `Renderer` gets
/// its anti-aliasing for free from `SUPERSAMPLE` instead (see `FrameTargets`), so it always
/// creates pipelines with a sample count of 1 regardless of this constant.
pub const MSAA_SAMPLES: u32 = 4;

/// Per-frame uniform block (camera, lights, sun, background); mirrored by `Globals` in `shaders/*.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GlobalUniform {
    pub view_proj: [[f32; 4]; 4],
    pub light_view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub ambient: [f32; 4],
    pub light_pos_or_dir: [[f32; 4]; MAX_LIGHTS],
    pub light_color_intensity: [[f32; 4]; MAX_LIGHTS],
    pub counts: [f32; 4],
    pub bg_top: [f32; 4],
    pub bg_bottom: [f32; 4],
}

/// Per-object uniform block (transform and material); one slot per mesh.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ObjectUniform {
    pub model: [[f32; 4]; 4],
    pub normal_mat: [[f32; 4]; 4],
    pub base_color: [f32; 4],
    pub material: [f32; 4],
    pub emissive: [f32; 4],
}

/// Uniform for the clarity post pass (see `shaders/postfx.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PostUniform {
    /// near, far, projection x-scale, projection y-scale
    pub cam: [f32; 4],
    /// ao strength, outline strength, ao world radius (m), outline width (px)
    pub params: [f32; 4],
    /// target width px, target height px, outline threshold, unused
    pub params2: [f32; 4],
}

/// A mesh uploaded to the GPU (vertex + index buffers).
pub struct GpuMesh {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub index_count: u32,
    /// Local-space (pre-transform) AABB, used by the live viewer for per-frame frustum culling
    /// (see `viewer::world_aabb`) — computed once here at upload time rather than every frame.
    pub local_min: Vec3,
    pub local_max: Vec3,
}

impl GpuMesh {
    /// Uploads `mesh` into GPU buffers.
    pub fn upload(device: &wgpu::Device, mesh: &Mesh) -> Self {
        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("mesh-indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let mut local_min = Vec3::splat(f32::INFINITY);
        let mut local_max = Vec3::splat(f32::NEG_INFINITY);
        for v in &mesh.vertices {
            let p = Vec3::from_array(v.pos);
            local_min = local_min.min(p);
            local_max = local_max.max(p);
        }
        GpuMesh { vertex_buf, index_buf, index_count: mesh.indices.len() as u32, local_min, local_max }
    }
}

/// The wgpu device and queue, created headless (no window) for offline rendering and tools.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Requests an adapter and device; errors if no GPU backend is available.
    pub fn new() -> Result<Self> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self> {
        let instance = wgpu::Instance::default();
        // Prefer real hardware; on a machine with none (CI runner, container) accept a software adapter (Mesa lavapipe,
        // Windows WARP): slower, but the offline `frame`/`tour`/`verify` renders still work.
        let hardware =
            instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }).await;
        let adapter = match hardware {
            Ok(a) => a,
            Err(_) => instance
                .request_adapter(&wgpu::RequestAdapterOptions { force_fallback_adapter: true, ..Default::default() })
                .await
                .context("no compatible GPU adapter found, not even a software one (needs Vulkan, DX12 or Metal; on Linux install mesa-vulkan-drivers; `red_engine2 doctor` explains)")?,
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("forge3d-device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .context("failed to create GPU device")?;
        Ok(Gpu { device, queue })
    }
}

/// Bind-group layouts shared by every pipeline.
pub struct BindLayouts {
    pub global_uniform: wgpu::BindGroupLayout,
    pub global_full: wgpu::BindGroupLayout,
    pub object: wgpu::BindGroupLayout,
}

/// The render pipelines (shadow, background, main, ...) plus their bind layouts.
pub struct Pipelines {
    pub layouts: BindLayouts,
    pub shadow: wgpu::RenderPipeline,
    pub background: wgpu::RenderPipeline,
    pub main: wgpu::RenderPipeline,
}

/// Bindings used by the shadow and background passes: just the uniform. Kept separate from
/// [`make_global_full_layout`] because the shadow pass renders *into* the shadow map texture,
/// and wgpu forbids a texture being bound as a sampled resource in the same pass that writes to
/// it as a depth attachment — even if the shader in that pass never actually samples it.
fn make_global_uniform_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("global-uniform-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<GlobalUniform>() as u64),
            },
            count: None,
        }],
    })
}

/// Bindings used by the main pass: uniform + shadow map texture + comparison sampler.
fn make_global_full_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("global-full-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<GlobalUniform>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
        ],
    })
}

fn make_object_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("object-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ObjectUniform>() as u64),
            },
            count: None,
        }],
    })
}

/// `sample_count` applies to the `main` and `background` pipelines, which draw into whatever
/// color target the caller sets up (single-sampled for the offline `Renderer`, `MSAA_SAMPLES`
/// for the live viewer). The `shadow` pipeline always stays single-sampled — it has no color
/// attachment at all, and its depth attachment (the shadow map) is never multisampled by
/// either caller — so it ignores this parameter.
pub fn create_pipelines(device: &wgpu::Device, color_format: wgpu::TextureFormat, sample_count: u32) -> Pipelines {
    let global_uniform_bgl = make_global_uniform_layout(device);
    let global_full_bgl = make_global_full_layout(device);
    let object_bgl = make_object_layout(device);

    let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene-shader"),
        source: wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/common.wgsl"), include_str!("shaders/scene.wgsl")).into()),
    });
    let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow-shader"),
        source: wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/common.wgsl"), include_str!("shaders/shadow.wgsl")).into()),
    });
    let bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("background-shader"),
        source: wgpu::ShaderSource::Wgsl(concat!(include_str!("shaders/common.wgsl"), include_str!("shaders/background.wgsl")).into()),
    });

    let main_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("main-pipeline-layout"),
        bind_group_layouts: &[Some(&global_full_bgl), Some(&object_bgl)],
        immediate_size: 0,
    });
    let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow-pipeline-layout"),
        bind_group_layouts: &[Some(&global_uniform_bgl), Some(&object_bgl)],
        immediate_size: 0,
    });
    let bg_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bg-pipeline-layout"),
        bind_group_layouts: &[Some(&global_uniform_bgl)],
        immediate_size: 0,
    });

    let vertex_buffers = [Some(Vertex::layout())];

    let main = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("main-pipeline"),
        layout: Some(&main_layout),
        vertex: wgpu::VertexState { module: &scene_shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &vertex_buffers },
        fragment: Some(wgpu::FragmentState {
            module: &scene_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(color_format.into())],
        }),
        primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), front_face: wgpu::FrontFace::Ccw, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: sample_count, mask: !0, alpha_to_coverage_enabled: false },
        multiview_mask: None,
        cache: None,
    });

    let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow-pipeline"),
        layout: Some(&shadow_layout),
        vertex: wgpu::VertexState { module: &shadow_shader, entry_point: Some("vs_shadow"), compilation_options: Default::default(), buffers: &vertex_buffers },
        fragment: None,
        // Every primitive mesh is a closed solid, so a backface never wins the depth test
        // against its own front face — culling it in the shadow pass is free (no peter-panning
        // risk here, since it only affects self-occlusion within the same closed mesh).
        primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), front_face: wgpu::FrontFace::Ccw, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        // Always single-sampled: the shadow map is a depth-only texture that's never
        // multisampled by either caller, regardless of `sample_count`.
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("background-pipeline"),
        layout: Some(&bg_layout),
        vertex: wgpu::VertexState { module: &bg_shader, entry_point: Some("vs_bg"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &bg_shader,
            entry_point: Some("fs_bg"),
            compilation_options: Default::default(),
            targets: &[Some(color_format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        // Draws into the same (possibly multisampled) color target as `main` within one pass
        // sequence, so its sample count has to match.
        multisample: wgpu::MultisampleState { count: sample_count, mask: !0, alpha_to_coverage_enabled: false },
        multiview_mask: None,
        cache: None,
    });

    Pipelines { layouts: BindLayouts { global_uniform: global_uniform_bgl, global_full: global_full_bgl, object: object_bgl }, shadow, background, main }
}

/// Offscreen color/depth targets an offline frame renders into, at a given output size.
pub struct FrameTargets {
    pub width: u32,
    pub height: u32,
    pub color_tex: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    pub depth_view: wgpu::TextureView,
    pub shadow_view: wgpu::TextureView,
    pub staging_buffer: wgpu::Buffer,
    pub unpadded_bytes_per_row: u32,
    pub padded_bytes_per_row: u32,
}

fn align_up(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

impl FrameTargets {
    /// Creates targets for an `out_width` x `out_height` frame.
    pub fn new(device: &wgpu::Device, out_width: u32, out_height: u32) -> Self {
        let width = out_width * SUPERSAMPLE;
        let height = out_height * SUPERSAMPLE;

        let color_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color-target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth-target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            // TEXTURE_BINDING: the clarity post pass samples the world depth.
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-map"),
            size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let unpadded_bytes_per_row = width * 4;
        let padded_bytes_per_row = align_up(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("staging-buffer"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        FrameTargets {
            width,
            height,
            color_view: color_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            depth_view: depth_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            shadow_view: shadow_tex.create_view(&wgpu::TextureViewDescriptor::default()),
            color_tex,
            staging_buffer,
            unpadded_bytes_per_row,
            padded_bytes_per_row,
        }
    }
}

/// Uniform for the crosshair overlay pass.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CrosshairUniform {
    pub color: [f32; 4],
    pub to_ndc: [f32; 4],
}

/// The 2-D crosshair draw pass, the only overlay the game has.
pub struct CrosshairPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

/// A minimal always-on-top HUD pipeline (see `shaders/crosshair.wgsl`) used only by the live
/// viewer — the offline `Renderer` has no on-screen aim reticle, so this isn't part of
/// [`create_pipelines`].
pub fn create_crosshair_pipeline(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> CrosshairPipeline {
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("crosshair-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<CrosshairUniform>() as u64),
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("crosshair-pipeline-layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("crosshair-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/crosshair.wgsl").into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("crosshair-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_crosshair"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_crosshair"),
            compilation_options: Default::default(),
            targets: &[Some(color_format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    CrosshairPipeline { pipeline, bind_group_layout }
}

/// A comparison sampler for reading the shadow map.
pub fn make_shadow_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shadow-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    })
}

/// The clarity post pass: reads the world's depth buffer and multiplies contact-AO and silhouette
/// outlines into the lit color (see `shaders/postfx.wgsl`). One pipeline per (color format,
/// sample count); the depth texture it samples must have been created with `TEXTURE_BINDING`.
pub struct PostFx {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buf: wgpu::Buffer,
}

/// Builds the depth-based clarity post pass (contact AO + silhouette outline; `shaders/postfx.wgsl`) for a target with `sample_count` samples.
pub fn create_post_pipeline(device: &wgpu::Device, color_format: wgpu::TextureFormat, sample_count: u32) -> PostFx {
    let multisampled = sample_count > 1;
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("post-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<PostUniform>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { multisampled, sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2 },
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post-pipeline-layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });
    let depth_ty = if multisampled { "texture_depth_multisampled_2d" } else { "texture_depth_2d" };
    let source = include_str!("shaders/postfx.wgsl").replace("DEPTH_TEXTURE_TYPE", depth_ty);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post-shader"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    // dst_color * src_color: the pass writes a per-pixel brightness multiplier (1 = untouched).
    let multiply = wgpu::BlendState {
        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::Src, operation: wgpu::BlendOperation::Add },
        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("post-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_post"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(multiply), write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState { count: sample_count, mask: !0, alpha_to_coverage_enabled: false },
        multiview_mask: None,
        cache: None,
    });
    let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("post-uniform"),
        size: std::mem::size_of::<PostUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    PostFx { pipeline, bind_group_layout, uniform_buf }
}

impl PostFx {
    /// Binds the uniform and the world depth view; rebuild whenever the depth texture is recreated.
    pub fn bind(&self, device: &wgpu::Device, depth_view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(depth_view) },
            ],
        })
    }
}

/// Builds the post pass uniform for a target of `width` x `height` pixels seen through a
/// perspective camera (`fov_deg` vertical), from a scene's `post` settings. `edge_width_px`
/// is the outline thickness in *target* pixels (scale it with resolution/supersampling).
pub fn post_uniform(settings: &crate::schema::PostSettings, near: f32, far: f32, fov_deg: f32, width: u32, height: u32, edge_width_px: f32) -> PostUniform {
    let tan_half = (fov_deg.to_radians() * 0.5).tan().max(1e-4);
    let aspect = width as f32 / height.max(1) as f32;
    PostUniform {
        cam: [near, far, 1.0 / (aspect * tan_half), 1.0 / tan_half],
        params: [if settings.enabled { settings.ao } else { 0.0 }, if settings.enabled { settings.outline } else { 0.0 }, settings.ao_radius, edge_width_px],
        params2: [width as f32, height as f32, 0.035, 0.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMON: &str = include_str!("shaders/common.wgsl");
    const SHADERS: [(&str, &str); 3] = [
        ("scene.wgsl", include_str!("shaders/scene.wgsl")),
        ("shadow.wgsl", include_str!("shaders/shadow.wgsl")),
        ("background.wgsl", include_str!("shaders/background.wgsl")),
    ];

    /// `(size, alignment)` of a WGSL type as used in uniform structs (no `vec3`: pad to `vec4` instead).
    fn type_layout(ty: &str) -> (usize, usize) {
        let ty = ty.trim();
        match ty {
            "f32" | "u32" | "i32" => (4, 4),
            "vec2<f32>" => (8, 8),
            "vec4<f32>" => (16, 16),
            "mat4x4<f32>" => (64, 16),
            _ => {
                let inner =
                    ty.strip_prefix("array<").and_then(|s| s.strip_suffix('>')).unwrap_or_else(|| panic!("type `{ty}` is not supported by the layout test"));
                let (elem, n) = inner.rsplit_once(',').unwrap_or_else(|| panic!("bad array type `{ty}`"));
                let (size, align) = type_layout(elem);
                let stride = size.div_ceil(align) * align;
                (stride * n.trim().parse::<usize>().unwrap_or_else(|_| panic!("array length in `{ty}` must be a number")), align)
            }
        }
    }

    /// The field names and total size of `struct <name> { ... }` in `src`, laid out like a WGSL uniform.
    fn wgsl_struct(src: &str, name: &str) -> (Vec<String>, usize) {
        let start = src.find(&format!("struct {name} {{")).unwrap_or_else(|| panic!("struct {name} not found"));
        let body = &src[start..];
        let body = &body[body.find('{').unwrap_or(0) + 1..body.find("};").unwrap_or(body.len())];
        let (mut offset, mut max_align, mut names) = (0usize, 1usize, Vec::new());
        for line in body.lines().map(|l| l.split("//").next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
            let (field, ty) = line.trim_end_matches(',').split_once(':').unwrap_or_else(|| panic!("cannot parse field `{line}`"));
            let (size, align) = type_layout(ty);
            offset = offset.div_ceil(align) * align + size;
            max_align = max_align.max(align);
            names.push(field.trim().to_string());
        }
        (names, offset.div_ceil(max_align) * max_align)
    }

    #[test]
    fn the_rust_uniform_structs_match_their_wgsl_definitions() {
        let (names, size) = wgsl_struct(COMMON, "Globals");
        assert_eq!(size, std::mem::size_of::<GlobalUniform>(), "Globals: wgsl {names:?} is {size} bytes");
        let (_, size) = wgsl_struct(COMMON, "ObjectUniform");
        assert_eq!(size, std::mem::size_of::<ObjectUniform>(), "ObjectUniform");
        let (_, size) = wgsl_struct(include_str!("shaders/postfx.wgsl"), "Post");
        assert_eq!(size, std::mem::size_of::<PostUniform>(), "Post");
        let (_, size) = wgsl_struct(include_str!("shaders/crosshair.wgsl"), "Crosshair");
        assert_eq!(size, std::mem::size_of::<CrosshairUniform>(), "Crosshair");
    }

    #[test]
    fn globals_and_the_light_count_exist_once() {
        assert!(COMMON.contains(&format!("const MAX_LIGHTS: u32 = {}u;", MAX_LIGHTS)), "common.wgsl MAX_LIGHTS must equal schema::MAX_LIGHTS ({MAX_LIGHTS})");
        assert!(COMMON.contains(&format!("array<vec4<f32>, {MAX_LIGHTS}>")), "the light arrays must be MAX_LIGHTS long");
        for (file, src) in SHADERS {
            assert!(
                !src.contains("struct Globals"),
                "{file} defines its own `struct Globals`: it lives only in common.wgsl (a stale copy renders the sky black)"
            );
            assert!(!src.contains("struct ObjectUniform"), "{file} defines its own `struct ObjectUniform`: it lives only in common.wgsl");
        }
    }
}
