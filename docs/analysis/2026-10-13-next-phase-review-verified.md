# The outside review of b5b458f, verified against HEAD (2026-10-13)

Every claim of the review was re-checked at `4c45d86` (main after #115) before the plan in ADR 2026-10-13-the-next-phase was written. Verdicts: **holds**,
**corrected** (true in substance, a number or a detail differs), **fixed** (no longer true; dropped from the plan).

## Method

Line counts are `wc -l` over the listed files. Hosted CI numbers are the job and log timestamps of PR #115's run 38371353231 (all 16 jobs green), read with
`gh api .../actions/runs/38371353231/jobs` and each job's log (`Finished \`test\` profile ... in`, nextest `Summary`). Local numbers are from the 4-core
development box during the #115 work (same machine, other agents' builds possibly running: read them as orders of magnitude). Routing was re-measured with
`python3 scripts/launchpad.py start "<prompt>"`, which builds nothing. The owner's ground truth is the feedback notes, `docs/analysis/idea-forge/` (read through
`python3 scripts/idea_forge.py backlog`) and STATUS.md.

## A. Game code in the engine

| claim | verdict | evidence at HEAD |
|---|---|---|
| ~30K lines (~24%) are showcase-game systems | holds | `src/` is 125,019 lines (`src/` + `crates/` 135,677). Shooter 5,832 (arsenal, firearms, killcam, shooter_world, objective_world, sim/{kit,ordnance,shooter,objective,objective_run}) + weapons.rs 776; museum 5,960 + client mode 889 + `museum` command 243; procgen 3,480 + nature/terrain/daycycle/expanse 1,938; karts 2,362 (sim/kart, kart_camera, kart_sound, track, sim/race); music/listen/ambience/soundscape 2,423 |
| Killchain's client moved out (S6); the museum moved a game in "because only re2/re2d ship" | holds | `games/killchain` (ea45010); ADR 2026-10-11-an-endless-museum-...: "The release pipeline packages only `re2`/`re2d` plus data" |
| `src/museum/state.rs:979` include_str!s `examples/frameless/collection.json` | corrected | true, and also `build.rs:1023`, but both are inside `#[cfg(test)]`: the engine's tests depend on a game's data; the runtime does not |
| Killchain-shaped structs in the wire protocol | holds | `src/net/protocol.rs`: `OwnKit` 500, `ArenaSnap` 613, `FlagSnap` 634, `ObjSnap` 687 (and `KartSnap` 382); protocol v15 |
| games/killchain has no maps/ | holds | `games/killchain/{Cargo.toml,examples,src}`; `main.rs` defaults to `maps/main.json` |

## B. The ceiling of the rules language

| claim | verdict | evidence at HEAD |
|---|---|---|
| topdown_switch: 165 lines of Rust + 5 rules | holds | `src/lib.rs` 152 + `main.rs` 13 (+ 164 lines of test); `topdown_switch.json` 5 rules |
| Gentle Carousel: 252 rules / 102 KB | holds | 252 rules, 102,527 bytes, 30 vars |
| Honeybound: 750 KB of JSON from 2,835 lines of Python | holds | `honeybound.game2d.json` 750,821 bytes; `examples/2d/honeybound/*.py` 2,835 lines (plus `solutions.json` 109,921 bytes) |
| RULES_LANGUAGE_GAPS designed, not built | corrected | #115 built the prop-subject piece of gap 1 (`self`, `prop` lists) and aimed fields; per-entity variables, restartable timers, the fall-out height (`player_below` does not exist) and movement rules are not built |
| MAX_RULE_VARS = 16, server silently `.take(16)`, no lint | holds | `protocol.rs:54`; `net/server.rs:1163`; no check anywhere names the cap |
| a new body, vehicle or RTS needs engine Rust | holds | `player::Character` is a fixed enum of six bodies (`player.rs:186`); karts are `assets/karts.json` + `sim/kart.rs` (1,191 lines) |
| (not in the review) strings | corrected | 2D has string tables, `pick`, `wrap` and a dialogue widget since #99 (ADR 2026-10-10-story-is-rules-and-tables-not-a-story-system); 3D has none. `random`/`floor`/`min`/`max` exist in both; `describe rules` lists them since #115 |

## C. The monolith and the compiled-in docs

