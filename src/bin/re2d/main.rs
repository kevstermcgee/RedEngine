//! `re2d`: the small native program that plays one 2D game (`NAME.game2d.json`), the file the Windows/Linux releases ship next to the game.
//!
//! It is `red_engine2 play2d` without the rest of the command line: the window, sound, saves and `F11` fullscreen are `red_engine2::play2d` (one implementation, so a game that
//! passes `verify` plays the same here and under `play2d`). What this file adds is what a player double-clicking an .exe needs: the game defaults to `game.game2d.json` next to the
//! program, the release build on Windows has no console window, and every failure is also written to `re2d.log` next to the program (the only place such a player would find it).
//!
//! usage: `re2d [GAME.game2d.json]`
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use red_engine2::play2d::{run, Options};
use std::path::PathBuf;

/// Reports a failure on stderr and in `re2d.log` next to the program, then exits with `code`.
fn fail(msg: &str, code: i32) -> ! {
    eprintln!("{msg}");
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) {
        let _ = std::fs::write(dir.join("re2d.log"), format!("{msg}\n"));
    }
    std::process::exit(code)
}

fn main() {
    let game = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("game.game2d.json"))).unwrap_or_default());
    if !game.is_file() {
        fail(&format!("re2d: cannot find the game {}\nusage: re2d GAME.game2d.json", game.display()), 2);
    }
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    if let Err(e) = run(Options { game: game.clone(), seed, save: None, mute: false, max_ticks: None }) {
        fail(&format!("re2d: {e}\n(run `red_engine2 validate {}` for the way out)", game.display()), 1);
    }
}
