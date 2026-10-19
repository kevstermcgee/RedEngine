//! `redcraft shot` and `redcraft map`: the world without a window.
//!
//! * `shot SCENE OUT.png --at X,Y,Z [--look YAW,PITCH] [--hour H] [--seed N]` draws what a player standing there sees (degrees; yaw 0 looks north,
//!   -Z), with the world streamed around them first. `--ground` puts the eye 1.62 m over the highest block at X,Z.
//! * `map SCENE OUT.png [--seed N] [--size BLOCKS] [--at X,Z]` draws the biomes and heights from above, one pixel per `--scale` blocks, and prints
//!   what share of the land each biome has.

use crate::gen::{Biome, Generator, SEA};
use glam::Vec3;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

/// What `shot` draws.
pub struct ShotArgs {
    pub scene: PathBuf,
    pub out: PathBuf,
    pub at: Vec3,
    pub ground: bool,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub hour: f32,
    pub seed: Option<u32>,
    pub size: (u32, u32),
    /// The aimed block and mining stage to show (`--aim X,Y,Z,STAGE`).
    pub aim: Option<[f32; 4]>,
    /// Hang every kind of block in rows in front of the eye.
    pub gallery: bool,
}

/// Every kind of block (one of each facing kind), in rows of 12 on a stone shelf 6 m in front of `eye`, facing it.
fn build_gallery(t: &mut crate::stream::Terrain, eye: Vec3, yaw: f32) {
    use crate::block::Block;
    t.settle(eye, 30.0);
    let kinds: Vec<Block> = crate::item::Item::all().into_iter().filter_map(|i| i.block()).collect();
    let fwd = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let base = eye + fwd * 7.0 - right * 6.0 + Vec3::Y * 1.0;
    for (i, b) in kinds.iter().enumerate() {
        let (col, row) = ((i % 12) as f32, (i / 12) as f32);
        let p = base + right * col - Vec3::Y * row * 2.0;
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let b = match b.facing() {
            // Face the eye: the facing whose front points back along the view.
            Some(_) => {
                let f = ((yaw.to_degrees() + 180.0 + 45.0).rem_euclid(360.0) / 90.0) as u8 % 4;
                Block::faced(*b, f)
            }
            None => *b,
        };
        t.set_block(x, y - 1, z, Block::STONE);
        t.set_block(x, y, z, b);
    }
    t.settle(eye, 30.0);
}

/// The direction a yaw and pitch look along (yaw 0 looks along -Z, positive turns toward +X).
pub fn look_dir(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos())
}

/// How much daylight there is at an hour (0 at night .. 1 at noon), as the block shader reads it.
pub fn daylight(hour: f32) -> f32 {
    let d = ((hour - 6.0) / 12.0 * std::f32::consts::PI).sin();
    (d * 1.6).clamp(0.0, 1.0)
}

