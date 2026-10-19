# RedCraft: a Minecraft-like game on Red Engine 2 (plan, 2026-10-19)

The ask (owner, 2026-10-19): "Using the latest RedEngine, create a Minecraft clone called RedCraft." Nothing else was specified, so the bar is
the game people mean by the words: an endless world of blocks you mine and place, survival with crafting and a day and night, a creative mode
to build in, and a world that is still there tomorrow.

## What the engine already had, and what it lacked

Surveyed on main c78f6b7 (`describe`, `search`, the client and renderer seams):

| needed | engine on main | decision |
|---|---|---|
| a game's own world drawn by both renderers | `world_layer::WorldLayer` + `LayerDef` (Frameless's museum) | the voxel world is a layer of the game crate |
| a game's own first-person rules in the live client | `client::GameMode` + `ModeContext`, `client::run(factory)` | `CraftMode` |
| textures | none: objects have vertex colours, painted surfaces (`fn surface`) and no texture bindings | block textures are procedural pixel art in the game's WGSL `surface` |
| per-vertex data for a painted surface | `surface` sees point, normal, uniform numbers; the vertex colour only tints | **seam**: `SurfaceIn.vertex`; `material.z` 1 = the vertex colour is data, not a tint |
| see-through layer surfaces (water, glass, ice) | layers draw solid and glow (additive) only | **seam**: `WorldLayer::draw_blended` with the alpha pipeline |
| a body that walks on blocks | player capsule against `Collider2D` boxes, 0.35 m step-up, engine controls | the mode owns the body (`input_frozen`) with its own AABB physics; **seam**: `ModeContext::key_held`, `pad` (scripts and pads drive it too) |
| saving the world when the window closes | no hook | **seam**: `GameMode::closing` (winit `exiting`, end of a headless run) |
| time of day in a save, sleeping | the clock's `start` is fixed in the scene | **seam**: `ModeContext::time_of_day` / `set_time_of_day` |
| day and night, stars, sun shadows, haze | `clock` block, cascaded sun shadows, sky shader | used as is |
| music and nature sounds | `audio` block: scores, journey, ambience beds | used as is; block sounds synthesized in the crate |
| HUD and menus | CPU `ui::Canvas` (5x7 pixel font), overlays, pause ownership | hotbar, hearts, inventory and crafting screens painted by the mode |

The seams are general (a voxel or tile game, any game with a save) and small; the voxel world itself stays in `games/redcraft`
(ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or).

## The game

* **World**: 16 x 128 x 16 chunks streamed around the player by worker threads; seeded terrain with biomes (plains, forest, birch forest, taiga,
  desert, snowy plains, mountains, ocean and beaches), caves, ores by depth (coal, iron, gold, diamond), trees (oak, birch, spruce), cacti,
  flowers and grass, lakes and sea, a bedrock floor.
* **Light**: sky light and block light (torches, lava) flood-filled per block like the original and baked into the vertex data with ambient
  occlusion; the engine's sun, moon, shadows and haze on top.
* **Look**: 16 x 16 procedural pixel textures per block face, drawn in the game's `surface` WGSL; HUD icons from a CPU twin of the same code.
* **Play**: walk, sprint, sneak (no falling off edges), jump, swim; creative flight. Hold to mine (time by hardness and tool), place against the
  aimed face, drops into the inventory. Hotbar 1-9 and wheel, inventory (E) with a 2x2 grid, crafting table 3x3, furnace smelting, chests.
  Health, fall damage, drowning, lava; zombies at night and animals by day (later phase).
* **Persistence**: one save per world: seed, edited chunks, player, inventory, time of day; saved every 30 s, on pause and on close.

## Phases (each verified by pictures and tests before the next)

1. Engine seams + crate skeleton + world generation + meshing + layer: pictures of terrain from `redcraft shot`.
2. Mode: body physics, mining and placing, hotbar, HUD; headless scripts prove walking, mining, placing.
3. Inventory, crafting, smelting, chests, tools; survival health; saves.
4. Sounds, music, water flow, mobs; polish; feedback doc of engine gaps.
