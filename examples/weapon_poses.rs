//! Render every firearm through the real live renderer without opening a window.
//! Run: cargo run --example weapon_poses -- <output-directory>
#[cfg(feature = "gfx")]
fn main() -> anyhow::Result<()> {
    use glam::{Mat4, Vec3};
    use red_engine2::{
        firearms,
        gpu::{FrameTargets, Gpu},
        viewer::{viewmodel_transform, FpsCamera, FrameOptions, LiveRenderer},
        weapons::Weapon,
    };
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "out/weapon-poses".into()));
    std::fs::create_dir_all(&out)?;
    let scene = red_engine2::schema::parse_scene(
        r##"{
        "camera":{"position":[0,1.7,0],"target":[0,1.7,-12]},
        "background":{"sky_top":"#6e8294","sky_bottom":"#b8c2c5"},
        "ambient":{"color":"#ffffff","intensity":0.8},
        "lights":[{"id":"sun","type":"directional","direction":[-0.5,-1,-0.5],"color":"#ffffff","intensity":2}],
        "objects":[{"id":"target","type":"box","size":[0.12,0.12,0.12],"position":[0,1.7,-12],"material":{"color":"#ff5030"}}]
    }"##,
    )
    .map_err(|e| anyhow::anyhow!(e.join("\n")))?;
    let gpu = Gpu::new()?;
    let targets = FrameTargets::new(&gpu.device, 480, 270);
    let mut renderer = LiveRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, &scene, targets.width, targets.height);
    for (index, weapon) in Weapon::FIREARMS.into_iter().enumerate() {
        for (label, ads, kick) in [("hip", 0.0, 0.0), ("aim", 1.0, 0.0), ("recoil", 1.0, 0.6)] {
            let mut camera = FpsCamera::new(Vec3::new(0.0, 1.7, 0.0), 0.0);
            if ads > 0.0 {
                camera.fov_deg = (2.0 * (1.0 / weapon.aim_magnification()).atan()).to_degrees();
            }
            let (offset, rotation) = firearms::held_pose(weapon, ads, kick, 0.0);
            renderer.render_ex(
                &gpu.device,
                &gpu.queue,
                &scene,
                0.0,
                &camera,
                &targets.color_view,
                false,
                viewmodel_transform(&camera, offset, rotation),
                Mat4::from_scale(Vec3::splat(0.00001)),
                FrameOptions { weapon, crosshair: true, viewmodel: true, pickup: false, muzzle_flash: 0.0, ..FrameOptions::default() },
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &targets.color_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer: &targets.staging_buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(targets.padded_bytes_per_row), rows_per_image: Some(targets.height) },
                },
                wgpu::Extent3d { width: targets.width, height: targets.height, depth_or_array_layers: 1 },
            );
            gpu.queue.submit(Some(encoder.finish()));
            let slice = targets.staging_buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                tx.send(r).ok();
            });
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            rx.recv()??;
            let rgba = slice.get_mapped_range()?;
            let mut pixels = Vec::new();
            for row in rgba.chunks(targets.padded_bytes_per_row as usize) {
                pixels.extend_from_slice(&row[..targets.width as usize * 4]);
            }
            drop(rgba);
            targets.staging_buffer.unmap();
            image::save_buffer(out.join(format!("{index:02}-{label}.png")), &pixels, targets.width, targets.height, image::ColorType::Rgba8)?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "gfx"))]
fn main() {
    eprintln!("weapon_poses needs the gfx feature");
}
