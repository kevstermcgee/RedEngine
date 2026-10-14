# 2026-10-14. A game ships as its own binary: the client is a library with a game-mode seam
Status: accepted
Summary: The first-person client becomes library code a game crate runs with its own GameMode, generated worlds draw through one WorldLayer trait, packaging knows a game's binary, and Frameless moves to games/frameless behind those seams

## Context
ADR 2026-10-13-the-next-phase, item 5: "`package`, `game play` and `publish` know a game binary (Killchain first). Add the two engine seams above,
then move Frameless to a crate behind them, its saves and a seeded visit unchanged." The seams are findings 1 and 2 of
docs/analysis/2026-10-11-frameless-feedback.md: one trait for a generated world that both renderers draw (`models`, `colliders_near`, `lamps`,
`shader_params`), and a game-mode hook in the live client (update after the player moves, freeze input, own the camera, the HUD, the pause menu, keys,
footsteps, the state dump, script targets). The owner said to go ahead on 2026-10-14.

Measured at 94e5263 before deciding:
- The live client is the binary `re2`: 7,389 lines in `src/bin/re2/`, every part an `impl App` block. The museum's client (`src/bin/re2/museum.rs`,
  889 lines) is one of those blocks and reads or writes about fifty of `App`'s fields (the player's position and velocities, the camera, pause and
  cards, the window, audio and music, settings, the scene's lights, the GPU), from eleven of the client's files.
- Killchain left the engine (S6) by writing its own client on the public API (`games/killchain`: `app.rs`, `game.rs`, `input.rs`, ...). That works
  for a game whose client differs; Frameless is the first-person client plus a mode, and a copy of `re2` in its crate would be the fork ADR 0024 forbids.
- Both renderers (`render.rs`, `viewer.rs`) hold the meadow (`stream_gpu::StreamLayer`) and the museum (`museum_gpu::MuseumLayer`) as two named
  fields with their own construction, update, shadow and draw calls.
- `red_engine2`'s integration suites start `re2` through `CARGO_BIN_EXE_re2` (`client_headless` and others), so the binary stays in the engine
  package (ADR 2026-10-14-the-server-binary-stays-in-the-engine-crate-while-red: Cargo gives a test only its own package's binaries).

## Decision
- **The client is library code.** `src/bin/re2/*` moves to `src/client/` (the engine library, behind `gfx`) as `red_engine2::client`; the `re2`
  binary becomes `fn main() { red_engine2::client::main() }`. Structural only, its own pull request, `re2`'s headless scripted runs as the parity.
- **`client::GameMode`** is the seam a game plugs into the client: one trait whose methods are the feedback's entry points (`start`, `frame` after the
  player moved, `input_frozen`, `camera`, `hud`, `pause_rows`/`pause_action`, `window_event` for its keys, `footstep`, `dump`, `target_bounds` for
  script targets, `fx`, `sync_gpu`), each with a default that does nothing, and a `ModeContext` that is the only door to the client's state a mode
  gets (the player's pose and teleport, the camera, pause, cards and overlay, audio, music steering, settings, the scene's lights, the viewer). A game
  binary runs `client::run(Box::new(MyMode))`; plain `re2` runs with no mode. The museum's client is the first mode, still in the engine.
  *As built (2026-10-14, the pull request that added it):* `client::run` takes a `ModeFactory` (`fn(&Scene) -> Option<Box<dyn GameMode>>`), not a
  mode, because the client builds a fresh state for a restart and for the map switcher and a mode belongs to its scene; plain `re2` runs
  `client::engine_mode`, which gives a museum scene the museum's mode until Frameless moves. The camera is placed through `ModeContext::set_camera`
  from `frame` (no `camera` method), and the pause menu is the mode's whole when it says so (`owns_pause`, `repaint_pause`, `left_pause`) rather than
  rows added to the client's: the museum's menu and map share nothing with the client's. A context call that comes back to the client (opening the
  pause menu) does not reach the mode, which is out of the client while its method runs; the mode repaints its own after it.
- **`WorldLayer`** is the seam a generated world draws through: the renderers hold `Vec<Box<dyn WorldLayer>>` instead of named fields, built from
  the scene by layer factories registered like scene blocks (ADR 2026-10-13-a-scene-block-can-be-defined-above-the-core); the meadow and the museum
  implement it. The CPU side a client and the tools ask (`colliders_near`, `lamps`, `shader_params`) is part of the mode or the world, not the renderer.
  *As built (2026-10-14):* `world_layer::WorldLayer` (`prepare`, `settle`, `set_view_distance`, `draw_shadow`, `draw_main`, `draw_glow`, and
  `still_light` for the offline renderer, which has no client to light its scene), `LayerDef { name, build, art }` defined by
  `world_layer::define_layers` beside the blocks (`scene_blocks::register`), the meadow built in first because `procgen` is a core field. A layer the
  client feeds is reached by its type (`LiveRenderer::layer_mut::<MuseumLayer>()`); a still picture's museum carries its own visit (`StillMuseum`).
  The game's painted-surface shader (`museum.art`) comes from the layer definitions (`art`), so neither renderer names a museum type.
- **A game binary is a package's binary.** A playable in `games-publish.json` and a game project's `game.json` may name the executable that runs it
  (`"binary"`, default `re2`); `package`, `game play` and `publish` build and launch that one (Killchain's `killchain` first). The RedEngineGames
  builder, which builds a RedEngine commit and launches each playable, learns the field in a pull request of its own in that repository, for the
  owner to merge (it changes what players download).
  *As built (2026-10-14):* the binary is a package of the engine's workspace with a `[[bin]]` of its own name, built on its own (`cargo build -p
  killchain --bin killchain`): in one cargo build with the engine's binaries its dependency on the engine's graphics would reach the headless server
  through feature unification. `package` builds every such game (`package::game_binaries`: `games/killchain`); `scripts/red play-local` builds the
  project's; `game play`, which joins with re2's `--connect`, says that a game with its own client joins from its own menu; `game publish` lists the
  playable with its `"binary"` and without re2's `--host`. Killchain's download in RedEngineGames played the map with `re2` until then (re2 warns that
  a `shooter` map has a client of its own).
- **Frameless moves to `games/frameless`** behind those seams: `src/museum`, `museum_gpu`, the museum mode and the `museum` tool. Its binary registers
  the `museum` block and its layer and runs the client with its mode. Parity: the same rooms, plaques and keys for the same seed, and a save written
  before the move loads after it. Then `src/museum` is gone from the engine crate.
- Each bullet is its own pull request in this order, structural moves never mixed with behaviour.

## Consequences
A game whose difference is a mode (a museum, a kart race) is a small crate, not a fork of the client and not engine code. The client's state gets a
public face (`ModeContext`) that has to stay small: what a mode may change is decided there. `re2` with a museum scene no longer works once Frameless
has moved (the block is Frameless's); `frameless` runs it. Undo per step: move `src/client` back under `src/bin/re2`, inline the mode calls, name
the layers again.