pub fn shot(a: &ShotArgs) -> Result<String, String> {
    use red_engine2::track::Track;
    let mut scene = red_engine2::load_scene(&a.scene).map_err(|e| format!("{}: {}", a.scene.display(), e.join("; ")))?;
    let spec = crate::spec::spec_of(&scene).ok_or_else(|| format!("{}: has no `redcraft` block", a.scene.display()))?;
    if let Some(seed) = a.seed {
        let fixed = crate::spec::CraftSpec { seed: red_engine2::atmosphere::LaunchSeed::Fixed(seed), ..(*spec).clone() };
        scene.blocks.set(crate::spec::BLOCK.key, Some(std::sync::Arc::new(fixed) as red_engine2::schema::BlockValue));
    }
    let seed = a.seed.unwrap_or_else(|| spec.seed.value(None));
    let mut eye = a.at;
    if a.ground {
        // The ground under the eye: generate the column and stand on it.
        let g = Generator::new(seed);
        let (x, z) = (a.at.x.floor() as i32, a.at.z.floor() as i32);
        let c = g.generate(crate::chunk::ChunkPos::of(x, z));
        let top = (0..crate::chunk::H).rev().find(|&y| c.get(x.rem_euclid(16), y, z.rem_euclid(16)).solid()).unwrap_or(SEA);
        eye.y = top as f32 + 1.0 + 1.62;
    }
    let dir = look_dir(a.yaw_deg.to_radians(), a.pitch_deg.to_radians());
    scene.width = a.size.0;
    scene.height = a.size.1;
    scene.camera.position = Track::constant(eye);
    scene.camera.target = Track::constant(eye + dir * 10.0);
    let view = std::env::var("REDCRAFT_VIEW").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
    if let Some(c) = scene.clock.as_mut() {
        let (start, density) = crate::mode::haze_for(view);
        c.fog_start = start;
        c.fog = density;
    }
    let t = scene.clock.as_ref().map_or(0.0, |c| c.t_for_hour(a.hour));
    let started = std::time::Instant::now();
    let mut renderer = red_engine2::render::Renderer::new(&scene).map_err(|e| e.to_string())?;
    if let Some(still) = renderer.layer_mut::<crate::gpu::StillCraft>() {
        still.game_params[2][0] = daylight(a.hour);
        if let Some(aim) = a.aim {
            still.game_params[0] = aim;
            still.game_params[1][0] = 1.0;
        }
    }
    if a.gallery {
        if let Some(still) = renderer.layer_mut::<crate::gpu::StillCraft>() {
            build_gallery(&mut still.terrain, eye, a.yaw_deg.to_radians());
        }
    }
    let rgb = renderer.render_frame(&scene, t);
    image::save_buffer(&a.out, &rgb, scene.width, scene.height, image::ColorType::Rgb8).map_err(|e| e.to_string())?;
    let mut out = String::new();
    let (tris, sections) = renderer.layer_mut::<crate::gpu::StillCraft>().map_or((0, 0), |s| s.layer.drawn.get());
    let _ = writeln!(
        out,
        "wrote {} ({:.1} s): eye {:.1},{:.1},{:.1}, {} sections, {} triangles drawn",
        a.out.display(),
        started.elapsed().as_secs_f32(),
        eye.x,
        eye.y,
        eye.z,
        sections,
        tris
    );
    Ok(out)
}

/// What `map` draws.
pub struct MapArgs {
    pub out: PathBuf,
    pub seed: u32,
    /// Pixels across.
    pub size: u32,
    /// Blocks per pixel.
    pub scale: i32,
    pub centre: (i32, i32),
}

pub fn map(a: &MapArgs) -> Result<String, String> {
    let g = Generator::new(a.seed);
    let n = a.size as i32;
    let mut px = vec![0u8; (a.size * a.size * 3) as usize];
    let mut share: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut highest = 0;
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (a.centre.0 + (i - n / 2) * a.scale, a.centre.1 + (j - n / 2) * a.scale);
            let c = g.column(x, z);
            *share.entry(c.biome.name()).or_default() += 1;
            highest = highest.max(c.height);
            let base = c.biome.map_color();
            let shade = if matches!(c.biome, Biome::Ocean | Biome::FrozenOcean | Biome::River) {
                0.75 + 0.25 * ((c.height - (SEA - 30)) as f32 / 30.0).clamp(0.0, 1.0)
            } else {
                0.7 + 0.5 * ((c.height - SEA) as f32 / 60.0).clamp(0.0, 1.0)
            };
            let k = ((j * n + i) * 3) as usize;
            for ch in 0..3 {
                px[k + ch] = (base[ch] as f32 * shade).min(255.0) as u8;
            }
        }
    }
    // The spawn, as a red cross.
    let (sx, _, sz) = g.spawn_point();
    let (pi, pj) = ((sx - a.centre.0) / a.scale + n / 2, (sz - a.centre.1) / a.scale + n / 2);
    for d in -4..=4 {
        for (i, j) in [(pi + d, pj), (pi, pj + d)] {
            if (0..n).contains(&i) && (0..n).contains(&j) {
                let k = ((j * n + i) * 3) as usize;
                px[k..k + 3].copy_from_slice(&[230, 30, 30]);
            }
        }
    }
    image::save_buffer(&a.out, &px, a.size, a.size, image::ColorType::Rgb8).map_err(|e| e.to_string())?;
    let total: u32 = share.values().sum();
    let mut out =
        format!("wrote {} ({} x {} blocks), seed {}, spawn {},{}, highest ground y {}\n", a.out.display(), n * a.scale, n * a.scale, a.seed, sx, sz, highest);
    let mut v: Vec<(&&str, &u32)> = share.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in v {
        let _ = writeln!(out, "  {:<13} {:>5.1}%", name, *count as f32 * 100.0 / total as f32);
    }
    Ok(out)
}
