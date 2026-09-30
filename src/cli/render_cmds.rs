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
