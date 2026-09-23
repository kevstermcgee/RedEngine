pub mod color;
pub mod easing;
pub mod gpu;
pub mod mesh;
pub mod render;
pub mod schema;
pub mod skeleton;
pub mod track;
pub mod video;
pub mod viewer;

use anyhow::{Context, Result};
use std::path::Path;

pub fn load_scene(path: &Path) -> Result<schema::Scene, Vec<String>> {
    let text = std::fs::read_to_string(path).map_err(|e| vec![format!("io: {e}")])?;
    schema::parse_scene(&text)
}

pub fn validate_scene_file(path: &Path) -> Result<(), Vec<String>> {
    load_scene(path).map(|_| ())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn load_scene_or_bail(scene_path: &Path) -> Result<schema::Scene> {
    load_scene(scene_path).map_err(|errs| anyhow::anyhow!(errs.join("\n")))
}

pub fn render_frame_png(scene_path: &Path, out_png: &Path, t: f32) -> Result<()> {
    let scene = load_scene_or_bail(scene_path)?;
    let mut renderer = render::Renderer::new(&scene)?;
    let rgb = renderer.render_frame(&scene, t);
    ensure_parent_dir(out_png)?;
    image::save_buffer(out_png, &rgb, scene.width, scene.height, image::ColorType::Rgb8)
        .context("failed to write PNG")?;
    Ok(())
}

pub fn render_video(
    scene_path: &Path,
    out_path: &Path,
    mut on_progress: impl FnMut(u32, u32),
) -> Result<()> {
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