| claim | verdict | evidence at HEAD |
|---|---|---|
| `search.rs` include_str!s SPEC/AGENTS/REFERENCE/PLAY_2D/GLOSSARY/ADR index; build.rs embeds every ADR and analysis note | holds | `src/tools/search.rs:14-21`; `build.rs` `FOLDERS` = `docs/adr`, `docs/analysis` with `rerun-if-changed` |
| a doc edit recompiles the library | holds | this PR (an ADR, this note, STATUS.md, `fixes.json`: no Rust): `red_engine2` rebuilt in 157 s before `preflight` could run, then `scripts/dev affected` spent 629 s (clippy 46, unit 171, suites 402 s) |
| warm `iterate` is ~105 s, AGENTS.md says "seconds" | holds | AGENTS.md still says "(seconds)"; during #115: a `rules.rs` edit took fmt 2 + check 10 + unit 11 + red2d 24 s (~50 s); an edit that touched SPEC.md and `describe.rs` took check 68 + unit 169 + suites 146 s |
| red2d shares source files with the main crate through `#[path]` | corrected | 10 files, not 9 (`lib.rs`: audio_analysis, audio_fx, dsp, fields, rules_expr, score, suggest, synth, voice_spec; `font.rs`: font_glyphs); 58 inline tests compile in both crates; `dsp::Voice` is two distinct types |
| shards each recompile every test binary, ~150-180 s each, 20+ job-minutes | corrected (worse) | #115: the five Linux shards compiled for 229-258 s each and ran tests for 48-96 s; the three headless shards compiled for 126-166 s and ran 100-114 s. ~27 job-minutes of compilation per PR, ~21 of them duplicate. The whole PR: 59.0 job-minutes |
| headless runs the full suite a second time; rust-cache saves on main only; Docker uncached on PRs | holds | `ci.yml`: headless-linux 3 shards on every PR (1,563 tests vs 1,683); `save-if: github.ref == 'refs/heads/main'`; `docker build` with no cache, 252 s on #115 |
| the CI note names the test archive as next | holds | `docs/analysis/2026-10-10-ci-wall-clock.md`, "Next": `cargo nextest archive` |
| (found here) the server tree reaches outside a would-be core | new | `net/host.rs:202` and `bin/red_server.rs:349` call `tools::portmap`; `net/session.rs:379` builds a `ui::race::RaceHud`: the first edges a `red_core` must cut |

## D. Uniqueness is capped by vocabulary

| claim | verdict | evidence at HEAD |
|---|---|---|
| six primitives, a five-field material, no meshes/textures (ADR 0008), 39 props as a Rust enum | holds | `schema.rs` `PRIM_TYPES` (box sphere cylinder cone capsule plane); `Material` = color metallic roughness emissive opacity (`schema.rs:188`); `props.rs:21` `PropKind`, 39 variants |
| 1,143 hard-coded hex colours in prefabs | corrected | 1,010 literal `"#rrggbb"` in the ten prefab libraries `assets/*.json` (1,300 with `karts.json`) |
| own prefab ~20x a library prop; style-own lint, own_prefab recipe, shape ops not built | holds | `docs/analysis/agent-loop-trace.md` (1 call / 558 B vs 6 calls / 12.7 KB); no `style-own`, `own_prefab`, authoring `lathe`/`extrude` (a `lathe` exists only inside `viewer.rs` for the bat and `museum/shape.rs`) |
| scaffolds and `catalog` lead with "Reuse" | holds | `tools/newgame.rs:92` "Follow Reuse -> Modify -> Generate -> Import"; `tools/catalog.rs:396` banner |

## E. Paperwork

| claim | verdict | evidence at HEAD |
|---|---|---|
| AGENTS.md 16,368 B, no budget, SPEC.md contradiction | holds | 16,368 bytes; line 6 "read SPEC.md for the scene language", line 27 "Do not open SPEC.md ... whole"; its largest section ("Spend context and time", 5.5 KB) is engine-maintainer material |
| 158 ADRs, 87 in Oct 1-12 | holds | 159 now (+ #115's); 87 dated 2026-10-01..12 |
| 64 ADRs owned by no feature | corrected | every ADR is owned (the docs feature owns `docs/adr/`); 69 are listed as rationale (`docs`) by no feature |
| 26 files over 1,000 lines; red2d game.rs 3,493 with a ~688-line parse and no tests; sim.rs 2,566 | corrected | 27 in `src/` + `crates/` at b5b458f, 28 now: #115 pushed `src/sim/rules_run.rs` from 989 to 1,388 (mostly tests) and `rules.rs` from 1,441 to 1,870. game.rs: `parse` is lines 2710-3390 (681 lines), 0 `#[test]`; sim.rs 2,566 lines, 25 tests |
| stale claims | holds | ENGINE_OVERVIEW.md:250-256 (no audio samples; no jumping between floors; no slopes but stairs: symphonia playlists, `jump_speed`, jump pads and terrain exist); `tools/symbols.rs:2` "12k lines"; `thiserror` in Cargo.toml with no use; `src/macros.rs` holds scene macro objects |

## F. Cold start

| claim | verdict | evidence at HEAD |
|---|---|---|
| describe/search/context need the binary; a cold build is minutes | holds | a seeded worktree built `red_engine2` in 185 s on 2026-10-12; Idea Forge's #1 backlog key is `cold-worktree-build-and-preflight-rebuild` (recurring, 12 runs, 100 min) |
| prebuilt only on an exact fingerprint; the owner's question is open | corrected | `gh release list` is empty: no release exists, so `prebuilt.py` has never matched. The owner chose "publish on every main commit that changes a build input" (option A) on 2026-10-10; the `release.yml` change was blocked by the permission check, and STATUS.md still says "waiting" |
| `start` misroutes plain game prompts | **fixed** | "Make a game where the player herds fireflies into jars before dawn", "Build a small racing game with a jumping frog", "Make a puzzle game where you swap places with your shadow": all `game-create (high)`; the first lists engine-change only as the alternative |
