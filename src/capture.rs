//! Reading a rendered frame back to the CPU, so anything that draws can be looked at without a screen grab (ADR 2026-09-28-seeing-what-the-player-sees).
//!
//! A screen capture takes whatever is on top of the desktop: another window, an unfocused game, a lock screen, a second run started in parallel. [`Capture`] is an
//! offscreen colour target the same format as the renderer's: draw the frame into [`Capture::view`] (the same `LiveRenderer::render_ex` call that draws to the
//! window), then [`Capture::read_rgba`] copies it back. It needs a device and nothing else: no window, no focus, no visible desktop.

use anyhow::{anyhow, Result};
use image::RgbaImage;
use std::path::Path;

/// An offscreen colour target with a staging buffer to read it back through.
pub struct Capture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    staging: wgpu::Buffer,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    padded_bytes_per_row: u32,
}

/// Whether the channel order of `format` is blue-green-red-alpha (swapped for the PNG); `Err` for a format that is not 8 bits per channel.
fn bgra(format: wgpu::TextureFormat) -> Result<bool> {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb => Ok(false),
        F::Bgra8Unorm | F::Bgra8UnormSrgb => Ok(true),
        other => Err(anyhow!("cannot capture a {other:?} target (only 8-bit RGBA and BGRA)")),
    }
}

impl Capture {
    /// A `width` x `height` target of `format` (the format the `LiveRenderer` was built for).
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> Result<Capture> {
        bgra(format)?;
        let (width, height) = (width.max(1), height.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture-target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let padded_bytes_per_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture-staging"),
            size: u64::from(padded_bytes_per_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Ok(Capture { view: texture.create_view(&wgpu::TextureViewDescriptor::default()), texture, staging, width, height, format, padded_bytes_per_row })
    }

    /// The view to draw the frame into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Whether this target already has that format and size (so a caller can keep one and reuse it).
    pub fn matches(&self, format: wgpu::TextureFormat, width: u32, height: u32) -> bool {
        self.format == format && self.width == width.max(1) && self.height == height.max(1)
    }

    /// Size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Copies what was drawn into [`view`](Self::view) back as an RGBA image (waits for the GPU).
    pub fn read_rgba(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<RgbaImage> {
        let swap = bgra(self.format)?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("capture-copy") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.staging,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.padded_bytes_per_row), rows_per_image: Some(self.height) },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        let slice = self.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| anyhow!("the GPU did not finish the capture: {e:?}"))?;
        rx.recv()??;
        let mapped = slice.get_mapped_range()?;
        let mut pixels = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in mapped.chunks(self.padded_bytes_per_row as usize).take(self.height as usize) {
            pixels.extend_from_slice(&row[..self.width as usize * 4]);
        }
        drop(mapped);
        self.staging.unmap();
        if swap {
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        RgbaImage::from_raw(self.width, self.height, pixels).ok_or_else(|| anyhow!("the captured frame has the wrong size"))
    }
}

/// Writes `image` as a PNG, creating the folder.
pub fn save_png(image: &RgbaImage, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    image.save(path).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// How much of `image` is the same colour as its top-left pixel, 0..1 (a frame that is all one colour is a renderer that drew nothing).
pub fn flat_fraction(image: &RgbaImage) -> f32 {
    let Some(first) = image.pixels().next() else { return 1.0 };
    image.pixels().filter(|p| p.0 == first.0).count() as f32 / (image.width() * image.height()).max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_eight_bit_rgba_and_bgra_formats_can_be_captured() {
        use wgpu::TextureFormat as F;
        assert!(!bgra(F::Rgba8UnormSrgb).unwrap());
        assert!(bgra(F::Bgra8UnormSrgb).unwrap());
        assert!(bgra(F::Rgba16Float).unwrap_err().to_string().contains("cannot capture"));
    }

    #[test]
    fn a_flat_image_is_recognised_as_flat() {
        let flat = RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255]));
        assert_eq!(flat_fraction(&flat), 1.0);
        let mut striped = flat.clone();
        for x in 0..4 {
            for y in 0..8 {
                striped.put_pixel(x, y, image::Rgba([200, 0, 0, 255]));
            }
        }
        assert!((flat_fraction(&striped) - 0.5).abs() < 1e-6);
    }

    /// Needs a GPU adapter (hardware or software); skipped where there is none (CI without lavapipe), the same way the golden-view checks are.
    #[test]
    fn a_frame_drawn_offscreen_comes_back_with_its_colours_in_the_right_order() {
        let Ok(gpu) = crate::gpu::Gpu::new() else {
            eprintln!("no GPU adapter here: capture readback not exercised");
            return;
        };
        for format in [wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Bgra8UnormSrgb] {
            let cap = Capture::new(&gpu.device, format, 37, 21).unwrap(); // a width that needs row padding
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            {
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: cap.view(),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
            gpu.queue.submit(Some(encoder.finish()));
            let img = cap.read_rgba(&gpu.device, &gpu.queue).unwrap();
            assert_eq!((img.width(), img.height()), (37, 21));
            assert_eq!(img.get_pixel(36, 20).0, [255, 0, 0, 255], "red stays red whichever channel order the target uses: {format:?}");
            assert_eq!(flat_fraction(&img), 1.0);
        }
    }
}
