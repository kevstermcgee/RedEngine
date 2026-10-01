# 2026-10-01. Persisted per-game audio settings and a music-download button
Status: accepted
Summary: The pause menu's MUSIC/SOUND toggles are saved per game and reloaded on relaunch; a small arrow button saves the generated music loop as a WAV wherever the player chooses

## Context
`re2` already generates its own music (`music::loop_samples`, ADR 0008/0056) and lets the `N` key silence it
(`App::toggle_music`), but that choice lived only in memory: every relaunch started from the scene's `"music"`
key or `RE2_MUSIC` again, there was no equivalent toggle for sound effects at all, and there was no way to keep
a copy of a track a player liked. The pause menu (`ui::screens::pause_layout`) had only RESUME, FULLSCREEN and
QUIT.

## Decision
- **`settings.rs`** (new, `gfx`-gated like `audio.rs`/`music.rs`): a `Settings { music, sfx }` keyed by
  [`key_for`], which walks up from the played map to an enclosing `game.json` (the same ancestor-walk
  `game::destination_for_blueprint` already uses) and combines the project's name with a short hash of its
  canonical directory — so the setting "stays with the game" (two different games, even same-named ones, never
  share a file) rather than being one on/off switch for every game on the machine. `load`/`save` live under the
  OS's normal config directory (`$XDG_CONFIG_HOME`/`%APPDATA%`/`$HOME/.config`, the same three-branch,
  no-new-dependency approach already written for the upgrade-tooling build cache). A missing or corrupt
  settings file is never an error — `load` just returns `Settings::default()` (everything on, today's
  behavior), the same "a missing piece of the audio stack never takes the game down" reasoning `Audio::new`
  already uses.
- **`Audio` gets an `sfx_on` flag** (`set_sfx_enabled`), checked once inside `play`/`play_at` instead of at
  every one of their call sites across combat/kart/UI code. Music keeps its own existing volume-based mute
  (`set_music_volume`).
- **The pause menu gains MUSIC/SOUND toggle buttons and a `↓` (download) button** (`PauseAction::ToggleMusic`,
  `ToggleSfx`, `DownloadMusic`; `ScreenOpts::music_on`/`sfx_on` so `ui-shot`/`ui-check` can show and audit every
  on/off combination). The arrow is a new glyph in the shared 5x7 bitmap font (`tools::font::GLYPHS`), which the
  UI kit and the analysis tools' labelled images already share — no new font dependency.
- **Downloading never guesses a folder.** It opens the OS's native "Save As" dialog (`rfd`, a new optional
  dependency folded into the existing `gfx` feature alongside `winit`/`rodio`/`wgpu`, so the headless/server
  build is unaffected) defaulting to `<map-name>-music.wav`; the player picks the destination, so a redirected
  Downloads folder, a different drive, or no such folder at all is never the engine's problem to solve. The WAV
  itself is a plain 44-byte RIFF/WAVE header plus 16-bit PCM (`audio::wav_bytes_i16`) — no encoder dependency,
  consistent with ADR 0008 (procedural, no imported/licensed audio): this exports what the engine already
  generated, it does not import anything.
- **Scope**: this covers the standard `re2` client (ADR 0024's default path for virtually every game). A custom
  Rust client (ADR 0043 — `topdown_switch`, the in-engine `killchain` example) already owns its entire
  input/UI/audio and is not touched here; it would need to call `settings`/`audio::wav_bytes_i16` itself to get
  the same behavior.

## Consequences
A player's audio choices now survive a relaunch, and sound effects are finally toggleable (not just music). Saving
music costs one small, well-established new dependency (`rfd`), gated out of headless/server builds. Not done:
volume sliders (the ask was "toggle"), coverage for custom-Rust-client games, and the no-folder constraint means
the save flow blocks the event loop while the modal dialog is open (acceptable: the game is already paused to
reach this button). To undo: delete `settings.rs`, the three new `PauseAction` variants and their
`pause_layout`/wiring additions, the `↓` glyph, `Audio::sfx_on`, and the `rfd` dependency; nothing else is
affected.
