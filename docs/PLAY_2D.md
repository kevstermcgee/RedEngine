# Playing and shipping 2D games

A 2D game is one JSON file, `NAME.game2d.json` (`red_engine2 describe 2d`). Games run natively: `red_engine2 play2d` opens a window, plays the game's own synthesized sound and music, and keeps progress between runs. There is no browser or WebAssembly path.

## The loop

1. `red_engine2 validate G` — the file is well formed and every name resolves; each error says the fix.
2. `red_engine2 verify G` — native and headless, about a second: scripted scenarios, `checks.reach` map analysis, frames you can look at, sound waveforms. `sim G --only NAME` debugs one scenario, `frame G out.png` shows a frame.
3. `red_engine2 play2d G` — play it. `--mute` for no sound, `--seed N`, `--save FILE` to keep progress somewhere else, `--max-ticks N` for a smoke test that quits by itself.

## What the player does

- The picture is the same CPU renderer `verify` and `frame` use, scaled to the window with the aspect ratio kept (whole-number scaling for games that ask for `integer`). F11 toggles fullscreen.
- 60 simulation ticks a second from a fixed clock. A stall never turns into a burst of more than five ticks, and losing focus releases every held key.
- Keys: arrows or WASD move, Space/Z/J is `action`, Shift/X/K is `secondary`, Escape/P is `pause`; the mouse is the pointer and left click is a click. Games read the same names the rules use (`KeyA`, `ArrowLeft`, `Space`).
- Progress is written atomically every two seconds while it changes and on exit, to `progress.json` under the per-game save directory (`RE2_SAVE_DIR`, else the user config directory keyed by the game id).

## Not built yet

- A gamepad in the 2D player.
- Packaging a 2D game as its own Windows `.exe` for RedEngineGames.
