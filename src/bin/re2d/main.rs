//! `re2d`: the small native program that plays one 2D game (`NAME.game2d.json`), the file the Windows/Linux releases ship next to the game.
//!
//! It is `red_engine2 play2d` without the rest of the command line: the window, sound, saves and `F11` fullscreen are `red_engine2::play2d` (one implementation, so a game that
//! passes `verify` plays the same here and under `play2d`). What this file adds is what a player double-clicking an .exe needs: the game defaults to `game.game2d.json` next to the
//! program, the release build on Windows has no console window, and every failure is also written to `re2d.log` next to the program (the only place such a player would find it).
//!
//! usage: `re2d [GAME.game2d.json]` · `re2d --check GAME.game2d.json` (load the game exactly as the player would and exit 0 or 1: what a release runs before shipping it)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use red_engine2::play2d::{run, Options};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// `--check` runs unattended (a release job): no message box, only the exit code and the log.
static UNATTENDED: AtomicBool = AtomicBool::new(false);

/// Reports a failure on stderr, in `re2d.log` next to the program and (on Windows, where a double-clicked program has no console) in a message box, then exits with `code`.
fn fail(msg: &str, code: i32) -> ! {
    eprintln!("{msg}");
    #[cfg(windows)]
    if !UNATTENDED.load(Ordering::Relaxed) {
        let shown: String = msg.chars().take(1500).collect();
        rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("RedEngine 2D").set_description(shown).show();
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) {
        let _ = std::fs::write(dir.join("re2d.log"), format!("{msg}\n"));
    }
    std::process::exit(code)
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let check = args.peek().is_some_and(|a| a == "--check");
    if check {
        args.next();
        UNATTENDED.store(true, Ordering::Relaxed);
    }
    let game = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("game.game2d.json"))).unwrap_or_default());
    if !game.is_file() {
        fail(&format!("re2d: cannot find the game {}\nusage: re2d GAME.game2d.json", game.display()), 2);
    }
    if check {
        let text = std::fs::read_to_string(&game).unwrap_or_else(|e| fail(&format!("re2d: {}: {e}", game.display()), 2));
        match red2d::host::Host::default().init(&text, 1) {
            Ok(_) => {
                println!("re2d: {} loads", game.display());
                return;
            }
            Err(e) => fail(&format!("re2d: {} does not load:\n{e}", game.display()), 1),
        }
    }
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    if let Err(e) = run(Options { game: game.clone(), seed, save: None, mute: false, max_ticks: None }) {
        fail(&format!("re2d: {e}\n(run `red_engine2 validate {}` for the way out)", game.display()), 1);
    }
}
