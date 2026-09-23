use crate::mesh::{Mesh, Vertex};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const SHADOW_SIZE: u32 = 2048;
pub const MAX_LIGHTS: usize = 4;
pub const SUPERSAMPLE: u32 = 2;

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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ObjectUniform {
    pub model: [[f32; 4]; 4],
    pub normal_mat: [[f32; 4]; 4],
    pub base_color: [f32; 4],
    pub material: [f32; 4],
    pub emissive: [f32; 4],
}

pub struct GpuMesh {
    pub vertex_buf: wgpu::Buffer,
    pub index_buf: wgpu::Buffer,
    pub index_count: u32,
}

impl GpuMesh {
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
        GpuMesh { vertex_buf, index_buf, index_count: mesh.indices.len() as u32 }
    }
}

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .context("no compatible GPU adapter found (forge3d needs Vulkan, DX12, or Metal)")?;
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

pub struct BindLayouts {
    pub global_uniform: wgpu::BindGroupLayout,
    pub global_full: wgpu::BindGroupLayout,
    pub object: wgpu::BindGroupLayout,
}

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

pub fn create_pipelines(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Pipelines {
    let global_uniform_bgl = make_global_uniform_layout(device);
    let global_full_bgl = make_global_full_layout(device);
    let object_bgl = make_object_layout(device);

    let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene.wgsl").into()),
    });
    let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
    });
    let bg_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("background-shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/background.wgsl").into()),
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
        vertex: wgpu::VertexState {
            module: &scene_shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &vertex_buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module: &scene_shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(color_format.into())],
        }),
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow-pipeline"),
        layout: Some(&shadow_layout),
        vertex: wgpu::VertexState {
            module: &shadow_shader,
            entry_point: Some("vs_shadow"),
            compilation_options: Default::default(),
            buffers: &vertex_buffers,
        },
        fragment: None,
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let background = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("background-pipeline"),
        layout: Some(&bg_layout),
        vertex: wgpu::VertexState {
            module: &bg_shader,
            entry_point: Some("vs_bg"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &bg_shader,
            entry_point: Some("fs_bg"),
            compilation_options: Default::default(),
            targets: &[Some(color_format.into())],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    Pipelines {
        layouts: BindLayouts { global_uniform: global_uniform_bgl, global_full: global_full_bgl, object: object_bgl },
        shadow,
        background,
        main,
    }
}

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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CrosshairUniform {
    pub color: [f32; 4],
    pub to_ndc: [f32; 4],
}

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
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_crosshair"),
            compilation_options: Default::default(),
            buffers: &[],
        },
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
