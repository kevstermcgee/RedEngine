# Split-screen co-op: survey and plan (2026-10-04)

The ask: native split-screen co-op for up to four players on one machine, "reliable, stable, and smooth as hell". This is the survey the audio and Marcel work promised before any code, with the measurements that make "smooth" a number.

## What the engine has
- **One local player per `re2`.** `App` (src/bin/re2, ~5 k lines) holds one player's state as flat fields: the camera, `physics_pos` / `foot_y` / velocities, the body object, weapon and swing state, view mode, walk phase, target under the crosshair, the flashlight. About 200 references. Offline it steps `sim::player::step_player_on_tuned` itself and runs `RulesEngine::step_props(tick, &[player], ..)`, which already takes a *slice* of players. Online it presents one `NetSession`.
- **The simulation is already multi-player.** `MatchSim` (the server's) holds up to `MAX_PLAYERS`, rules take `who`, `slot`; bots and props work. Nothing in `sim` is single-player.
- **Rendering is one camera per `render_view` call** into one target sized to the window (`LiveRenderer` owns its depth, MSAA and shadow targets; its final pass resolves into whatever `target_view` it is given). The object uniforms are staged once per call and culled against that call's camera. `StreamLayer` (the procgen world) streams around the single eye it is given.
- **Input**: keyboard and mouse into `App`; one `controller::Poll` (gilrs) reads *a* pad. UI: `ui` layouts are pure and sized to any rectangle; the HUD and the pause menu are painted to one overlay for the window.
- **Audio**: `mixer::hear` (this week) answers what several listeners hear; the single player's cues use `sfx::spatial` with one listener.

## Decisions I am taking
1. **Offline first, built into `re2`, for every walking and shooting game** (Marcel, prop hunt, the lab): N local players in the one process, each a full player (own camera, body, weapon, HUD). Kart racing and online play keep their one-player-per-process model; local players in a hosted match is a later step on the same pieces. This is the case "couch co-op" means and the one the offline `App` can do without a server.
2. **Do not rewrite `App`: give it a player context to swap.** The per-player fields move into one `PlayerCtx` struct; the frame loop runs the existing update for each local player by swapping that player's context in and out. The 5 k lines keep working unchanged for one player (swap of nothing), and a second player is the same code with a different context. That is the smallest change that can be proved not to break the single-player game (every existing test keeps passing).
3. **Viewports are rendered offscreen at viewport size and copied into the window.** One `LiveRenderer` sized for the largest viewport renders each player's view in turn into a viewport-sized texture (HUD included), which is copied into its rectangle of the swapchain. A single viewport is the window, exactly today's path. No change to the passes themselves, and the cost is what a split screen honestly costs: N scene draws at 1/N of the pixels.
4. **The procgen world streams around all the eyes** (the union of their discs), so two players far apart each have their own ground.
5. **Layouts**: 1 full; 2 side by side on a wide window and stacked on a tall one; 3 as two on top and one below (the third spans the bottom, or a corner left empty for a map); 4 a 2x2 grid. A small gutter; each viewport has its own aspect and a field of view corrected for it.
6. **Devices**: player 1 keyboard and mouse (or the first pad); each other player takes the next gamepad in connection order; with too few pads the extra players are refused with a message that says how many are missing, rather than sharing a keyboard silently. A pad that disconnects pauses the game and names the player.
7. **Smooth means a number**: a frame budget per player count, measured by a headless benchmark on the software adapter (relative cost, N=1 against N=2,3,4) and on the CPU side (`update`, the draw submission, uploads), checked by a test, plus no stalls from streaming (chunk uploads per frame are capped).

## Phases (each a PR)
1. **Foundations (headless, tested):** viewport layouts and field-of-view rules; device assignment; the benchmark harness with the measured baseline. This note.
2. **Rendering:** viewport-sized renderer targets and copy-composition; multi-eye streaming; `frame --players N` / an offscreen contact of N views, so it can be looked at.
3. **The player context and N local players in `re2`:** `--players N`, per-player input, bodies, HUD, rules with N players, bodies visible to each other, teleport and spawn per slot, the pause menu and cards for the group.
4. **Audio listeners and polish:** all local players as listeners; hot-join/leave if wanted; the frame-budget test in CI.

## Risks
- The budget cannot be proved on a real GPU here; I will report relative cost and CPU time and keep the draw count per view as low as the single-player path.
- Online and kart paths stay one-player-per-process: stated plainly rather than half-supported.
- Four shadow-casting views at once is the heaviest part; shadow maps are 2048 and the follow-shadow radius is per view, so the cost is four small passes, and I will measure before deciding on a lower resolution for 3-4 players.

## Measurements so far
`frame examples/marcel/marcel.json` on the software adapter (llvmpipe, 4 cores), best of three, whole command including about 0.85 s of start-up (adapter, scene, streaming the view): 1920x1080 2.29 s, 960x540 1.19 s, 480x270 0.93 s. So pixels cost about 1.4 s per 1080p frame and 0.37 s per quarter-size view, and a 2x2 split at 1080p fills the same number of pixels as one full-window frame: on a pixel-bound GPU split-screen is about the cost of one frame plus four times the geometry. The geometry-side cost (draw submission, uniforms, culling, chunk upload) is what phase 2 will measure per player count.

## Progress
- **Phase 1 built:** `src/splitscreen.rs`: layouts for 1 to 4 players (equal-sized views, 2x2 grid with an empty place for three, side by side or stacked for two), the field-of-view rule for narrow views, device assignment (player 1 keyboard and mouse, the rest the next pads; a shortfall says how many are missing) and which player owns an unplugged pad.
- **Phase 2 built:** `src/split_gpu.rs` (`SplitScreen`: one `LiveRenderer` sized for one view draws each player in turn into a single texture that is blitted into the player's rectangle, each with its own HUD overlay and a window-wide overlay on top), multi-eye streaming (`Streamer::update_many`: the world follows every viewer; tested), `OffscreenSplit` and `red_engine2 splitshot` to look at it, `splitscreen::view_distance` (270 m for one player, 230 for two, 190 for three or four). Tests (`tests/split_render.rs`): four views are four different pictures in their rectangles with dark gutters; one view equals the single-player path; a HUD appears only in its own view; four views cost four views (a measured 4.0x one view of the same size on the software adapter, bound 4.7x); a crowded screen stays under three million triangles (2.5 M measured).
- **Where the time goes (software adapter):** a view's fixed cost is its geometry: about 100 ms for 0.55 to 0.75 M visible triangles whatever its pixel size (llvmpipe does a few million triangles a second; a real GPU does hundreds of millions). So the figure to hold on any machine is triangles per frame, and the budget is under three million with four players.
