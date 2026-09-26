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
| [0010](0010-headless-match-server.md) | Headless multiplayer match server (historical proposal; built as 0016/0017) | superseded by 0016, 0017 |
| [0011](0011-two-characters-and-exact-melee-hits.md) | Human + Cheddar the rat, a launch menu, exact melee hit-testing | accepted |
| [0012](0012-rapier-prop-physics-dormant-until-disturbed.md) | Loose props run on rapier and stay dormant until disturbed | accepted |
| [0013](0013-revolver-second-weapon-hitscan-infinite-ammo.md) | Silver revolver: mouse-wheel second weapon, hitscan, infinite ammo for now | accepted |
| [0014](0014-sim-foundations-fixed-tick-scratch-change-tracking-static-props.md) | Sim foundations: 60 Hz tick + tick-based weapons, scratch buffers, change tracking, static props | accepted |
| [0015](0015-engine-first-refocus-test-lab-and-legacy-maps.md) | Engine-first refocus: the Red Test Lab is the dev map, the four maps are legacy, roadmap | accepted |
| [0016](0016-authoritative-udp-multiplayer.md) | Authoritative UDP multiplayer: server, delta snapshots, prediction, interpolation, reconnect | accepted |
| [0017](0017-headless-build-gfx-feature.md) | The dedicated server builds without graphics: the `gfx` feature, `collide`/`geometry` extraction, CI-enforced | accepted |
| [0018](0018-self-describing-engine-pattern.md) | The self-describing engine pattern (describe/search/catalog/verify): a reusable design | accepted |
| [0019](0019-strict-fields-versioning-and-json-envelope.md) | Unknown scene fields are errors, a schema version, and one `--json` envelope for every command | accepted |
| [0020](0020-game-rules-as-data-and-headless-scenarios.md) | Game rules as data (`vars`/`rules`), proven by headless scenarios (`sim`, `checks.sim`) | accepted |
| [0021](0021-deterministic-traces-replay-and-checksums.md) | Deterministic traces, replay, and split checksums: the first divergent tick | accepted |
| [0022](0022-authoritative-interactions-per-prop-acks-and-interest.md) | Authoritative pick-up/combat, per-prop acknowledgement, rooms-and-portals interest management | accepted |
| [0023](0023-walk-failures-name-their-blocker-routes-are-planned.md) | Walk failures name their blocker (id, gap, passage width); routes are planned (`walk --auto`), not guessed | accepted |
| [0024](0024-the-framework-layer-blueprints-and-game-projects.md) | The framework layer: blueprints compile to complete self-checking maps; games are projects that pin the engine, not forks | accepted |
| [0025](0025-handoff-status-and-derived-doc-facts.md) | Handoff (`status`, STATUS.md) and doc facts derived from the repo, checked by a test | accepted |
| [0026](0026-a-headless-audited-ui-kit.md) | A headless, audited UI kit: one layout drives painting, clicks and the audit; `ui-shot`, `ui-check` | accepted |
| [0027](0027-runs-anywhere-doctor-env-config-container-lf.md) | Runs anywhere: `doctor`, env-var server config, container image, software GPU fallback, LF | accepted |
| [0028](0028-authenticated-datagrams-and-join-keys.md) | Authenticated datagrams and join keys: challenge/response, HMAC tags (protocol v3) | accepted |
| [0029](0029-match-flow-lobby-rounds-rematch.md) | The match flow: lobby, ready-up, countdown, rounds, results, rematch; the online screens | accepted |
| [0030](0030-performance-is-a-contract.md) | Performance is a contract: `perf` and `checks.perf` | accepted |
| [0031](0031-home-hosting-with-upnp.md) | Home hosting with UPnP: `portmap` and `red_server --upnp` | accepted |
| [0032](0032-provable-releases.md) | Provable releases: `package` and `package --verify` | accepted |
| [0033](0033-feature-index-and-impact.md) | A feature index that cannot rot, and `impact` | accepted |
| [0034](0034-net-test-and-bad-network-resilience.md) | `net-test` and bad-network resilience (bursty loss, bounded extrapolation) | accepted |
| [0035](0035-rule-state-in-the-standard-client.md) | Shared rule state drives standard-client visibility, effects and a generic HUD | accepted |
| [0036](0036-repeated-bounded-online-rule-state.md) | Repeated bounded rule-state snapshots give online clients loss/reconnect/late-join recovery | accepted |

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
