//! Library root: module list plus the scene-loading and offline render entry points (`render_frame_png`, `render_video`).

#[cfg(feature = "gfx")]
pub mod audio;
pub mod characters;
pub mod collide;
pub mod color;
pub mod crypto;
pub mod easing;
#[cfg(feature = "gfx")]
pub mod firearms;
pub mod geometry;
#[cfg(feature = "gfx")]
pub mod gpu;
pub mod hit;
pub mod macros;
#[cfg(feature = "gfx")]
pub mod menu;
#[cfg(feature = "gfx")]
pub mod mesh;
pub mod net;
#[cfg(feature = "gfx")]
pub mod overlay;
pub mod physics;
pub mod player;
pub mod prefabs;
#[cfg(feature = "gfx")]
pub mod probe;
pub mod props;
#[cfg(feature = "gfx")]
pub mod render;
#[cfg(feature = "gfx")]
pub mod revolver;
pub mod schema;
pub mod sim;
pub mod skeleton;
pub mod strict;
pub mod tools;
pub mod track;
pub mod ui;
#[cfg(feature = "gfx")]
pub mod video;
#[cfg(feature = "gfx")]
pub mod viewer;
pub mod weapons;

#[cfg(feature = "gfx")]
use anyhow::{Context, Result};
use std::path::Path;

/// Reads and fully validates a scene file (macros and prefabs expanded); `Err` is a list of `object.field: message` strings.
pub fn load_scene(path: &Path) -> Result<schema::Scene, Vec<String>> {
    let text = std::fs::read_to_string(path).map_err(|e| vec![format!("io: {e}")])?;
    schema::parse_scene(&text)
}

/// Validates a scene file without rendering (`red_engine2 validate`).
pub fn validate_scene_file(path: &Path) -> Result<(), Vec<String>> {
    load_scene(path).map(|_| ())
}

#[cfg(feature = "gfx")]
fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

#[cfg(feature = "gfx")]
fn load_scene_or_bail(scene_path: &Path) -> Result<schema::Scene> {
    load_scene(scene_path).map_err(|errs| anyhow::anyhow!(errs.join("\n")))
}

/// Renders one frame of a scene at time `t` seconds to a PNG.
#[cfg(feature = "gfx")]
pub fn render_frame_png(scene_path: &Path, out_png: &Path, t: f32) -> Result<()> {
    let scene = load_scene_or_bail(scene_path)?;
    let mut renderer = render::Renderer::new(&scene)?;
    let rgb = renderer.render_frame(&scene, t);
    ensure_parent_dir(out_png)?;
    image::save_buffer(out_png, &rgb, scene.width, scene.height, image::ColorType::Rgb8).context("failed to write PNG")?;
    Ok(())
}

/// Renders the whole scene to an MP4 via ffmpeg, reporting `(frame, total)` progress.
#[cfg(feature = "gfx")]
pub fn render_video(scene_path: &Path, out_path: &Path, mut on_progress: impl FnMut(u32, u32)) -> Result<()> {
    let scene = load_scene_or_bail(scene_path)?;
    let mut renderer = render::Renderer::new(&scene)?;
    let num_frames = ((scene.duration * scene.fps as f32).round() as u32).max(1);
    let mut encoder = video::VideoEncoder::start(out_path, scene.width, scene.height, scene.fps)?;
    for i in 0..num_frames {
        let t = i as f32 / scene.fps as f32;
        let rgb = renderer.render_frame(&scene, t);
        encoder.write_frame(&rgb)?;
        on_progress(i + 1, num_frames);
    }
    encoder.finish()?;
    Ok(())
}

/// Renders `num_frames` evenly spaced frames into one contact-sheet PNG.
#[cfg(feature = "gfx")]
pub fn render_storyboard_png(scene_path: &Path, out_png: &Path, num_frames: u32) -> Result<()> {
    let scene = load_scene_or_bail(scene_path)?;
    let mut renderer = render::Renderer::new(&scene)?;
    let num_frames = num_frames.max(1);
    let cols = (num_frames as f32).sqrt().ceil() as u32;
    let rows = num_frames.div_ceil(cols);
    let (cell_w, cell_h) = (scene.width, scene.height);

    let mut canvas = image::RgbImage::new(cell_w * cols, cell_h * rows);
    for i in 0..num_frames {
        let t = if num_frames <= 1 { 0.0 } else { i as f32 / (num_frames - 1) as f32 * scene.duration };
        let rgb = renderer.render_frame(&scene, t);
        let (col, row) = (i % cols, i / cols);
        for y in 0..cell_h {
            for x in 0..cell_w {
                let src = ((y * cell_w + x) * 3) as usize;
                canvas.put_pixel(col * cell_w + x, row * cell_h + y, image::Rgb([rgb[src], rgb[src + 1], rgb[src + 2]]));
            }
        }
    }
    ensure_parent_dir(out_png)?;
    canvas.save(out_png).context("failed to save storyboard")?;
    Ok(())
}
