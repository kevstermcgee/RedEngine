# Glossary

Red Engine 2 vocabulary in one place. `red_engine2 search <term>` finds these too (and `describe
glossary` prints the lot). Terms are grouped; each line says what it *is* and where to go next.

## The two programs

- **`re2`** — the real-time first-person game/viewer (`src/bin/re2.rs`). Plays one map. Windowing,
  input, held item, audio. Never analyses maps.
- **`red_engine2`** — the offline CLI (`src/main.rs`). Validates, renders, lints, plans, edits and
  searches maps; no window. Also the engine's self-description (`describe`, `search`, `src`).
- **the tools** — everything under `src/tools/`. They call the *same* collision/physics functions as
  `re2` (see ADR 0003), so "the tools say it's walkable" means it is.
- **MCP server** — `mcp_server.py`, a thin wrapper exposing the CLI commands as MCP tools.

## Scene language

- **scene** — one map: a JSON file with `meta`, `camera`, `lights`, `objects`, optional `zones`,
  `prefabs`, `post`, `checks`. Spec: `SPEC.md`.
- **object** — one entry in `objects`; has a unique `id` and a `type` (primitive, `wall`, `fence`,
  `stairs`, `group`, `humanoid`, `prop`, `prefab`). `red_engine2 describe objects`.
- **primitive** — `box`, `sphere`, `cylinder`, `cone`, `capsule`, `plane`. **Only `box` collides.**
- **macro / sugar** — an object type expanded into plain boxes *at parse time* (`wall`, `fence`).
  Nothing downstream ever sees the macro (ADR 0006). Lives in `src/macros.rs`.
- **prop** — a Rust-built object with a custom collision policy (sofa, crate, vending machine…),
  defined in `src/props.rs`. 39 of them. Origin = middle of the **base**, front = local **+Z**.
- **prefab** — a parametric group of objects authored in **JSON** (`assets/*.json`); placed like a
  prop with `"type":"prefab"`. ~100 of them. New assets need no Rust (ADR 0002). Expanded at parse
  time by `src/prefabs.rs`. `$param` / `"=expr"` / `extends` are its template syntax.
- **catalogue** (`catalog`) — the searchable list of every prop and prefab, with real sizes, tags and
  a paste-ready snippet. `--sheet out.png` renders a labelled contact sheet.
- **zone** — a named rectangle (`id`, `rect`, `y`) declaring a room or yard. Affects nothing in
  rendering or physics; it is how the tools talk about places (`reach`, `plan`, `tour`, `scatter`).
- **`post`** — the clarity post-pass: depth-based contact AO + silhouette outline (`postfx.wgsl`).
- **`checks`** — a scene's own regression tests (lint budget, reach, walks, object asserts, golden
  views), run by `verify`. Test-enforced for every recipe.
- **`lint_ignore`** — per-object list of lint codes to silence (e.g. `["unreachable"]`).
- **`collide: false`** — any object can opt out of blocking the player.

## Maps and examples

- **the four maps** — the game's plan: **house, school, office, convenience store**, all sharing one
  prop/prefab library (the same apple may appear in every map — intentional; ADR 0009). House is done
  (`examples/house.json`); the other three are seeded by recipes.
- **recipe** — a complete, lint-clean, `verify`-passing example map in `recipes/`. `recipe <name>
  --new my.json` copies one to start from. `rooms_and_door`, `two_floor_house`, `convenience_store`,
  `classroom_wing`.
- **example** — a scene in `examples/` (demo/cinematic/reference), not necessarily walkable-lint-clean:
  `house.json` (reference map), `prop_hunt_yard.json`, and the cinematic `hello_world`/`orbit_walk`.
- **golden view** — a recorded reference render (`recipes/golden/<scene>/<name>.png`); `verify`
  diffs against it (renders are bit-deterministic). `--bless` re-records.

## Physics and walkability

- **fixed step** — player physics ticks at 1/60 s regardless of frame rate; the rendered camera
  interpolates between ticks (ADR 0005).
- **foot height / foot_y** — the y of the player's feet. Everything vertical is relative to it.
- **body band** — the vertical slice `foot_y+0.05 .. foot_y+2.0` in which colliders block the
  player; re-evaluated every tick against the *current* foot height. This is why upstairs walls need
  their own objects at the upstairs `y`.
- **ground snap** (`GROUND_SNAP_EPS`, ~0.35 m) — the largest step-up the player takes freely. A
  collider whose top is higher than that blocks; lower is stepped onto. The one rule behind floors,
  slab edges and stairs (ADR 0004).
- **ground candidates** — every walkable top surface in the scene (box tops, stairs ramps), collected
  once at load; `ground_height_at` picks the highest *reachable* one at an xz.
- **stairs ramp vs treads** — stairs are drawn as stacked box treads but walked as a smooth linear
  ramp, so climbing isn't a washboard. Climb from the bottom end only.
