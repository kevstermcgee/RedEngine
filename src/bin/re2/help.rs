//! `re2 --debug-help`: the environment switches and hotkeys that exist for testing and screenshots. They used to be findable only by reading the source; a test keeps this
//! table complete (every `RE2_*` name mentioned anywhere in the client's code is listed).

/// `(name, what it does)` for every `RE2_*` environment variable the client reads.
pub(crate) const SWITCHES: &[(&str, &str)] = &[
    ("RE2_CHARACTER", "play as human, rat, wizard, cowboy, alien or robot (`--as` does the same; a scene's `player.humans_play_as` wins)"),
    ("RE2_CONNECT", "server to join, HOST:PORT (`--connect`)"),
    ("RE2_KEY", "the server's join key (`--key`)"),
    ("RE2_SERVER_FINGERPRINT", "sha256:... the server's TLS identity to pin when joining over QUIC (`--server-fingerprint`; a loopback server needs none)"),
    ("RE2_NAME", "the name shown in the lobby (`--name`)"),
    ("RE2_WINDOW", "x,y,w,h: a plain window at that place and size (tile two clients side by side)"),
    (
        "RE2_STATS",
        "1: print frame rate and the slowest update/draw every two seconds, and online the remote-player counters (drawn, undrawn, hidden by interest)",
    ),
    ("RE2_AUTOWALK", "forward | circle[:deg/s] | still: play by itself (a walker; combine with the next two)"),
    ("RE2_AUTOFIRE", "1: pull the trigger five times a second (with RE2_AUTOWALK)"),
    ("RE2_AUTOAIM", "1: a sentry that turns to the nearest enemy it can see and fires (with RE2_AUTOWALK)"),
    ("RE2_PLAYERS", "N (1-4): local players sharing the screen, as --players N"),
    (
        "RE2_SAVE_DIR",
        "folder: keep this game's settings and saved variables there, whatever folder the game runs from (an installed game sets it to `Saved Games\\<game>`)",
    ),
    ("RE2_RELAY", "HOST:PORT (a hostname works): a red_relay to register with when hosting, so a friend joins with a short code instead of port forwarding"),
    ("RE2_AMBIENCE", "0: start without the scene's ambience and mood music (an `audio` block); RE2_MUSIC=0 is the music toggle for the built-in loop"),
    ("RE2_LOG_CUES", "1: print every sound cue as it plays (check the feedback without listening)"),
    ("RE2_CONSOLE", "1: (Windows) the shipped, console-less copy of the game shows its output in the terminal that started it instead of re2.log"),
    ("RE2_FEEL", "hit | kill | hurt | dead | low | protected | flash: hold that screen effect (for screenshots)"),
    ("RE2_MUSIC", "0: start without the music"),
    ("RE2_VIEW", "third: start in third person"),
    ("RE2_PITCH", "degrees: start looking up (+) or down (-)"),
    ("RE2_WEAPON", "start with that weapon in hand (a firearm's name, or bat)"),
    ("RE2_FREEZE_SHOT", "seconds since the shot: hold the firearm's recoil and muzzle flash there"),
    ("RE2_FREEZE_SWING", "seconds into the swing: hold the bat's swing there"),
];

/// Keys that are for testing, not playing.
pub(crate) const HOTKEYS: &[(&str, &str)] = &[
    ("F3", "the debug overlay: frame rate, ping, and a counter for every way a player can fail to be drawn"),
    ("F12", "save a screenshot now (rendered offscreen, into --shot-dir, default out/shots)"),
    ("N", "music on or off"),
    ("Q", "first or third person"),
];

/// The text of `--debug-help`.
pub(crate) fn text() -> String {
    let mut s = String::from("Red Engine 2 client: debug switches (environment variables) and hotkeys\n\n");
    for (name, what) in SWITCHES {
        s.push_str(&format!("  {name:<18} {what}\n"));
    }
    s.push_str("\nHotkeys\n");
    for (key, what) in HOTKEYS {
        s.push_str(&format!("  {key:<18} {what}\n"));
    }
    s.push_str(
        "\nSeeing and asserting without a person at the keyboard\n  \
         --headless --script play.json --dump state.json   the real client loop with a script (`red_engine2 describe playtest`)\n  \
         --shot-at 5,10,20 --shot-dir out/                  pictures at those seconds, rendered offscreen (no focus, no visible window)\n  \
         --playtest --secs 60 --shots 12 [--host]           a scripted session with a contact sheet and a JSON report (`red_engine2 playtest MAP`)\n",
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `.rs` file under `dir`, recursively.
    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())).flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Every `RE2_*` name in the client's code is documented, and every documented name is read somewhere. "The client's code" is everything under `src/` except this
    /// table and the developer command line (`src/cli`, `src/tools`, `src/main.rs`: `RE2_SRC` and friends belong to `red_engine2`, not to a game), found by walking the
    /// directory, so a new file or a name read in a library module (`RE2_SAVE_DIR`) is covered without anyone keeping a list.
    #[test]
    fn every_switch_the_code_reads_is_listed() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let client: Vec<(String, String)> = files
            .iter()
            .filter(|p| {
                let rel = p.strip_prefix(&src).unwrap_or(p).to_string_lossy().replace('\\', "/");
                !(rel == "bin/re2/help.rs" || rel == "main.rs" || rel.starts_with("cli/") || rel.starts_with("tools/"))
            })
            .map(|p| (p.strip_prefix(&src).unwrap_or(p).display().to_string(), std::fs::read_to_string(p).unwrap_or_default()))
            .collect();
        assert!(client.len() > 50, "the walk found only {} files under {}", client.len(), src.display());
        for (file, text) in &client {
            for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| w.starts_with("RE2_") && w.len() > 4) {
                assert!(SWITCHES.iter().any(|(n, _)| *n == word), "{word} is read in src/{file} but missing from `re2 --debug-help` (src/bin/re2/help.rs)");
            }
        }
        for (name, _) in SWITCHES {
            assert!(client.iter().any(|(_, t)| t.contains(name)), "{name} is documented but no code reads it");
        }
    }

    #[test]
    fn the_help_text_lists_the_switches_the_hotkeys_and_the_headless_flags() {
        let t = text();
        assert!(t.contains("RE2_AUTOAIM") && t.contains("F12") && t.contains("--headless"));
    }
}
