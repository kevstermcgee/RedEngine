//! `killchain [MAP] [--fullscreen] [--name NAME]`: the game (see the library for what it is). `MAP` defaults to `maps/main.json`; `KC_SCRIPT=file.json` plays a scripted run with no
//! window and takes pictures offscreen; the player's name can also come from `RE2_NAME`.

use killchain::app::{run, Options};
use std::path::PathBuf;

const HELP: &str = "killchain [MAP] [--fullscreen|-f] [--name NAME]\n  MAP         the map to play (default maps/main.json)\n  --fullscreen start in borderless fullscreen\n  --name NAME  your name (or RE2_NAME)\n  KC_SCRIPT=file.json  play a scripted run with no window";

fn main() {
    let mut scene = None;
    let mut fullscreen = false;
    let mut name = std::env::var("RE2_NAME").ok().filter(|v| !v.is_empty());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return;
            }
            "-f" | "--fullscreen" => fullscreen = true,
            "--name" => match args.next() {
                Some(n) if !n.is_empty() => name = Some(n),
                _ => exit_with("--name needs a name"),
            },
            flag if flag.starts_with('-') => exit_with(&format!("unknown option {flag}")),
            _ if scene.is_none() => scene = Some(PathBuf::from(a)),
            _ => exit_with("only one map can be given"),
        }
    }
    env_logger::init();
    run(Options { scene: scene.unwrap_or_else(|| PathBuf::from("maps/main.json")), fullscreen, name });
}

fn exit_with(message: &str) -> ! {
    eprintln!("killchain: {message}\n{HELP}");
    std::process::exit(2);
}
