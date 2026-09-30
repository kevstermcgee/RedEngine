//! Render every weapon of the loadout arsenal, in hand, for both teams, into two contact sheets (no window).
//! Run: cargo run --example arsenal_poses -- <output-directory> [aim]
#[cfg(feature = "gfx")]
fn main() -> anyhow::Result<()> {
    use glam::{Mat4, Vec3};
    use red_engine2::{
        firearms,
        gpu::{FrameTargets, Gpu},
        tools::sheet::{contact_sheet, Tile},
        viewer::{viewmodel_transform, FpsCamera, FrameOptions, LiveRenderer},
        weapons::Weapon,
    };
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "out/arsenal".into()));
    let aimed = std::env::args().nth(2).as_deref() == Some("aim");
    std::fs::create_dir_all(&out)?;
    let scene = red_engine2::schema::parse_scene(
        r##"{
        "camera":{"position":[0,1.7,0],"target":[0,1.7,-12]},
        "background":{"sky_top":"#6e8294","sky_bottom":"#b8c2c5"},
        "ambient":{"color":"#ffffff","intensity":0.8},
        "lights":[{"id":"sun","type":"directional","direction":[-0.5,-1,-0.5],"color":"#ffffff","intensity":2}],
        "objects":[{"id":"floor","type":"plane","size":[40,40],"material":{"color":"#6b6f66"}},{"id":"target","type":"box","size":[0.12,0.12,0.12],"position":[0,1.7,-12],"material":{"color":"#ff5030"}}]
    }"##,
    )
    .map_err(|e| anyhow::anyhow!(e.join("\n")))?;
    let gpu = Gpu::new()?;
    let targets = FrameTargets::new(&gpu.device, 480, 270);
    let mut renderer = LiveRenderer::new_loadout(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, &scene, targets.width, targets.height);
    for skin in [1u8, 2u8] {
        let mut tiles = Vec::new();
        for weapon in Weapon::ROSTER {
            let mut camera = FpsCamera::new(Vec3::new(0.0, 1.7, 0.0), 0.0);
            let ads = if aimed && weapon.is_gun() { 1.0 } else { 0.0 };
            if ads > 0.0 {
                camera.fov_deg = (2.0 * (1.0 / weapon.aim_magnification()).atan()).to_degrees();
            }
            let (offset, rotation) =
                if weapon == Weapon::Bat { (Vec3::new(0.1, -0.125, 0.3), Mat4::IDENTITY) } else { firearms::held_pose(weapon, ads, 0.0, 0.0) };
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
                FrameOptions { weapon, crosshair: false, viewmodel: true, pickup: false, muzzle_flash: 0.0, skin, ..FrameOptions::default() },
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
            let image = image::RgbaImage::from_raw(targets.width, targets.height, pixels).ok_or_else(|| anyhow::anyhow!("bad frame"))?;
            tiles.push(Tile { image, title: weapon.name().to_uppercase() });
        }
        contact_sheet(&tiles, 4, 360).save(out.join(format!("team{skin}{}.png", if aimed { "-aim" } else { "" })))?;
    }
    Ok(())
}

#[cfg(not(feature = "gfx"))]
fn main() {
    eprintln!("arsenal_poses needs the gfx feature");
}
