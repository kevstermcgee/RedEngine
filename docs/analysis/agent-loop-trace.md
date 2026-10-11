# Agent loop trace (2026-10-10): what building on the engine costs an agent

Phase 1 of the agent-loop efficiency work: three representative tasks run strictly per AGENTS.md, from `scripts/dev start`, on a fresh worktree of `main`
(af7175f). No engine code was changed (task c's lint was written, measured and reverted; the patch is kept outside the repo). Every command went through a
logger: wall seconds, CPU seconds (user + sys), exit status and the bytes it printed; the raw log is [`agent-loop-trace.log.tsv`](agent-loop-trace.log.tsv)
(`R` rows are files read by hand, with the reason). Tokens are estimated as bytes / 4.

**Caveats.** (1) One agent (the author of this note, who has worked on this repository before) played the "fresh agent": the trace records where the engine
led and where prior knowledge had to fill in, but a truly cold agent would read more, not less. (2) From 19:52 to 20:22 a second checkout ran the full CI
suite on the same 4-core box, so wall times of the heavy tiers in that window are inflated up to ~2x; **CPU seconds are the comparable number** (lesson from
the 2026-10-06 efficiency pass). (3) n = 1 per task.

## Totals

| | engine commands | bytes printed (~tokens) | of which describe/search/context/recipe/src | files read by hand | wall | CPU |
|---|---|---|---|---|---|---|
| session start (every task) | AGENTS.md + CLAUDE.md | 17,795 (~4,450) | | | | |
| a. 3D cabin | 24 | 38,106 (~9,500) + 16,289 read | 32,990 in 11 calls | 2 recipe copies (content) | 12.3 min (10.3 of it the first build) | 433 s |
| b. 2D collect-and-dodge | 13 | 17,138 (~4,300) + 5,084 read | 8,962 in 3 calls | the starter (content) | 2 min of tool time ≈ 1 s | 1 s |
| c. new lint code | 21 | 43,087 (~10,800) + 3,800 read | 35,962 in 13 calls | lint.rs tests, describe.rs table, 2 greps | 21.7 min | 1,790 s |

Turns to green: **a** lint clean on the 2nd lint (1 fix), verify green on the 1st verify; **b** validate clean on the 2nd, verify green on the 2nd (one
expectation wrong); **c** `iterate` green first time, `affected` green first time **but that green was false** (below); really green after a doc fix and a
flaky-test rerun (3 `affected` runs).

## Trace a: new 3D scene (two-room cabin, locked door + key, style block, dialogue on pickup)

| # | command | bytes | wall / CPU s | what happened |
|---|---|---|---|---|
| 0 | read AGENTS.md + CLAUDE.md | 17,795 | | AGENTS.md line 6 says "read SPEC.md for the scene language"; CLAUDE.md and AGENTS.md §"Spend context" say never open it whole |
| 2 | `start "New 3D scene: ..."` | 1,510 | 1.1 / 0.4 | **routed `game-change`** ("room" cue); NEXT = `scripts/dev red check`; executable missing, no prebuilt; hint: `seed --from <a built sibling's target dir>` |
| 3-5 | `seed --from ../RedEngine/target` → "no donor"; `ls -d ../*/target`; `seed --from ../RedEngine-story/target` | 598 | 108.8 / 4.0 | the hint names no candidate; the agent had to find one (2.29 GB copied in 107 s) |
| 6 | `scripts/dev red check` (NEXT) | 429 | **618 / 426** | built the CLI (551 s, seeded deps) and then **`error: unrecognized subcommand 'check'`**: `check` is a game-project command (`scripts/red check`), not one in the engine checkout |
| 7 | `describe --brief` | 1,987 | 0.03 | good |
| 8 | `search "locked door opened by a key"` | 5,482 | 0.4 | #1 the **2D** `key-door` mechanic; then `wall` docs, an Idea Forge ADR ("keyed backlog"), blueprint docs, a Rust const, `two_floor_house`, ADR 0028 (join keys); the right answer, recipe `gated_garden`, is **9th (last)** |
| 9-10 | `recipe`, `recipe gated_garden` | 2,681 | | good: the recipe teaches phases for exactly this |
| 11 | `search "show dialogue when the player picks up an item in a 3D scene"` | 6,794 | 0.5 | nothing says "a 3D scene has no dialogue widget"; the story ADR fragment says `dialogue` widget **(2D)** in passing. **Guessed** that 3D has none and used `ui.objective` lines as the stand-in |
| 12-14 | `describe ui`, `describe style`, `describe rules` | 11,043 | | complete and accurate; `describe rules` has no story-primitive line (tables/pick are 2D) |
| 15-16, R | `recipe ... --new` ×2, read both copies | 16,289 read | | editing content means reading it: legitimate |
| 17 | `lint` | 368 | 0.05 | **ERROR `door-blocked`** on the locked door's opening: "furniture right in front of it, or it opens onto a wall/void". The cause is the closed door itself (the kitchen is sealed until the phase `door_open`); `lint` is phase-aware for `unreachable`/`zone`/`floor` but not for `door-blocked`, and the message names a wrong fix |
| 18 | `search "door-blocked locked door phase"` | 4,765 | 0.3 | no fragment covers it. Worked around by copying `gated_garden`'s shape (two wall pieces and a gap holding a door box) |
| 19-21 | `lint` (clean), `plan --phase door_open`, `verify` | 1,256 | 0.6 / 0.9 | 11 checks green first time |
| — | looked at `golden/cabin/living.png` | | | the recipe's `living` view camera sits inside the floor lamp's shade; `verify` recorded it as the golden without complaint. Moved the eye |
| 22-25 | `ui-check`, `style-report`, `verify`, `ui-shot` | 955 | | style-report: 8 of 9 roles non-default, **0 own prefabs, 45% shared catalogue, 48% of surfaces still default colours**; lint silent on that |

## Trace b: new 2D game via `new-game --kind 2d` (90 s collect-and-dodge, own palette, two original props)

| # | command | bytes | what happened |
|---|---|---|---|
| 26 | `start "New 2D game: ..."` | 1,115 | **unrouted** ("no workflow cue") although the task says "New 2D game"; asks to rerun with `--workflow` |
| 27 | `describe 2d` | 8,058 | one page, everything needed; the best packet of the three traces |
| 28 | `start ... --workflow game-create` | 2,068 | good plan; proposes the directory name `second-collect-dodge` (from "90-second") |
| 29 | `new-game ... --engine-path ../../RedEngine-agentloop` | 435 | the agent got the relative path wrong (it is relative to the new project); `new-game` **accepted a path that does not exist** |
| 30 | `scripts/red recipe spawner-waves` | 84 | `red: cloning the engine () into .red/engine ... fatal: repository '' does not exist`: names neither the bad path nor the fix |
| 31 | (path fixed) `recipe spawner-waves` | 820 | lists rule names, not their JSON; wrote the game from `describe 2d` instead |
| 32 | `validate` | 109 | **guessed** `{"ended": "any"}`; the error lists `win`/`lose` (fine) |
| 33 | `validate` + `sim --every 15` | 1,886 | balance visible at once (dead in 15 s); each `validate`/`verify` prints two ~350-byte `install ... unverified` warnings |
| — | 6 × `sim --only` over seeds | | found a seed where the edge strategy wins; quick (≈0.1 s each) |
| 34-36 | `verify` (1 fail: my expectation), `frame`, `verify` | 2,367 | green on the 2nd verify. The frame showed the hazards nearly invisible (dark on dark): `verify`'s render check ("not blank") cannot see contrast; fixed by eye |
| 37 | `style-report` | 167 | "a 2D game draws everything itself: the shared-catalogue questions do not apply": compliance is automatic in 2D |

## Trace c: engine change (a new lint code `ui-no-rules`: a scene with a `ui` but no `rules`)

| # | command | bytes | wall / CPU s | what happened |
|---|---|---|---|---|
| 39 | `start "Engine change: add a lint code ..."` | 1,927 | 0.1 | owners by word match: **scene_format, idea_forge, karts**; lint lives in `map_analysis`. NEXT = `context scene_format` |
| 40 | `context scene_format` (NEXT) | 11,971 | 0.2 | the wrong feature: ~3,000 tokens with nothing about lint |
| 41 | `context lint` | 10,617 | 0.3 | map_analysis + verification: files, API, tests. **Does not say how a lint code is added**: the codes' documentation table lives in `src/tools/describe.rs` (`LINT_CODES`) and a unit test there fails when it is missing |
| 42 | `search "how to add a new lint code"` | 2,899 | 0.3 | ADR 0007 says "adding a lint code makes `cargo test` name the doc": true, but see #52 |
| 43-45 | `src show lint`, `src show check_style`, `src show lint::check_style` | 5,335 | | `src show check_style` printed **the other** `check_style` (src/style.rs, 3.7 KB) although the packet was about lint.rs |
| 48-50, R | `src outline` ×3, read lint.rs test helpers (26 lines) | 1,939 + 1,100 | | no packet names the test helpers; opened the file |
| 46 | `iterate` (first in this tree) | 1,534 | **272 / 393** | fmt 3.7, clippy 53, unit 216 s (the lib test binary, cold). AGENTS.md: "seconds" |
| 47 | `affected --quick` | 1,353 | **325 / 384** | 4 suites, cold |
| 51 | `iterate` (after adding the test) | 681 | **105 / 195** | warm: clippy 27 s, unit 58 s for a one-file change |
| 52 | `affected` | 1,656 | 202 / 100 | **"verified (closure scope)" — a false green**: the doc guard (`tools::describe::tests::lint_table_covers_every_code_the_linter_can_emit`) is a unit test of another feature and was not scheduled. Found only by knowing ADR 0007 and running it by hand (#56: it fails). CI would have failed |
| 53 | `preflight` | 83 | 0.2 | OK — it checks describe budgets, not the lint table |
| 56-57 | doc row added to `LINT_CODES`; `preflight` | | | |
| 58 | `affected` | 656 | 277 / 487 | describe.rs pulled in more suites; **1 failure: `idea_forge`'s process-supervisor timing test** (19.2 s against an 11.1 s bound at load average ~5): unrelated, load-sensitive |
| 59 | `affected` (rerun) | 352 | 65 / 68 | green |

## Ranked sinks

Cost = what the trace actually spent (or, for the false green, what it would have spent: a hosted CI run is ~4.6 min after #97, plus re-loading the
context to fix it). Minutes are CPU-comparable where contention mattered.

| rank | sink | evidence | tokens | minutes | fix in the engine's idiom |
|---|---|---|---|---|---|
| 1 | **`start` misroutes and its NEXT can be wrong** | a: `game-change` → NEXT `scripts/dev red check`, which does not exist in a checkout, *after* a 10-min build; b: "New 2D game" unrouted; c: owners by word (scene_format/idea_forge/karts) → a 12 KB packet of the wrong feature | ~4,000 (wrong packet + reroutes) | 10.3 (a: the build ran to reach a failing command) | launchpad routes: a scene/map task in a checkout → recipe → lint → verify; "2D game"/"new game" cues → game-create; engine-change owners from `docs/features.json` file and command names (lint codes, commands) before words; every NEXT is a command that exists (test) |
| 2 | **`affected` can report a false green** | c: a unit test that guards `describe lint` sits in another feature; changing `lint.rs` did not schedule it | (a CI failure costs a whole re-read of the change) | ~5-10 per occurrence | `docs/features.json`: features whose code is guarded by `tools::describe` tests list them (a test asserts every guard test is owned by the features it guards) |
| 3 | **`search` returns the wrong domain for 3D questions** | a: three searches, 17 KB, the 2D mechanics lane first for a 3D question, ADRs that match "key" as a word; the right recipe 9th of 9; no fragment for "3D dialogue" or "door-blocked with a locked door" | ~4,250 | — | rank by domain: no 2D cue → the 2D mechanics lane does not lead; a query naming a recipe's subject puts the recipe first; recall guard over a fixed query set (as in the 2026-10-06 pass) |
| 4 | `context` lacks "how to extend" | c: no packet says "a lint code = a check in lint.rs + a row in `describe::LINT_CODES` + a unit test (helpers `run`/`room`)"; source opened | ~1,500 | — | an `extend` line per feature in features.json, printed by `context` |
| 5 | first build and first ladder run in a fresh checkout | seed 1.8 min (hint names no donor) + build 9.2 min (426 CPU-s) + first iterate 393 CPU-s + first suites 384 CPU-s | — | ~20 (CPU) | prebuilt-on-main is blocked on permissions (2026-10-09); cheap part: `start`/`seed` name the built sibling to seed from |
| 6 | warm `iterate` is not "seconds" | c: 105 s wall / 195 CPU-s for one file (clippy 27 s, unit 58 s: relinking the lib test binary) | — | 1.7 per edit | measure uncontended first; then either say minutes or narrow the unit stage |
| 7 | `door-blocked` is not phase-aware and blames furniture | a: the locked door is the "furniture" | ~1,300 | — | run `door-blocked` per phase like `unreachable`; message mentions a closed door/phase |
| 8 | 3D has no dialogue; nothing says so | a: had to infer from "(2D)" | ~1,700 | — | `describe ui` says what 3D offers for story (objective lines, start/end cards) |
| 9 | load-sensitive wall-clock test in the closure | c: `idea_forge` supervisor bound 11 s, took 19 s at load 5 | — | 4.6 (a rerun) | bound on CPU or retry, or serial tier |
| 10 | entry reading | AGENTS.md + CLAUDE.md every session; AGENTS.md line 6 contradicts the rule | 4,450 per session | — | fix the contradiction; keep AGENTS.md under a budget like `describe --brief` |
| 11 | small: `src show` ambiguity (3.7 KB), `new-game` accepts a missing engine path + `scripts/red` error without a fix, `install unverified` warnings on every validate/verify, a recipe's golden view inside a lamp | | ~1,500 | | |

**Candidates for Phase 2 (top three by the evidence):** 1 (launchpad routing: the largest minute sink and a wrong-feature packet), 2 (the false green:
correctness of the ladder itself), 3 (domain-aware search: the largest token sink of task a). Sink 5 is larger in minutes but its real fix (a prebuilt
from `main`) is the blocked permission question, not engine work.

## Phase 2 (2026-10-11): the top three sinks, before and after

Each fix is one PR with an ADR holding its own before/after numbers; the trace's own command or query was re-run.

| rank | sink | before | after | PR / ADR |
|---|---|---|---|---|
| 1 | `start` misroutes | a: NEXT a command that does not exist (after a 10-min build) + 3 searches (17 KB) to find the recipe and topics; b: unrouted + rerun (2 calls, 3.2 KB), dir `second-collect-dodge`; c: `context scene_format` (12 KB, ~3,000 tokens, wrong feature) | a: NEXT build -> `recipe`, pointers `describe style/ui/rules` + `recipe gated_garden`; b: game-create first time (1 call, 2.8 KB), dir `collect-dodge-unique`; c: `context map_analysis` directly | #104, 2026-10-11-the-launchpad-routes-by-what-a-task-names |
| 2 | `affected` false green | `iterate`, `--quick` and `affected` all pass a lint code with no `describe lint` row ("verified" in 202 s); CI fails | all three fail at the unit step naming the missing row (`affected` in 67 s) | #103, 2026-10-11-compile-time-includes-are-changes |
| 3 | search: 2D answers for 3D questions | `gated_garden` 9th of 9 for "locked door opened by a key"; a "3D scene" question led by a 2D mechanic; "two rooms joined by a door" led by `key-door` | 2nd (1st when the query says 3d); no mechanic for a 3D query; `rooms_and_door` 1st | #105, 2026-10-11-search-leads-with-the-domain-a-query-names |

Left as found (ranks 4-11): `context` has no "how to extend" line; the cold first build (blocked on the prebuilt permission); warm `iterate` ~105 s;
`door-blocked` is not phase-aware; no word that 3D has no dialogue; the load-sensitive `idea_forge` test; AGENTS.md's SPEC contradiction; small ones.

## Uniqueness, as observed (input to Phase 3)

- **Task a did not get original assets, unprompted or otherwise.** The look is non-default (palette 8/9 roles changed, sky, fog, lamp: style lint has
  nothing to say), but every prop is a shared catalogue prop or a hand-built box; `style-report` reports 0 own prefabs, 45% shared catalogue and 48% of
  surfaces still in default colours, and suggests making prefabs. **No lint fires** for any of that; only `style-report`, which nothing in the loop runs.
- **Task b complied by construction**: a 2D game draws its own sprites; there is no shared library to fall back on.
- Placing a library prop cost 0 discovery tokens here (copied from the recipe; `catalog <name>` is ~1 KB with a snippet). A novel prop's cost is measured in
  Phase 3.

## Phase 3 (2026-10-11): uniqueness

**Compliance.** Task a was asked for a non-default look and got one (8 of 9 palette roles changed, sky, fog, lamp; style lint silent because nothing
default was kept), but **no original asset, unprompted**: 10 shared catalogue placements, 12 hand-built boxes, 0 own prefabs, 0 voxel models. Nothing in
the loop said so: `lint` has no rule about it, and `style-report` (which does say "make a few prefabs of your own") is not part of `lint`/`verify`, so an
agent that follows the loop never runs it. Task b complied by construction (a 2D game draws its own sprites; `style-report` says the catalogue questions
do not apply).

**What the style lint caught:** nothing in either task. Its three codes (`style-palette`, `style-lamp`, `style-sky`) check that a game chose colours; a
game that chose colours but placed only shared props passes.

**Cost of a novel prop vs a library one** (measured on the cabin: an original `woodstove` prefab, palette-bound, 8 parts; log rows `u`):

| | calls | printed / written | first try |
|---|---|---|---|
| place a library prop (`catalog stove` -> paste the `add` line) | 1 | 558 B printed (~140 tokens) | yes |
| make an own prefab | 6: 2 searches (the first did not show the definition format), author, `lint`, `catalog --scene --local --sheet` (look), `style-report` | 10,265 B searched + 1,404 B written + ~1,000 B tool output (~3,200 tokens) + one image | lint clean on the 2nd try (the first spot covered the goal zone); the prefab itself was right first time |

About **20x the tokens** of placing a library prop, almost all of it discovery: the format is in SPEC "`prefab`" and AGENT_REFERENCE, reachable only by search.

**Proposal (not built): one lint code and one recipe, in the engine's idiom.**
1. Lint `style-own` (warn, games only, the same "game" test as the other style codes: the scene has a `ui`): a game with no own prefab and no voxel model
   while it places shared catalogue items -> "every prop is the shared catalogue's: make one of your own -> `recipe own_prefab`". This is what makes
   the agent comply unprompted: lint is the step every route already runs.
2. Recipe `own_prefab` (3D): a small room with one scene-local prefab whose params are palette-bound (`{"default": ..., "palette": "wood"}`), `$param`
   and `=` expressions, and one `voxels` object, with `checks` that pass. `recipe`, the search recipe lane (#105) and the scene route's `prop` pointer
   (#104) then lead with a paste-ready definition: discovery ~10 KB of searches becomes one ~1.5 KB `recipe own_prefab`, about 2,200 tokens saved, and
   the cost gap falls from ~20x to ~5x.
The procedural-materials design (ADR 2026-10-10-procedural-json-materials-and-shape-ops, B6) widens what an own asset can look like; it does not close
this gap, which is about an agent knowing to make one and finding the format. No new command is needed.
