# 2026-10-04. An endless world: procgen as pure functions of a seed
Status: accepted
Summary: The world is not stored: ground, climate, biomes and every plant are pure functions of (seed, position), built per 48 m chunk, so it is infinite, identical on every machine, and costs nothing until looked at.

## Context
Marcel (the first game made to test procedural generation) is a world you can walk through forever. The engine's ground is a stored heightfield of at most a million samples, a scene lists its objects, and one draw call is issued per object, so a forest of tens of thousands of plants cannot be objects in a scene, and an endless world cannot be data in a file.

## Decision
`procgen` (`src/procgen/`) is a library of pure functions, with no state and no I/O:
- **Noise** (`noise.rs`) is integer hashing plus `f64` `+ - * /` only (gradient noise from eight fixed directions, no `sin`/`cos`/`powf`, whose last bit differs between maths libraries), so a seed gives the same world on every machine and a server and its clients agree on every tree without exchanging any terrain. `f64` coordinates keep the ground smooth a hundred kilometres out (tested at 10^7 m).
- **World** (`world.rs`): `height(x, z)`, `normal`, `climate` (three slow fields: cool, wet, wood), `biome` (meadow, wildflower field, grove, forest, pinewood, glade), `ground_color`, and `plants(chunk, kind)` / `trunks(chunk)` for a 48 m `ChunkId`. Placement is a jittered grid per layer (trees 6 m, shrubs 4 m, flower patches 12 m, grass 1.5 m) aligned to chunks so a cell belongs to exactly one chunk and depends only on its own coordinates; flower patches spill across borders, so a chunk also reads the neighbours' patch cells and keeps the flowers that land in it (tested: no seam). Small plants keep out of tree trunks by reading the neighbouring chunks' trees. Every plant carries a `rank` (0..1): keeping only `rank < t` thins a layer evenly and nests, which is how distant chunks will be cheap.
- **Flora** (`flora.rs`) is a table of real species (6 trees, 2 shrubs, 10 flowers, 2 grasses, each with its Latin name, height, spread, colours and the climate bands it grows in). A species is picked by how well it suits the local climate, with per-species patchiness so birches gather in stands.
- **`red_engine2 procgen out.png --seed N`** draws a top-down map of any region (`--biomes` for the kind of country, `--grid` for chunk borders) and prints plant counts per species, country shares and ms per chunk, so a world can be looked at and measured with no renderer or GPU.
- `the_world_is_pinned` fingerprints heights and placements for a seed: it fails when the generator changes, because changing it changes every world and every save made in one. Update it on purpose and say so here.
- Not yet wired into scenes: the `procgen` scene block arrives with the streaming renderer (phase 4), so no scene key exists that does nothing.

## Consequences
Easier: an infinite world with no storage, no network sync of terrain, and checks (map, counts, determinism, border continuity, walkable slopes, species coverage, chunk build time of about 2 ms in a debug build) that need no GPU. Harder: the ground can only be queried, so collision and the renderer must call `World::height` instead of reading a stored heightfield (phase 4 does); changing a threshold reshapes the world, so tuning belongs before a game ships, and the pinned fingerprint is the guard. Undo: delete `src/procgen/` and `src/tools/procgen_map.rs`; nothing else depends on them yet.
