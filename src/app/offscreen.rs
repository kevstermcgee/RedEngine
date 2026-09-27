//! Draw a client's view without a window: the same [`LiveRenderer`] world pass and HUD overlay a window shows, read
//! back to RGBA. For presentation checks in tests and CI screenshots (a machine with no GPU can still use a software
//! adapter; `red_engine2 doctor` says which).

use crate::app::camera::ViewCamera;
use crate::gpu::Gpu;
use crate::schema::Scene;
use crate::ui::Layout;
use crate::viewer::LiveRenderer;
use anyhow::{Context, Result};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A headless render target plus a world renderer for one scene.
pub struct Offscreen {
    gpu: Gpu,
    renderer: LiveRenderer,
    texture: wgpu::Texture,
    staging: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row: u32,
}

impl Offscreen {
    /// Builds a `width` x `height` target and uploads `scene` (see [`LiveRenderer::world`] for what may change later).
    pub fn new(scene: &Scene, width: u32, height: u32) -> Result<Self> {
        let (width, height) = (width.max(1), height.max(1));
        let gpu = Gpu::new().context("no GPU adapter for offscreen rendering")?;
        let renderer = LiveRenderer::world(&gpu.device, FORMAT, scene, width, height);
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-color"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offscreen-readback"),
            size: padded_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Offscreen { gpu, renderer, texture, staging, width, height, padded_row })
    }

    /// Target size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Renders `scene` at time `t` from `camera`, omitting `hidden` objects, with `hud` painted over it, and returns
    /// tightly packed RGBA8 (sRGB) pixels, top row first.
    pub fn render<'a>(
        &mut self,
        scene: &Scene,
        t: f32,
        camera: &ViewCamera,
        hidden: impl IntoIterator<Item = &'a str>,
        hud: Option<&Layout>,
    ) -> Result<Vec<u8>> {
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        self.renderer.set_hidden_objects(hidden);
        match hud {
            Some(layout) if !layout.widgets.is_empty() => self.renderer.overlay.set(device, queue, self.width, self.height, &layout.paint().px),
            _ => self.renderer.overlay.hide(),
        }
        let view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.render_view(device, queue, scene, t, camera, &view, None);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("offscreen-copy") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.staging,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.padded_row), rows_per_image: Some(self.height) },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        let slice = self.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let mapped = slice.get_mapped_range()?;
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in mapped.chunks(self.padded_row as usize).take(self.height as usize) {
            pixels.extend_from_slice(&row[..self.width as usize * 4]);
        }
        drop(mapped);
        self.staging.unmap();
        Ok(pixels)
    }

    /// [`Self::render`] straight to a PNG file (parent directories are created).
    pub fn save_png<'a>(
        &mut self,
        path: &std::path::Path,
        scene: &Scene,
        t: f32,
        camera: &ViewCamera,
        hidden: impl IntoIterator<Item = &'a str>,
        hud: Option<&Layout>,
    ) -> Result<()> {
        let px = self.render(scene, t, camera, hidden, hud)?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        image::save_buffer(path, &px, self.width, self.height, image::ColorType::Rgba8).with_context(|| format!("writing {}", path.display()))
    }
}
