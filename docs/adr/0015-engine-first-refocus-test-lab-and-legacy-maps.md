# 0015. Engine-first refocus: the Red Test Lab, legacy maps, and the roadmap
Status: accepted

## Context
Red Engine 2 grew map-first: four polished maps (house, school, office, store) before there was a headless
authoritative simulation, any networking, or CI. The goal is a high-quality, AI-friendly multiplayer engine
that better maps and games can be built on later, so engine quality, architecture, testing and
extensibility now come first; cosmetic map work is paused.

## Decision
**1. Legacy maps.** `house`, `school`, `office`, `store` (and `prop_hunt_yard`, the recipes) are
legacy/reference content. Their assets, prefabs and data are preserved for reuse and we do not break them
on purpose (`tests/maps_verify.rs`, `house_walk.rs`, `prop_physics.rs` still run them). But they never
block a better architecture: if an engine change needs a schema or system change that breaks a legacy map,
make the change, then either migrate the map or mark its test `#[ignore = "legacy: <why> (ADR 0015)"]`
and record it under *Known incompatibilities* below. No new polish on them.

**2. One primary development map: `examples/test_lab.json`.** Deliberately basic (flat colours, a sun and
a lamp per room), one room per system under test. `tests/test_lab.rs` turns it into regression tests and
its own `checks` block (lint, reach, real-physics walks) runs under `verify`. A new engine feature gets a
Test Lab room and a test before any real map uses it.

| room | tests |
|---|---|
| `spawn_hall` | four multiplayer spawn points, open movement |
| `clearance` | a wall with 0.25 / 0.6 / 0.75 / 0.9 / 1.5 m gaps: who fits (human r 0.35, rat r 0.12) |
| `vertical` | stairs to a 2 m platform, a shallow ramp-like stair, 0.3 / 0.6 / 1.0 m ledges, unrailed drops |
| `props` | crate pyramid + 5-crate tower (stacks), domino barrels, pinned (static) crates, table items, heavy vs carriable furniture |
| `range` | hitscan lane with targets at 3-14 m, low cover, crate cover, a target behind a wall |
| `characters` | human and rat reference models, a 0.45 m crawl tunnel, climb-height blocks |
| doors between rooms | one 1.0 m portal each (`portals` in the JSON) |

**3. Roadmap** (the user's priority order) and where each stands:

| # | item | status |
|---|---|---|
| 1 | CI green | **done**: `.github/workflows/ci.yml` (ubuntu + windows), `scripts/ci.sh` runs the same steps locally (clippy `-D warnings`, `cargo test`, benches compile). Only Windows was run; the Linux job is unverified. |
| 2 | Strong rendering-free simulation | **started** (ADR 0014): `src/sim/` = tick clock, tick-based weapons, scratch buffers, change tracking, entities, static/dynamic props, snapshot encoding. **Not yet**: player movement, hit resolution and the weapon state still live in `App` in `re2.rs`; the headless `Sim` that owns them is the next extraction. |
| 3 | Authoritative headless prop physics + interactions | props are headless (`PropWorld`); pick-up/drop/strike are still *driven* from `App` |
| 4 | Real low-latency transport | not started |
| 5 | Server + two clients in the Test Lab | not started |
| 6 | Deterministic replay + state checksums | not started; blockers listed in ADR 0014 (live input in the tick, libm trig, cascade order) |
| 7 | Rooms/portals/spatial data in the map format | Test Lab carries a **draft** (`spawns`, `portals`, `interest`); the engine ignores it today, a test keeps it consistent; parser not written |
| 8 | Performance/network instrumentation and budgets | `benches/` + `check.py`, `tests/alloc_budget.rs`; no network metrics (no network) |
| 9 | AI-friendly | keep `describe`/`search`/`src` current; every module has `//!`; each decision an ADR |

**4. Engine gaps the Test Lab exposed** (none fixed yet): no true ramps (only flat box tops and stairs);
no per-scene spawn points (only the camera); rooms exist only as `zones` rectangles, with no connectivity;
`lint`/`reach` know one body size, so they cannot say "rat-only" (the crawl tunnel and the 0.25 m gap need
`lint_ignore`); a prop hit by a ray in the same tick it is promoted is invisible until the next `step`
(the query BVH refreshes on step).

## Known incompatibilities
None yet.

## Consequences
- Engine work is judged by CI + the Test Lab + benches, not by how a legacy map looks.
- Roadmap items 4-6 need the headless `Sim` extraction first (ADR 0010 step 1), so that is the next slice.
