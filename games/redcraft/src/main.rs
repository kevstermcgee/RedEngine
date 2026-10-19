//! `redcraft [SCENE] [re2's options]`: the game, in the engine's first-person client with RedCraft's game mode (every option of `re2`,
//! `--headless --script` included). `redcraft shot SCENE OUT.png --at X,Y,Z` and `redcraft map SCENE OUT.png`: the world without a window.

use clap::Parser;
use std::path::PathBuf;

/// `redcraft shot`: what a player standing somewhere sees.
#[derive(Parser)]
#[command(name = "redcraft shot", about = "Draw what a player standing at a point sees")]
struct ShotArgs {
    scene: PathBuf,
    out: PathBuf,
    /// The eye: "X,Y,Z" (with --ground, Y is ignored: the eye stands 1.62 m over the ground).
    #[arg(long, default_value = "8,80,8", allow_hyphen_values = true)]
    at: String,
    /// Stand on the ground at X,Z.
    #[arg(long)]
    ground: bool,
    /// "YAW,PITCH" in degrees (yaw 0 looks north, -Z; positive pitch looks up).
    #[arg(long, default_value = "0,-10", allow_hyphen_values = true)]
    look: String,
    /// The hour of the day (0..24).
    #[arg(long, default_value_t = 10.0)]
    hour: f32,
    /// The world's seed (default: the scene's).
    #[arg(long)]
    seed: Option<u32>,
    /// "WxH" pixels.
    #[arg(long, default_value = "1280x720")]
    size: String,
    /// Show the aimed-at block "X,Y,Z,STAGE" (its outline, and cracks 1..10).
    #[arg(long, allow_hyphen_values = true)]
    aim: Option<String>,
    /// Hang every kind of block in rows in front of the eye.
    #[arg(long)]
    gallery: bool,
}

/// `redcraft map`: the world's biomes from above.
#[derive(Parser)]
#[command(name = "redcraft map", about = "Draw the biomes and heights from above")]
struct MapArgs {
    out: PathBuf,
    #[arg(long, default_value_t = 1)]
    seed: u32,
    /// Pixels across.
    #[arg(long, default_value_t = 512)]
    size: u32,
    /// Blocks per pixel.
    #[arg(long, default_value_t = 8)]
    scale: i32,
    /// The middle "X,Z".
    #[arg(long, default_value = "0,0", allow_hyphen_values = true)]
    at: String,
}

fn numbers(text: &str, n: usize, what: &str) -> Vec<f32> {
    let v: Vec<f32> = text.split(',').filter_map(|p| p.trim().parse::<f32>().ok()).collect();
    if v.len() != n {
        eprintln!("redcraft: {what} must be {n} numbers separated by commas, not `{text}`");
        std::process::exit(2);
    }
    v
}

fn main() {
    redcraft::register();
    let first = std::env::args().nth(1);
    let result = match first.as_deref() {
        Some("shot") => {
            let a = ShotArgs::parse_from(std::env::args().skip(1));
            let at = numbers(&a.at, 3, "--at");
            let look = numbers(&a.look, 2, "--look");
            let size = numbers(&a.size.replace('x', ","), 2, "--size");
            let aim = a.aim.as_deref().map(|t| {
                let v = numbers(t, 4, "--aim");
                [v[0], v[1], v[2], v[3]]
            });
            Some(redcraft::tool::shot(&redcraft::tool::ShotArgs {
                scene: a.scene,
                out: a.out,
                at: glam::Vec3::new(at[0], at[1], at[2]),
                ground: a.ground,
                yaw_deg: look[0],
                pitch_deg: look[1],
                hour: a.hour,
                seed: a.seed,
                size: (size[0] as u32, size[1] as u32),
                aim,
                gallery: a.gallery,
            }))
        }
        Some("icons") => {
            // `redcraft icons OUT.png [SIZE]`: every item's picture on one sheet.
            let out = std::env::args().nth(2).unwrap_or_else(|| "icons.png".to_string());
            let size = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(32);
            let (w, h, px) = redcraft::icons::sheet(size);
            Some(image::save_buffer(&out, &px, w, h, image::ColorType::Rgba8).map(|_| format!("wrote {out}\n")).map_err(|e| e.to_string()))
        }
        Some("map") => {
            let a = MapArgs::parse_from(std::env::args().skip(1));
            let at = numbers(&a.at, 2, "--at");
            Some(redcraft::tool::map(&redcraft::tool::MapArgs {
                out: a.out,
                seed: a.seed,
                size: a.size,
                scale: a.scale.max(1),
                centre: (at[0] as i32, at[1] as i32),
            }))
        }
        _ => None,
    };
    match result {
        Some(Ok(text)) => print!("{text}"),
        Some(Err(e)) => {
            eprintln!("redcraft: {e}");
            std::process::exit(1);
        }
        None => red_engine2::client::run(redcraft::mode_for),
    }
}
