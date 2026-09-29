# 2026-09-29. Verification honours the map: spawn height, player tuning, jump pads, start rules
Status: accepted
Summary: lint, reach, walk, build and offline play start at the first spawn at its height, walks use the scene's player tuning and jump pads, and an unconditional start rule that disables collision is open to the tools too.

## Context
The analysis tools promised "lint is clean and the walk passes means the level is playable" (AGENTS.md, ADR 0003), but they modelled a
simpler game than the one that shipped. `MapWorld::spawn` was the camera's x/z with feet at y 0, so a spawn authored on a 4 m deck linted as
"inside solid geometry" with "walkable 0 m^2" (Knockdown Alley), and the offline client started at the camera at y 0 while the server used
the spawn's y. A blueprint with `spawns: []` and spawns in its `scene` block self-checked from the first room's centre, failing `game check` on
a clean map. `walk`, `reach` and `verify` drove a fixed 3.2 m/s walker through `step_horizontal` / `vertical_step`, blind to the scene's
`player` tuning and to `jump_pads`, so a route only a pad can take was "unreachable" (Fling Delivery). A gate that a `start` rule opens on the
first tick was solid to lint, making its zone "50% reachable". And a `path` walk always started at the spawn, which three builders read as
"the first point is the start" and lost time to.

## Decision
- **The spawn is the first `spawns` entry, at its height.** `MapWorld` gains `spawn_y` (0 for the camera fallback); `ReachParams::start_y`
  seeds the flood-fill there by default; `checks.walk` and `checks.reach` entries without `from` start at the spawn on its floor, `from` starts
  on the ground floor, and `from_y` (now accepted by `reach` too) picks the floor; the `walk` command's default start carries the height. The
  offline client (`re2`) starts at `spawns[0]` (x, y, z, yaw) when the scene has spawns, exactly where a match would put the player.
- **A blueprint's merged spawn wins.** With no blueprint spawns and `scene.spawns` present, the compiled map's camera, self-check flood and
  generated `reach` / `walk` checks (with `from_y` when raised) start from that spawn.
- **The walker is the player.** `tools::walk::walk_from` drives `sim::player::step_player_tuned` with the scene's `player` tuning and
  `jump_pads` (forward input toward each waypoint, never sprinting or jumping), the same function the server, `sim` and the client run, so a
  pad on the route launches the walker as it launches a player. Lint's grid flood-fill still walks; a pad-only area stays "unreachable" to
  lint (a `checks.walk` proves it instead).
- **Unconditional `start` rules apply.** `MapWorld` collects `collision: [id, false]` actions of `start` rules that have no `if` into
  `collision_disabled`, drops those objects' colliders and ground and marks their items non-solid, so lint, reach and walks see the gate open
  as the game does from tick one. A gate a later rule opens stays solid: the tools cannot know when.
- **The old surprise explains itself.** A failing `path` walk with no `from` whose first point is more than 2 m from the spawn says that the
  walk starts at the spawn and how to start elsewhere.
- Proven by `tests/verify_map.rs` through the real CLI and client: a spawn on a 3 m deck lints with 0 errors, reach and walks run on the deck,
  the client's feet start at y 3; a blueprint whose only spawn is in its `scene` block builds and verifies; a walk over a jump pad reaches a
  2 m platform and fails without the pad; a start-rule gate leaves its zone fully reachable and the walk through it passes, while the same
  map without the rule seals the zone and names the gate; the first-point hint appears.

## Consequences
- A raised or authored spawn is an ordinary thing to want; maps are no longer shaped around the tools' assumptions.
- Every existing map keeps its results: with no `spawns` the camera fallback is unchanged, and the tuned walker at the default tuning walks
  the same routes (`house_walk`, `test_lab`, `maps_verify` unchanged). Maps with spawns now lint and walk from `spawns[0]` rather than the
  camera, and the offline client starts there; `examples/test_lab.json`'s camera and first spawn are 3 m apart in the same room.
- Left out: the flood-fill does not launch off pads (lint may still call a pad-only area unreachable), a `jump: true` walk leg, and rules whose
  condition the tools cannot evaluate.
