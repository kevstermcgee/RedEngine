//! `re2d`: the small native program that plays one 2D game (`NAME.game2d.json`), the file the Windows/Linux releases ship next to the game.
//!
//! It is `red_engine2 play2d` without the rest of the command line: the window, sound, saves and `F11` fullscreen are `red_engine2::play2d` (one implementation, so a game that
//! passes `verify` plays the same here and under `play2d`). What this file adds is what a player double-clicking an .exe needs: the game defaults to `game.game2d.json` next to the
//! program, the release build on Windows has no console window, and every failure is also written to `re2d.log` next to the program (the only place such a player would find it).
//!
//! usage: `re2d [GAME.game2d.json] [--seed N]` (each launch picks a new seed and prints `seed N`; `--seed N` replays that run's random draws)
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut given_seed = None;
    let mut positional = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--seed" {
            match it.next().and_then(|s| s.parse::<u64>().ok()) {
                Some(n) => given_seed = Some(n),
                None => fail("re2d: --seed needs a whole number (the `seed N` an earlier run printed)", 2),
            }
        } else {
            positional.push(a.clone());
        }
    }
    let game = positional
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("game.game2d.json"))).unwrap_or_default());
    if !game.is_file() {
        fail(&format!("re2d: cannot find the game {}\nusage: re2d GAME.game2d.json [--seed N]", game.display()), 2);
    }
    let seed = red_engine2::play2d::launch_seed(given_seed);
    eprintln!("seed {seed} (replay this run's random draws with --seed {seed})");
    if let Err(e) = run(Options { game: game.clone(), seed, save: None, mute: false, max_ticks: None }) {
        fail(&format!("re2d: {e}\n(run `red_engine2 validate {}` for the way out)", game.display()), 1);
    }
}
