# 0010. Headless multiplayer match server (NOT BUILT)
Status: proposed

## Context
The goal is an online prop-hunt game with a lightweight server running headless. **No networking, match
server, orchestrator or lockstep code exists yet** — only this record of the constraints found while
building the single-player engine, so the eventual design starts from facts.

## What is true today
- Simulation and rendering are entangled in one crate. The authoritative-looking physics
  (`ground_height_at`, `colliders_on_floor`, `resolve_collision`, collider/ground collection from a
  `Scene`) lives in `src/viewer.rs`, next to wgpu code; `player.rs` imports it. `winit`/`wgpu`/`rodio`
  are unconditional dependencies.
- The fixed tick already exists (`FIXED_DT = 1/60`, ADR 0005) and is deterministic; the offline
  `walk` tool replays it with no window — proof that physics can run without a GPU.
- Scene loading (`schema::parse`, macros, prefabs) and `MapWorld` are already headless.

## Decision (proposed — open, not yet decided)
1. Extract the headless core — colliders, ground candidates, `player` step functions, scene→world
   collection — into a module (or workspace crate) with **no** wgpu/winit/rodio dependency; the viewer
   and tools depend on it, the server depends on only it. Do this refactor first; it changes no behaviour
   and `tests/house_walk.rs` proves it.
2. Define the client/server boundary as a small trait (input in, state snapshot out) so the server
   can be tested without sockets, mirroring how `walk` tests physics without a window.
3. Then choose orchestrator-vs-lockstep and write it up as a follow-up ADR that supersedes this one.

## Consequences (of waiting vs. acting)
Cheap now, expensive later: every new physics rule added to `viewer.rs` deepens the entanglement.
Until step 1 lands, keep new simulation logic in `player.rs`-style pure functions, not in `App`.
