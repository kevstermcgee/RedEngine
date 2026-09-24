# Architecture Decision Records

Short records of *why* Red Engine 2 is built the way it is, so nobody re-derives intent from code.
`red_engine2 search <topic>` indexes them (kind `adr`). Read the one-line summaries first.

| # | Decision | Status |
|---|---|---|
| [0001](0001-json-scenes-and-self-describing-cli.md) | Maps are JSON; the engine describes itself so AIs never read Rust | accepted |
| [0002](0002-props-in-rust-prefabs-in-json.md) | Props (custom collision) are Rust; everything else is JSON prefabs | accepted |
| [0003](0003-one-physics-shared-by-game-and-tools.md) | The game and the analysis tools run the same physics functions | accepted |
| [0004](0004-reachable-ground-height-for-floors.md) | Floors work by "highest reachable surface", not floor bookkeeping | accepted |
| [0005](0005-fixed-step-physics-with-camera-interpolation.md) | Fixed 1/60 s player physics, interpolated camera | accepted |
| [0006](0006-parse-time-expansion-for-sugar.md) | `wall`/`fence`/`prefab` expand to plain objects at parse time | accepted |
| [0007](0007-docs-cannot-drift.md) | Docs are generated or test-checked; source index is on demand | accepted |
| [0008](0008-procedural-assets-only.md) | Meshes and sounds are generated in code, nothing imported | accepted |
| [0009](0009-four-maps-one-asset-library.md) | Four maps (house, school, office, store) share one library | accepted |
| [0010](0010-headless-match-server.md) | Headless multiplayer match server (**not built**) | proposed |
| [0011](0011-two-characters-and-exact-melee-hits.md) | Human + Cheddar the rat, a launch menu, exact melee hit-testing | accepted |
| [0012](0012-rapier-prop-physics-dormant-until-disturbed.md) | Loose props run on rapier and stay dormant until disturbed | accepted |
| [0013](0013-revolver-second-weapon-hitscan-infinite-ammo.md) | Silver revolver: mouse-wheel second weapon, hitscan, infinite ammo for now | accepted |
| [0014](0014-sim-foundations-fixed-tick-scratch-change-tracking-static-props.md) | Sim foundations: 60 Hz tick + tick-based weapons, scratch buffers, change tracking, static props | accepted |
| [0015](0015-engine-first-refocus-test-lab-and-legacy-maps.md) | Engine-first refocus: the Red Test Lab is the dev map, the four maps are legacy, roadmap | accepted |
| [0016](0016-authoritative-udp-multiplayer.md) | Authoritative UDP multiplayer: server, delta snapshots, prediction, interpolation, reconnect | accepted |

## Writing one

Copy the shape below, take the next number, add a row above (a test fails if a file is missing from
the table or from the search index). Keep it under ~60 lines; link code by symbol name
(`red_engine2 src show <symbol>`), not line number.

```
# NNNN. Title
Status: accepted | proposed | superseded by NNNN
## Context      what forced a decision
## Decision     what we do
## Consequences what gets easier / harder; how to undo it
```