- **headroom** — 2.05 m (`PLAYER_HEADROOM`). Anything lower over walkable floor blocks or is flagged.
- **walkable grid** — the 2-D grid `reach`/`lint`/`plan` flood-fill using the real per-tick movement
  functions (`--cell` sets its resolution).
- **leak** — a gap in the perimeter through which the player can walk off the map (lint code).

## Lint vocabulary

`overlap` · `floating` / `sunk` · `headroom` · `stairs-top/bottom/narrow/steep` · `drop` · `leak` ·
`zone` · `floor` · `unreachable` · `door-blocked` · `door` · `spawn` · `light` · `z-fight` ·
`duplicate-id`. Meanings and fixes: `red_engine2 describe lint`. **error** fails `lint` (exit 1);
**warning** only fails with `--strict`.

## Rendering

- **`Globals`** — the uniform block (camera, lights, sun, fog) copied into `scene.wgsl`, `shadow.wgsl`
  **and** `background.wgsl`; all three must stay in lockstep with `gpu.rs`.
- **viewmodel** — the first-person held item, drawn in its own pass so it never clips into walls.
- **offline renderer** — `render.rs`; headless, 2x-supersampled; used by `frame`/`tour`/`verify` views.
- **live renderer** — `viewer.rs`; 4x MSAA, frustum culling, shadows, post pass.
- **procedural** — meshes and sounds are built in code (no imported models/textures/samples; ADR 0008).

## The melee weapon (naming trap)

The player calls it the **tire iron**. Internally it began as a hooked pry-bar object named `crowbar`
and the current held/third-person model is a **wooden bat** (`build_held_parts` in `viewer.rs`). If
someone says "tire iron", "crowbar" or "bat" they mean the same gameplay item — don't rename code
without asking.

## Characters and the launch menu

- **Character** — who the player is: `Human` or `Rat` (`player::Character`). Chosen on the launch menu
  (`--as human|rat` / `RE2_CHARACTER` skips it). Each has a `BodySpec` (radius, eye height, speeds,
  third-person camera, whether it carries the bat) and a model.
- **Cheddar** — the rat: small (0.43 m + tail, ~0.17 m tall), brownish-grey, pink ears/paws/tail. His
  ordinary speed equals a human's sprint (6.5 m/s); no bat; collides as a 0.12 m circle. `type:"rat"`.
- **human** — the redesigned `humanoid`: T-shirt (`material.color`), skin, hair, jeans, shoes, face.
  Built by `characters::human_parts`; part 3 is the left forearm the third-person bat is welded to.
- **melee hit / hit shape** — a swing connects only if its ray meets real geometry (`hit::raycast_shapes`),
  never merely a bounding box; only a connecting swing thunks and flashes. ADR 0011.
- **launch menu** — `menu.rs` + `overlay.rs`: a small 3-D backdrop plus a CPU-painted 2-D overlay.

## Weapons

- **Weapon** — `weapons::Weapon`: `Bat` (primary) or `Revolver`; the mouse wheel cycles (human only).
- **revolver** — the silver six-shooter: hitscan, 0.42 s between shots, 80 m, infinite ammo (`Ammo::Infinite`,
  flip `REVOLVER_AMMO` to `Limited` to cap it). Model in `revolver.rs`. ADR 0013.
- **hitscan / muzzle flash** — a shot is an instant ray from the eye; the flash is an emissive held part shown
  for 0.06 s (`HeldPart::muzzle_flash`).

## Loose props and physics

- **loose prop** — a top-level `prop` or floor-mounted prefab a person could lift and that is not a
  fixture; decided by `physics::classify`, overridable with `"movable": true|false`. Everything else
  is a fixed collider. Rats can *carry* only the smaller subset (`RAT_CARRY`) but can shove any of them.
- **dormant / dynamic** — a loose prop is *dormant* (a fixed rapier body exactly where authored) until
  disturbed by the player, a bat, a moving prop or being picked up; then it is *dynamic* (and drags
  along whatever rests on it), and rapier puts it back to sleep at rest. ADR 0012.
- **PropWorld** — `physics.rs`: the rapier world (fixed map colliders, loose-prop bodies, the player as
  a kinematic cylinder). Headless, no GPU types.
- **carry / hold pose** — a carried prop's body is disabled and the object follows the player
  (`PropWorld::hold_pose`, upright, in front, pulled in by walls); E drops it with the player's momentum.

## Prop-hunt game terms (not built yet)

- **hider / prop** — a player disguised as a map prop. **seeker** — the player who finds and strikes
  them. (Game-mode logic — rounds, disguises, scoring — does not exist yet.) Design intent: the
  seeker's primary action on objects is **hitting them with the bat** (built); the hider will use
  right-click to pick an object, then **R** to replicate it (not built). (**E** now picks up and
  drops loose props — see "Loose props" below.)
- **match server / orchestrator / lockstep** — the planned multiplayer architecture. **Not built.**
  Status and open questions: ADR 0010.
