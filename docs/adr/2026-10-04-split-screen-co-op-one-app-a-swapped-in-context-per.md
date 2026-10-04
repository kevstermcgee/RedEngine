# 2026-10-04. Split-screen co-op: one app, a swapped-in context per player
Status: accepted
Summary: Up to four local players share the screen by swapping a per-player context into the single-player client, rendering each into a view texture the compositor tiles into the window.

## Context
The user wants native split-screen for up to four local players that is reliable, stable and smooth. `re2`'s `App` is one player's client: nearly five thousand lines read and write that player's camera, body, physics position, weapon and input as flat fields. A rewrite into `Vec<Player>` would touch all of it and risk the single-player game. Rendering was measured first (docs/analysis/2026-10-04-split-screen-survey-and-plan.md): the cost per view is geometry-bound (about 100 ms per 0.55-0.75 M triangles on the software rasteriser, whatever the pixel size), four views cost four times one view, and composing them is free.

## Decision
- **Swap, don't rewrite.** A guest's state lives in `PlayerCtx` (`src/bin/re2/coop.rs`). `App::as_player(slot, f)` exchanges the per-player fields with the guest's, runs the very code the first player runs (movement, tick, camera, weapons, pad), and exchanges them back. `swap_player!` destructures `PlayerCtx` with no `..`, so a field added to the struct and forgotten in the swap does not compile. One player is the case of no guests: nothing is swapped and nothing changes.
- **Shared per game, own per player.** The scene, rules, clock, props, audio and the world stay on `App`; the loose props and the flashlight stay the first player's. Rules see every player's body (`fixed_step_rules`), a teleport from any of them applies to them.
- **Render once per player.** One `LiveRenderer` sized for one view draws each player in turn into a view texture; `SplitScreen` (`src/split_gpu.rs`) blits them into the layout's rectangles (`splitscreen::layout`, equal sizes, a gutter), with a HUD per player and a global overlay (pause menu, cards) over all. Streaming follows every eye (`wanted_many`), and `view_distance(players)` shrinks the draw distance with the crowd so the frame stays inside the triangle budget (`tests/split_render.rs`).
- **Devices.** Player 1 plays on the keyboard and mouse, each other player on the next gamepad in connection order (`Pads`); `--pads-only` puts player 1 on a pad too. Too few pads is a plain message, not a half-started game. An unplugged pad pauses the game and names the player.
- **Offline only.** `--players N` is for a local game; online and the kart racer stay one player per process.
- **Seeing it without a window.** A script hands the controls over with `{"player": N}`, `{"shot": ..., "camera": "split"}` takes the composed picture, and the dump lists `/players[]`.
- **Sound.** The speakers are shared: a guest's footsteps and swings are quieter than the first player's and lean toward where the guest is (`splitscreen::guest_mix`).

## Consequences
Single-player code paths are untouched and cost nothing. Every new per-player field on `App` must also go into `PlayerCtx` and the swap list (the compiler enforces the struct side only; the doc comment on `coop.rs` says so). Four views are four renders: a scene that is heavy for one view is four times heavier, which is why the draw distance falls with the player count. Undo: drop `--players`; deleting `coop.rs`, `split_gpu.rs` and the `split` branches in `draw`, `start_game` and `events` returns to the single-player client.
