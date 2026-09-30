//! Render a line-up of other players' avatars (how `NetSession::update_scene` draws them) without opening a window: standing with a rifle,
//! firing an SMG and a shotgun (muzzle flash), mid-swing with the bat, walking with a pistol, and one who has just died.
//! Run: cargo run --example avatar_poses -- <output.png>
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
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "out/avatar-poses.png".into()));
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut scene = red_engine2::schema::parse_scene(
        r##"{
        "camera":{"position":[0,1.7,8],"target":[0,1.2,0]},
        "background":{"sky_top":"#1a2340","sky_bottom":"#6a4a7a"},
        "ambient":{"color":"#aab4ff","intensity":0.7},
        "lights":[{"id":"sun","type":"directional","direction":[-0.4,-1,-0.6],"color":"#ffffff","intensity":2.2}],
        "objects":[{"id":"floor","type":"box","size":[40,0.2,20],"position":[0,-0.1,0],"material":{"color":"#3a3f5a"}}]
    }"##,
    )
    .map_err(|e| anyhow::anyhow!(e.join("\n")))?;

    // (name, weapon, speed, swinging, dead, shots-after-baseline, seconds to run)
    struct Case {
        weapon: Weapon,
        speed: f32,
        swinging: bool,
        dead: bool,
        fire: bool,
        frames: usize,
    }
    let cases = [
        Case { weapon: Weapon::Rifle, speed: 0.0, swinging: false, dead: false, fire: false, frames: 5 },
        Case { weapon: Weapon::Smg, speed: 0.0, swinging: false, dead: false, fire: true, frames: 1 },
        Case { weapon: Weapon::Shotgun, speed: 0.0, swinging: false, dead: false, fire: true, frames: 1 },
        Case { weapon: Weapon::Bat, speed: 0.0, swinging: true, dead: false, fire: false, frames: 6 },
        Case { weapon: Weapon::Pistol, speed: 6.0, swinging: false, dead: false, fire: false, frames: 20 },
        Case { weapon: Weapon::Rifle, speed: 0.0, swinging: false, dead: true, fire: false, frames: 60 },
    ];
    let looks = [Character::Human, Character::Cowboy, Character::Wizard, Character::Alien, Character::Robot, Character::Human];
    let base = scene.objects.len();
    for (i, who) in looks.iter().enumerate() {
        scene.objects.push(character_object(*who, &format!("avatar_{i}")));
    }
    let gpu = Gpu::new()?;
    let targets = FrameTargets::new(&gpu.device, 1280, 540);
    let mut renderer = LiveRenderer::new(&gpu.device, wgpu::TextureFormat::Rgba8UnormSrgb, &scene, targets.width, targets.height);
    let mut hands = Vec::new();
    for (i, (case, who)) in cases.iter().zip(looks).enumerate() {
        let mut pose = PlayerPose {
            pos: Vec3::new(-4.25 + 1.7 * i as f32, 0.0, 0.0),
            yaw: 1.9,
            pitch: 0.0,
            speed: case.speed,
            character: 0,
            crouching: false,
            swinging: false,
            dead: case.dead,
            weapon: case.weapon.wire(),
            held: red_engine2::net::protocol::NO_PROP,
            hp: 100,
            shots: 3,
            protected: false,
            kart: None,
        };
        let mut anim = AvatarAnim::default();
        animate(&mut scene.objects[base + i], who, &pose, &mut anim, 0.016, 0.0); // a baseline for the shot counter
        pose.swinging = case.swinging;
        if case.fire {
            pose.shots = 4;
        }
        let mut hand = None;
        for _ in 0..case.frames {
            hand = animate(&mut scene.objects[base + i], who, &pose, &mut anim, 0.016, 0.0);
        }
        hands.extend(hand);
    }
    renderer.set_remote_hands(&hands);
    let mut camera = FpsCamera::new(Vec3::new(0.0, 1.45, 6.0), 0.0);
    camera.fov_deg = 45.0;
    renderer.render_ex(
        &gpu.device,
        &gpu.queue,
        &scene,
        0.0,
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
    println!("wrote {}", out.display());
    Ok(())
}

#[cfg(not(feature = "gfx"))]
fn main() {
    eprintln!("avatar_poses needs the gfx feature");
}
