//! Rendering commands: frame, render, storyboard, tour (unavailable, with a clear message, in a build without the `gfx` feature).

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_frame(
    scene: &Path,
    out: &Path,
    t: f32,
    eye: Option<&str>,
    at: Option<&str>,
    fov: Option<f32>,
    hide: Vec<String>,
    cut_above: Option<f32>,
    size: Option<&str>,
) -> Result<(), String> {
    let started = Instant::now();
    let size = match size {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or("--size must look like 1280x720")?;
            Some((w.parse::<u32>().map_err(|_| "bad width")?, h.parse::<u32>().map_err(|_| "bad height")?))
        }
        None => None,
    };
    let opts = FrameOpts { eye: eye.map(v3).transpose()?, at: at.map(v3).transpose()?, fov, hide, cut_above, size, t };
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
