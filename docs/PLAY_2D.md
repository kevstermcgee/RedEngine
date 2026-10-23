# Playing and shipping 2D games

A 2D game is one JSON file, `NAME.game2d.json` (`red_engine2 describe 2d`). Games run natively: `red_engine2 play2d` opens a window, plays the game's own synthesized sound and music, and keeps progress between runs. There is no browser or WebAssembly path.

## The loop

1. `red_engine2 validate G` — the file is well formed and every name resolves; each error says the fix.
2. `red_engine2 verify G` — native and headless, about a second: scripted scenarios, `checks.reach` map analysis, frames you can look at, sound waveforms. `sim G --only NAME` debugs one scenario, `frame G out.png` shows a frame.
3. `red_engine2 play2d G` — play it. `--mute` for no sound, `--seed N`, `--save FILE` to keep progress somewhere else, `--max-ticks N` for a smoke test that quits by itself.

## Platformers: proving a level can be finished

A platformer's reach depends on jumps, speed and moving enemies, so `checks.reach` cannot answer it; `red_engine2 solve G --level 1-1 --goal "mode == 4" --avoid "mode == 3"` plays it instead: a beam search over the keys a player can hold each tenth of a second, every candidate the real simulation, dropping candidates that meet `--avoid`. It prints `hold`/`wait` steps that replay exactly as a scenario `script` (start the scenario in the same level, with the same seed). `--progress EXPR` says what "further along" means when it is not the player's x (a block to bump first, a log to go down). `examples/2d/honeybound` stores its runs in `solutions.json` and builds them into one scenario per stage, so `verify` proves every stage (ADR 2026-10-12-2d-platformers-are-data-contact-sides-become-levels-and).

## What the player does

- The picture is the same CPU renderer `verify` and `frame` use, scaled to the window with the aspect ratio kept (whole-number scaling for games that ask for `integer`). F or F11 toggles fullscreen in every game (no game may bind them). The game's `icon` sprite is the window's title-bar and taskbar icon (and RedEngineGames' installer makes it the shortcut icon).
- 60 simulation ticks a second from a fixed clock. A stall never turns into a burst of more than five ticks, and losing focus releases every held key.
- Keys: arrows or WASD move, Space/Z/J is `action`, Shift/X/K is `secondary`, Escape/P is `pause`; the mouse is the pointer and left click is a click. Games read the same names the rules use (`KeyA`, `ArrowLeft`, `Space`).
- A gamepad (the first connected; Xbox names, PlayStation/Nintendo positions mapped by gilrs): left stick or d-pad move, A is `action`, X, B or the right trigger is `secondary`, Start or Back is `pause`. Only changes are fed in, so keyboard and pad work side by side; the `gamepad` variable is 1 while one is connected (show pad buttons then).
- What the rules ask of the platform: `window: "fullscreen"|"close"` (close saves first), `save: true` (write progress now), `export_music: true` (the track playing now becomes `<Music folder>/<game title>/<track title>.wav`; the game hears `music_exported` or `music_not_exported`). With `pause_if` a pause menu is made of rules: the world and its clock stand still while press rules still run (ADR 2026-10-16-a-2d-game-s-clock-stands-still-while-its-world-is-paused).
- Started by a player (`RE2_PLAYER=1`, which the game's `Play-<slug>.exe` and the RedEngine Games app set; `--fullscreen --intro` by hand), the window opens in borderless fullscreen and the "RedEngine Games" title card (`ui::title_card`) plays for about two and a half seconds before the first tick; any key or click skips it. F or F11 toggles fullscreen, as in every game (also while the card plays, without skipping it).
- Progress is written atomically every two seconds while it changes and on exit, to `progress.json` under the per-game save directory (`RE2_SAVE_DIR`, else the user config directory keyed by the game id).

## Publishing

A 2D game reaches RedEngineGames as `"kind": "2d"` in `games-publish.json`, with its page in `pages/<slug>/`, in one pull request (`docs/GAMES_PUBLISHING.md`, `CONTRIBUTING.md`; the `make-game` skill walks it). There is no single `game publish2d` command yet, and the game's `Play-<slug>.exe` is built by RedEngineGames' own workflow.
