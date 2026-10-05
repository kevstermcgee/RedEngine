//! Rendering commands: frame, render, storyboard, tour (unavailable, with a clear message, in a build without the `gfx` feature).

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_frame(
    scene: &Path,
    out: &Path,
    t: f32,
    hour: Option<f32>,
    eye: Option<&str>,
    at: Option<&str>,
    fov: Option<f32>,
    hide: Vec<String>,
    cut_above: Option<f32>,
    size: Option<&str>,
) -> Result<(), String> {
    let started = Instant::now();
    // `--hour` reads the scene's clock: the scene time at which it shows that hour.
    let t = match hour {
        Some(h) => {
            let text = std::fs::read_to_string(scene).map_err(|e| format!("{}: {e}", scene.display()))?;
            let parsed = red_engine2::schema::parse_scene(&text).map_err(|e| format!("{}: {}", scene.display(), e.join("; ")))?;
            let clock = parsed.clock.ok_or("--hour needs a scene with a `clock` block (see `describe scene`)")?;
            clock.t_for_hour(h) + t
        }
        None => t,
    };
    let size = match size {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or("--size must look like 1280x720")?;
            Some((w.parse::<u32>().map_err(|_| "bad width")?, h.parse::<u32>().map_err(|_| "bad height")?))
        }
        None => None,
    };
    let opts = FrameOpts { eye: eye.map(v3).transpose()?, at: at.map(v3).transpose()?, fov, hide, cut_above, size, t };
    if let (Some(eye), Ok(text)) = (opts.eye, std::fs::read_to_string(scene)) {
        if let Ok(parsed) = red_engine2::schema::parse_scene(&text) {
            let target = opts.at.unwrap_or_else(|| parsed.camera.target.sample(t));
            if let Some(w) = far_plane_warning(eye, target, parsed.camera.far) {
                eprintln!("{w}");
            }
        }
    }
    shots::render_frame(scene, out, &opts)?;
    println!("wrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

#[cfg(feature = "gfx")]
pub(crate) fn run_render(scene: &Path, out: &Path) -> Result<(), String> {
    let started = Instant::now();
    red_engine2::render_video(scene, out, |done, total| {
        print!("\rrendering frame {done}/{total}");
        use std::io::Write;
        std::io::stdout().flush().ok();
    })
    .map_err(|e| e.to_string())?;
    println!("\nwrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

#[cfg(feature = "gfx")]
pub(crate) fn run_storyboard(scene: &Path, out: &Path, frames: u32) -> Result<(), String> {
    let started = Instant::now();
    red_engine2::render_storyboard_png(scene, out, frames).map_err(|e| e.to_string())?;
    println!("wrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

#[cfg(not(feature = "gfx"))]
pub(crate) fn run_render(_scene: &Path, _out: &Path) -> Result<(), String> {
    Err(red_engine2::tools::NO_GFX.to_string())
}

#[cfg(not(feature = "gfx"))]
pub(crate) fn run_storyboard(_scene: &Path, _out: &Path, _frames: u32) -> Result<(), String> {
    Err(red_engine2::tools::NO_GFX.to_string())
}

pub(crate) fn run_tour(scene: &Path, out: &Path, cols: u32, only: Option<&str>, views: &[String]) -> Result<(), String> {
    let started = Instant::now();
    let custom: Option<Vec<View>> = if views.is_empty() {
        None
    } else {
        let mut v = Vec::new();
        for s in views {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() < 3 {
                return Err(format!("--view '{s}' must look like label:ex,ey,ez:tx,ty,tz[:fov]"));
            }
            v.push(View {
                label: parts[0].to_string(),
                eye: v3(parts[1])?,
                at: v3(parts[2])?,
                fov: parts.get(3).and_then(|f| f.parse().ok()).unwrap_or(90.0),
                cut_above: None,
            });
        }
        Some(v)
    };
    let labels = shots::tour(scene, out, custom, cols, only)?;
    println!("wrote {} with {} view(s): {} ({:.1}s)", out.display(), labels.len(), labels.join(", "), started.elapsed().as_secs_f32());
    Ok(())
}

/// A warning when a free camera at `eye` looks at `target` from farther away than the scene's `camera.far`: the far plane clips everything, and the picture is
/// sky only (found the hard way rendering a 300 m overview of a 900 m circuit).
fn far_plane_warning(eye: glam::Vec3, target: glam::Vec3, far: f32) -> Option<String> {
    let d = eye.distance(target);
    (d > far).then(|| format!("warning: the eye is {d:.0} m from the target but the scene's camera.far is {far:.0} m: things beyond it are not drawn, so this may render as sky only (raise camera.far in the scene)"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_sky(
    scene: &Path,
    out: &Path,
    hours: &str,
    cols: u32,
    tile: u32,
    eye: Option<&str>,
    at: Option<&str>,
    fov: Option<f32>,
    size: Option<&str>,
    look: Option<&str>,
) -> Result<(), String> {
    if let Some(l) = look.filter(|l| *l != "sun" && *l != "moon") {
        return Err(format!("--look `{l}`: the sun or the moon"));
    }
    let started = Instant::now();
    let hours: Vec<f32> = hours.split(',').map(|h| h.trim().parse::<f32>().map_err(|_| format!("--hours: `{h}` is not a number"))).collect::<Result<_, _>>()?;
    let size = match size {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or("--size must look like 960x540")?;
            Some((w.parse::<u32>().map_err(|_| "bad width")?, h.parse::<u32>().map_err(|_| "bad height")?))
        }
        None => None,
    };
    let opts = FrameOpts { eye: eye.map(v3).transpose()?, at: at.map(v3).transpose()?, fov, size, ..Default::default() };
    let titles = shots::sky_sheet(scene, out, &hours, &opts, cols, tile, look)?;
    println!("wrote {} ({} views: {}) ({:.2}s)", out.display(), titles.len(), titles.join(", "), started.elapsed().as_secs_f32());
    Ok(())
}

/// `splitshot`: the split screen a scene would show with `players` local players.
#[cfg(feature = "gfx")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_splitshot(
    scene_path: &Path,
    out: &Path,
    players: usize,
    size: &str,
    hour: Option<f32>,
    spread: f32,
    gutter: u32,
    repeat: u32,
    stats: Option<&Path>,
) -> Result<(), String> {
    use red_engine2::app::{OffscreenSplit, ViewCamera};
    use red_engine2::split_gpu::PlayerView;
    let (w, h) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))).ok_or("--size must look like 1280x720")?;
    if !(1..=4).contains(&players) || !(64..=7680).contains(&w) || !(64..=4320).contains(&h) {
        return Err("--players is 1 to 4 and --size from 64x64 to 7680x4320".into());
    }
    if stats.is_some() && repeat == 0 {
        return Err("--stats needs --repeat N (how many timed renders to take after the picture)".into());
    }
    let started = Instant::now();
    let scene = red_engine2::load_scene(scene_path).map_err(|e| e.join("\n"))?;
    let t = match (hour, scene.clock.as_ref()) {
        (Some(hr), Some(c)) => c.t_for_hour(hr),
        (Some(_), None) => return Err("--hour needs a scene with a `clock`".into()),
        _ => 0.0,
    };
    let mut split = OffscreenSplit::new(&scene, w, h, players, gutter).map_err(|e| e.to_string())?;
    let layout = split.layout().clone();
    let centre = scene.camera.position.sample(t);
    let looking = (scene.camera.target.sample(t) - centre).normalize_or(glam::Vec3::NEG_Z);
    let base_yaw = looking.x.atan2(-looking.z);
    let views: Vec<PlayerView> = (0..players)
        .map(|k| {
            let yaw = base_yaw + k as f32 * std::f32::consts::TAU / players as f32;
            let dir = glam::Vec3::new(yaw.sin(), 0.0, -yaw.cos());
            let eye = centre + dir * spread;
            let mut cam = ViewCamera::look_at(eye, eye + dir * 10.0 + glam::Vec3::Y * looking.y * 10.0);
            cam.far = scene.camera.far;
            cam.near = scene.camera.near;
            cam.fov_deg = red_engine2::splitscreen::vertical_fov(scene.camera.fov.sample(t), w as f32 / h as f32, layout.views[k].aspect());
            PlayerView { camera: cam, layers: None, hidden: &[] }
        })
        .collect();
    split.set_view_distance(red_engine2::splitscreen::view_distance(players));
    let px = split.render(&scene, t, &views, &[]).map_err(|e| e.to_string())?;
    let rendered = started.elapsed().as_secs_f32();
    let mut millis = Vec::new();
    for _ in 0..repeat {
        let t0 = Instant::now();
        split.render(&scene, t, &views, &[]).map_err(|e| e.to_string())?;
        millis.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    if let Some(path) = stats {
        millis.sort_by(|a, b| a.total_cmp(b));
        let draw = split.last_draw_stats();
        let report = serde_json::json!({
            "scene": scene_path.file_name().map(|n| n.to_string_lossy().to_string()),
            "players": players,
            "window": [w, h],
            "view": [layout.size.0, layout.size.1],
            "hour": hour,
            "adapter": split.adapter(),
            "repeat": repeat,
            "ms_best": millis[0],
            "ms_median": millis[millis.len() / 2],
            "streamed": draw.is_some(),
            "resident": draw.map(|d| d.resident),
            "draws": draw.map(|d| d.draws),
            "tris": draw.map(|d| d.tris),
            "shadow_tris": draw.map(|d| d.shadow_tris),
        });
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&report).map_err(|e| e.to_string())? + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    image::save_buffer(out, &px, w, h, image::ColorType::Rgba8).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "wrote {} ({players} views of {}x{}, {}) ({rendered:.2}s)",
        out.display(),
        layout.size.0,
        layout.size.1,
        split.last_draw_stats().map_or("no streamed world".to_string(), |d| format!("last view drew {} triangles in {} draws", d.tris, d.draws))
    );
    Ok(())
}

#[cfg(not(feature = "gfx"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_splitshot(_: &Path, _: &Path, _: usize, _: &str, _: Option<f32>, _: f32, _: u32, _: u32, _: Option<&Path>) -> Result<(), String> {
    Err("this build has no renderer (built with --no-default-features); rebuild with `cargo build --release` (feature `gfx`, on by default) to use splitshot"
        .into())
}

/// `procgen`: a top-down map of a generated world and a count of what grows on it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_procgen(
    out: &Path,
    seed: u32,
    centre: &str,
    size: f64,
    scale: f32,
    grid: bool,
    biomes: bool,
    tweaks: [Option<f32>; 4],
) -> Result<(), String> {
    use red_engine2::procgen::{Config, World};
    let c = v2(centre)?;
    if !(8.0..=4000.0).contains(&size) || !(0.25..=16.0).contains(&scale) || size * scale as f64 > 8000.0 {
        return Err("--size must be 8 to 4000 m, --scale 0.25 to 16 px/m, and size x scale at most 8000 px".into());
    }
    let mut cfg = Config { seed, ..Config::default() };
    for (slot, v) in [&mut cfg.relief, &mut cfg.trees, &mut cfg.flowers, &mut cfg.grass].into_iter().zip(tweaks) {
        if let Some(v) = v {
            *slot = v;
        }
    }
    let started = Instant::now();
    let world = World::new(cfg);
    let opts = red_engine2::tools::procgen_map::MapOpts { centre: (c.x as f64, c.y as f64), size, scale, grid, biomes };
    let (img, stats) = red_engine2::tools::procgen_map::map(&world, &opts);
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    img.save(out).map_err(|e| format!("{}: {e}", out.display()))?;
    print!("{}", red_engine2::tools::procgen_map::report(&stats));
    println!("wrote {} ({}x{}, seed {seed}) ({:.2}s)", out.display(), img.width(), img.height(), started.elapsed().as_secs_f32());
    Ok(())
}

/// `flora`: a contact sheet of plant models.
pub(crate) fn run_flora(out: &Path, species: Option<&str>, variants: u32, cols: u32, tile: u32, seed: u32) -> Result<(), String> {
    use red_engine2::procgen::flora;
    let ids: Vec<flora::SpeciesId> = match species {
        Some(list) => list
            .split(',')
            .map(|k| {
                flora::by_key(k.trim())
                    .ok_or_else(|| format!("no species `{}` (try: {})", k.trim(), flora::SPECIES.iter().map(|s| s.key).collect::<Vec<_>>().join(", ")))
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    if !(32..=1024).contains(&tile) || variants > 16 {
        return Err("--tile must be 32 to 1024 pixels and --variants at most 16".into());
    }
    let tiles = red_engine2::tools::flora_sheet::tiles(&ids, variants, seed);
    let img = red_engine2::tools::flora_sheet::sheet(&tiles, cols, tile);
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    img.save(out).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} ({} plants)", out.display(), tiles.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_camera_beyond_the_far_plane_is_warned_about() {
        let w = far_plane_warning(glam::Vec3::new(0.0, 300.0, 0.0), glam::Vec3::ZERO, 200.0).expect("farther than far");
        assert!(w.contains("camera.far") && w.contains("300 m"), "{w}");
        assert!(far_plane_warning(glam::Vec3::new(0.0, 30.0, 0.0), glam::Vec3::ZERO, 200.0).is_none());
    }
}
