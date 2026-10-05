# Reliability milestone: making the pipeline trustworthy for a fresh, smaller model (2026-10-05)

Base: `main` at `a782b1a`; work on branch `reliability`. Contract under test: **AI intent -> scene/configuration -> engine behaviour -> analysis -> verification -> packaged game.**
The standard: an AI must not be able to write something plausible, get a green result, and conclude behaviour was proven when the engine ignored or misunderstood it.

## What was found (and what it turned out to be)

| # | Finding | Status at `a782b1a` | Now |
|---|---|---|---|
| 1 | `checks.audio` validated only its top-level keys: `peek_max`, `seam`, a misspelled `calls` all ignored | confirmed | typed fields with exact paths, alternatives, a likely fix |
| 2 | Wrong types read as defaults (`"rising": "false"` meant `true`, `"reachable": "false"` meant `true`, `"max_warnings": "5"` meant no limit); `secs: 99999` clamped silently | confirmed | each is an error saying what was expected and what was found |
| 3 | A `soundscape` item with no `calls`/`beds`, `calls: {}`, `reach: []`, `objects: {}`, a scenario with no `expect`: green and vacuous | confirmed | each is an error that says it asserts nothing |
| 4 | **The flagship example failed its own `verify`**: `checks.audio` ran the ambience without applying `audio.birds: false` (190 calls nobody hears); no test ran it | found while auditing | the check hears what the player hears; a test runs Marcel's soundscape checks |
| 5 | One-shot "cut off" reused the loop `seam` (last frame -> first): a cosine of whole cycles stops dead at full level and passed; a hard-attack decay to -45 dBFS was a false alarm | confirmed | separate `end_step` (the click of a hard stop), `seam` loop-only, one named limit |
| 6 | `reach`, `walk --auto`, `lint`, diagnostics ignored generated trees/shrubs that block the player; `lint` reported a perimeter "leak" on Marcel and the endless example; `plan` drew one floor height | confirmed | one shared blocker query; windows, not bounds; no leak in an endless world |
| 7 | Loose props rested on a flat `y = 0` slab whatever the ground: a prop in a hollow was hidden under it, a drop on a hill sank. **Gameplay bug**, not just tooling | found by the authoring exercise | props rest on a heightfield of the real ground (terrain or procgen) |
| 8 | Scripted `interact` stopped at a 2-D distance; on a slope the pickup reach (3-D, from the eye) was out of range | found by the exercise | walks until the eye-to-prop distance is within 80% of the reach |
| 9 | `headroom` lint called a lantern's roof a ceiling over higher ground beside it | found by the exercise | loose props are not ceilings |
| 10 | Online: a scene that declares an end card shows no outcome at all (the banner was suppressed for a card that online never shows) | confirmed (reproduced) | the banner stands in for any card that is not shown |
| 11 | Split-screen "props are player 1's" lived only in an ADR | confirmed | one list feeds `describe`, the client's start-up note and the scripted-`interact` failure; a test fails if guests ever carry props |
| 12 | A shader-only edit ran `shader_validation` but not the `gpu`/`fx`/`ocean_pass` tests that guard the shader text (so `affected --quick` could be green and CI red); the checked compositions were a hand list | confirmed | those tests are owned by the shader feature; compositions are derived from `src/` and compared; a permanent fixture of the Direct3D failure shape |
| 13 | Render trend pending (PR #36); no way to run the same scenes on a real GPU; `compare` could set milliseconds against another adapter | confirmed | integrated; native `render-trend`; `compare` pairs by adapter |
| 14 | `validate` said OK for a scene naming a score file that does not exist; a manifest `files` list could omit it | found by the exercise | load-time existence and parse check; `playable_resource_gaps` test over every published playable |
| 15 | `doctor` labelled llvmpipe "[hardware]" and said nothing about an unopenable GPU (this machine has one) | found by the exercise | labels what the adapter is; names the permission problem and its fix |
| 16 | My own `reach` schema hint said `[x, y, z]`; the height is last (`[x, z, y]`) | found by the exercise (an engine author's slip) | fixed and tested |

Already right at the base and kept: the HLSL translation test (verified against the pre-fix shader and a permanent fixture), `affected` not escalating a bare `pub mod`, every scene block rejecting nested typos and wrong types (15 of 15 probes on the game's blocks), main CI green on Windows at `a782b1a`.

## What was not confirmed or not done
- Per-player prop ownership in split-screen was **not** generalised: it needs a holder per local player in the physics world, the carried pose, the HUD prompt and the flashlight per player, and none of it is verifiable here without real gamepads. The limitation is now stated once, diagnosed early and pinned by a test.
- Whether the new checks *mean* something is the author's: a lint budget of 50 errors is well formed.

## Evidence: tests added (adversarial, each fails without its fix)
`check_schema` (23 mistakes), `audio_checks` (25 mistakes plus the `birds` behaviour and `seam_max` on a one-shot), `sim::scenario` (7), `audio_analysis` (8 fixtures: fade, truncation, seamless loop, bad seam, cosine start=end, -45 dBFS decay, silence, NaN/inf, constant level, stereo, 0-3 frames), `procgen_analysis` (8: reach, walk, planned route, lint, zone height, unreachable-why, shared query, shipped scenes), `physics` (3: rest on hollow and hill, ray through the prop's coordinates, flat floor unchanged), `client_headless` (online outcome with a declared card; split-screen guests), `shader_validation` (7), `publish_check` (5), `render_trend` (3), `lantern_walk` (9), `checks_wellformed` (2). Mutations done by hand: old one-shot rule restored (2 fixtures fail), generated trees dropped from the query (3 tests fail), flat prop floor restored (2 fail), a `concat!` changed in `gpu.rs` (coverage test fails naming both lists).

## The fresh-author exercise (Phase 7), and its limit
`examples/lantern_walk`: procgen terrain, dusk clock, generated score + nature ambience, rules with a carried prop, declared UI, hosted match, split-screen note. **Limit: I played the fresh author myself, using only `describe`, `search`, `recipe` and the engine's own messages; I was not a separate smaller model, and I know the engine.** What it found is findings 7-9 and 14-16 above, none of which a unit test of the parts had caught. The game verifies (11 checks: lint, four standing places, three scripted playthroughs, the score's loudness, the night's soundscape) and plays in a hosted match.

## Measurements (all on this machine; no real GPU was available)
- Render trend, `llvmpipe (LLVM 20.1.2, 256 bits) (Vulkan, Cpu)` [software rasteriser], 960x540: Marcel morning 372 ms, dusk 359, night 330, four players 755, house 557; triangles and draw calls identical to the baseline (897,067 / 745,177), milliseconds within +-2.3% (noise). **These say little about a player's GPU.**
- This machine has an Intel GPU it cannot open (`/dev/dri` owned by `video`/`render`, user in neither); `doctor` now says so. `red_engine2 render-trend --label "..." --out gpu.json` on a real GPU, then `benches/render_trend.py add`, is the way to get that number; it has not been run.
- Analysis cost: `lint` and `reach` on Marcel 0.3 s (now over 9,026 m^2 of real ground instead of a 259 m^2 sliver); `verify` of Lantern Walk 2.9 s including 450 s of scripted play.

## Known unsupported or unverified combinations
Loose props with a looping (`world.wrap`) axis; props thrown beyond 40 m of the props' footprint (they fall to a catch-floor); per-player props in split-screen; `ray`/`sight`/`nav` with generated trees; online play over the internet (loopback only here); anything with real gamepads; audio quality (only numbers and pictures); human playability.

## Where tools can still disagree with gameplay
`ray`/`sight` and the `nav` graph ignore generated trees; `plan --ascii` is one floor; analysis of an endless world covers a window (a target beyond it must be passed to `compute`); a vertical ray exactly on a heightfield grid line misses in parry (worked around by an off-grid offset, not fixed); the zone-height check trusts `y` while `prop_enter` uses the prop's origin.

## Next three
1. Per-player carried props in split-screen (holder per local player in `PropWorld`, per-player pose and prompt), then the limit disappears from `GUEST_LIMITS`.
2. Make `ray`, `sight` and `nav` use `MapWorld::blockers_in`, and give `plan --ascii` the endless treatment, closing the last tool/gameplay gaps above.
3. A real-GPU trend: run `render-trend` on a player-class machine and file it; then a non-gating CI job on a GPU runner if one exists. Also edit the four RedEngineGames projects whose empty or mistyped checks now fail (needs a re-release decision).
