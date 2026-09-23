use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "red_engine2", about = "Render a JSON scene to a 3D MP4, or check/preview it first.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check a scene file for errors before spending any render time.
    Validate { scene: PathBuf },
    /// Render a single PNG frame at time `--t` seconds (fast layout/lighting check).
    Frame {
        scene: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 0.0)]
        t: f32,
    },
    /// Render the full scene to an MP4.
    Render { scene: PathBuf, out: PathBuf },
    /// Render a multi-frame contact sheet PNG for fast whole-clip review.
    Storyboard {
        scene: PathBuf,
        out: PathBuf,
        #[arg(long, default_value_t = 6)]
        frames: u32,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Validate { scene } => run_validate(&scene),
        Command::Frame { scene, out, t } => run_frame(&scene, &out, t),
        Command::Render { scene, out } => run_render(&scene, &out),
        Command::Storyboard { scene, out, frames } => run_storyboard(&scene, &out, frames),
    };
    if let Err(msg) = result {
        eprintln!("{msg}");
        std::process::exit(1);
    }
}

fn run_validate(scene: &Path) -> Result<(), String> {
    match red_engine2::validate_scene_file(scene) {
        Ok(()) => {
            println!("OK: scene is valid");
            Ok(())
        }
        Err(errs) => {
            for e in &errs {
                eprintln!("error: {e}");
            }
            Err(format!("{} error(s)", errs.len()))
        }
    }
}

fn run_frame(scene: &Path, out: &Path, t: f32) -> Result<(), String> {
    let started = Instant::now();
    red_engine2::render_frame_png(scene, out, t).map_err(|e| e.to_string())?;
    println!("wrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}

fn run_render(scene: &Path, out: &Path) -> Result<(), String> {
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

fn run_storyboard(scene: &Path, out: &Path, frames: u32) -> Result<(), String> {
    let started = Instant::now();
    red_engine2::render_storyboard_png(scene, out, frames).map_err(|e| e.to_string())?;
    println!("wrote {} ({:.2}s)", out.display(), started.elapsed().as_secs_f32());
    Ok(())
}
