//! Render both teams' soldiers with their weapons (how other players see them) without opening a window.
//! Run: cargo run --example soldier_poses -- <output.png> [close|front|back]
#[cfg(feature = "gfx")]
fn main() -> anyhow::Result<()> {
    use glam::Vec3;
    use red_engine2::{
        avatar::{animate, AvatarAnim},
        characters::character_object,
        gpu::{FrameTargets, Gpu},
        net::interp::PlayerPose,
        player::Character,
        viewer::{FpsCamera, FrameOptions, LiveRenderer},
        weapons::Weapon,
    };
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "out/soldiers.png".into()));
    let view = std::env::args().nth(2).unwrap_or_else(|| "front".into());
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut scene = red_engine2::schema::parse_scene(
        r##"{
        "camera":{"position":[0,1.7,8],"target":[0,1.2,0]},
        "background":{"sky_top":"#8fb0cf","sky_bottom":"#d8d6c4"},
        "ambient":{"color":"#ffffff","intensity":0.6},
        "lights":[{"id":"sun","type":"directional","direction":[-0.4,-1,-0.6],"color":"#fff2d8","intensity":2.4}],
        "objects":[{"id":"floor","type":"box","size":[40,0.2,20],"position":[0,-0.1,0],"material":{"color":"#7a7d72"}}]
    }"##,
    )
    .map_err(|e| anyhow::anyhow!(e.join("\n")))?;
    // (team, weapon, speed, dead, throwing)
    let cases = [
        (1u8, Weapon::Rifle, 0.0, false, false),
        (1, Weapon::Pistol, 5.0, false, false),
        (1, Weapon::Sentinel, 0.0, false, false),
        (2, Weapon::Carbine, 0.0, false, false),
        (2, Weapon::Knife, 0.0, false, false),
        (2, Weapon::Frag, 0.0, false, true),
        (2, Weapon::Lancer, 0.0, false, false),
        (1, Weapon::Rifle, 0.0, true, false),
    ];
    let base = scene.objects.len();
    for (i, c) in cases.iter().enumerate() {
        scene.objects.push(character_object(if c.0 == 1 { Character::Ridgeback } else { Character::Nightfall }, &format!("avatar_{i}")));
    }
    let gpu = Gpu::new()?;
    let (w, h) = if view == "close" { (1280, 720) } else { (1600, 540) };
    let targets = FrameTargets::new(&gpu.device, w, h);
    let mut renderer = LiveRenderer::new_loadout(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, &scene, targets.width, targets.height);
    let mut hands = Vec::new();
    for (i, (team, weapon, speed, dead, throwing)) in cases.iter().enumerate() {
        let spacing = if view == "close" { 1.3 } else { 1.6 };
        let yaw = if view == "back" { std::f32::consts::PI } else { 0.25 };
        let mut pose = PlayerPose {
            extra: *team | if *throwing { 16 } else { 0 },
            pos: Vec3::new(-spacing * 3.5 + spacing * i as f32, 0.0, 0.0),
            yaw,
            pitch: 0.0,
            speed: *speed,
            character: 6 + (*team - 1),
            crouching: false,
            swinging: false,
            dead: *dead,
            weapon: weapon.wire(),
            held: u16::MAX,
            hp: 100,
            shots: 0,
            protected: false,
            kart: None,
        };
        let mut anim = AvatarAnim::default();
        let who = if *team == 1 { Character::Ridgeback } else { Character::Nightfall };
        for _ in 0..30 {
            if let Some(hand) = animate(&mut scene.objects[base + i], who, &pose, &mut anim, 0.016, 0.0) {
                if hands.len() <= i {
                    hands.push(hand);
                } else {
                    hands[i] = hand;
                }
            }
            pose.shots = pose.shots.wrapping_add(0);
        }
    }
    let mut camera = if view == "close" {
        let mut c = FpsCamera::new(Vec3::new(-1.2, 1.5, 3.2), 0.0);
        c.fov_deg = 40.0;
        c
    } else {
        let mut c = FpsCamera::new(Vec3::new(0.0, 1.5, 7.5), 0.0);
        c.fov_deg = 45.0;
        c
    };
    camera.yaw = 0.0;
    camera.pitch = -0.04;
    renderer.set_remote_hands(&hands);
    let t = 0.0;
    renderer.render_ex(
        &gpu.device,
        &gpu.queue,
        &scene,
        t,
        &camera,
        &targets.color_view,
        false,
        glam::Mat4::from_scale(Vec3::splat(0.00001)),
        glam::Mat4::from_scale(Vec3::splat(0.00001)),
        FrameOptions { crosshair: false, viewmodel: false, ..FrameOptions::default() },
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
    image::save_buffer(&out, &pixels, targets.width, targets.height, image::ColorType::Rgba8)?;
    Ok(())
}

#[cfg(not(feature = "gfx"))]
fn main() {
    eprintln!("soldier_poses needs the gfx feature");
}
