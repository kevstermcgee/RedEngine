//! `frameless [SCENE] [re2's options]`: the museum, in the engine's first-person client with Frameless's game mode (every option of `re2`,
//! `--headless --script` included). `frameless museum SCENE [--shot ...]`: the museum's rooms and art without a window (the `museum` tool).
//! `RE2_MUSEUM_FRESH=1` starts a new visit instead of continuing the saved one; `RE2_SEED=N` repeats a museum (its `"seed": "launch"`).

use clap::Parser;
use std::path::PathBuf;

/// `frameless museum`: a museum's rooms, art.
#[derive(Parser)]
#[command(name = "frameless museum", about = "A museum's rooms, art")]
struct MuseumArgs {
    scene: PathBuf,
    /// The museum's seed (default: the scene's, or 0 for "launch").
    #[arg(long)]
    seed: Option<u32>,
    /// Which room "x,z" (the first room is 0,0).
    #[arg(long, default_value = "0,0", allow_hyphen_values = true)]
    room: String,
    /// Draw a picture: the room from its doorway, a piece (`--look`), or the world inside a painting (`--inside`).
    #[arg(long)]
    shot: Option<PathBuf>,
    /// The piece to stand in front of for `--shot`.
    #[arg(long)]
    look: Option<u8>,
    /// The painting world to stand in: "N" (piece N's) or "N/I/J" (painting I inside it, then J inside that).
    #[arg(long)]
    inside: Option<String>,
    /// Draw the pause menu's plan of the museum.
    #[arg(long)]
    map: Option<PathBuf>,
    /// For `--map`: count every room within this many rooms as discovered.
    #[arg(long, default_value_t = 2)]
    explore: u32,
    /// Scene time for `--shot` (paintings move).
    #[arg(long, default_value_t = 4.0)]
    t: f32,
    /// Draw every art style of the collection on one wall.
    #[arg(long)]
    styles: Option<PathBuf>,
}

fn main() {
    frameless::register();
    if std::env::args().nth(1).as_deref() == Some("museum") {
        let a = MuseumArgs::parse_from(std::env::args().skip(1));
        let Some((x, z)) = a.room.split_once(',').and_then(|(x, z)| Some((x.trim().parse::<i32>().ok()?, z.trim().parse::<i32>().ok()?))) else {
            eprintln!("frameless museum: --room must look like 0,0");
            std::process::exit(2);
        };
        let args = frameless::tool::Args {
            scene: a.scene,
            seed: a.seed,
            room: (x, z),
            shot: a.shot,
            look: a.look,
            inside: a.inside,
            map: a.map,
            explore: a.explore,
            t: a.t,
            styles: a.styles,
        };
        match frameless::tool::run(&args) {
            Ok(text) => print!("{text}"),
            Err(e) => {
                eprintln!("frameless museum: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    red_engine2::client::run(frameless::mode_for);
}
