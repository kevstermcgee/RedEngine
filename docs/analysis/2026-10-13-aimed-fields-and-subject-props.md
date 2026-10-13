# Aimed fields and the rule's subject prop: Velvet Orchestra as the 3D authoring benchmark (2026-10-13)

Engine revision at the start: `b5b458f` (main; the same commit the previous review examined, so its findings were re-checked against the code, not trusted).

## 1. The benchmark, as it was

**Brief** (Idea Forge card `6859751002`, `docs/analysis/idea-forge/2026-10-12-3d-velvet-orchestra.md`): *you tune a radio dial where every frequency rewrites one rule of the
room, BUT gravity points toward the last thing you touched. Noise draws a patient listener that always arrives from a new direction. Empty the level until nothing is
left but you.*

**What the game implemented** (`examples/3d/velvet-orchestra/velvet-orchestra.json` at `b5b458f`, measured):

| | value |
|---|---|
| rules | 94 (setup 1, dial 2, lamps 8, stone touches 3, noise 3, **gravity 45**, listener 11, **slots 20**, win/lose 2) |
| vars | 13 (5 of them `it_<thing>_gone` flags, one per stolen thing) |
| file | 57,387 bytes, 4,200 lines; written by a Python generator kept in `/tmp` (not in the repository) |
| checks | lint clean; 7 `checks.sim` scenarios pass; `verify` 0.4 s |

The gravity was 45 rules `g_<thing>_<stone>_<dial>`: every 0.25 s an `impulse` along `-x`, `+x` or `+z` (one of three walls), gated on `pull` (which stone) and
`freq` (which dial setting), copied for each of 5 things x 3 stones x 3 non-silent dial settings. The slots were 20 rules: one `return` and three `bounce`
rules per thing, each guarded by that thing's `_gone` flag.

**Design decisions versus engine workarounds**

| difference from the brief | kind | evidence |
|---|---|---|
| gravity toward one of three walls, not toward the touched thing | **workaround** | feedback F2: `impulse` takes a fixed direction, no prop x/z, `fields` cannot be switched by a rule; "world shrunk to a hall with 3 axis-aligned exits so a fixed direction suffices" |
| the stones snap back to their plinths when touched (pickup + reset) | **workaround** | with a fixed direction per stone, a stone that moved would still pull toward its wall; the reset kept stone and direction consistent |
| 45 + 20 near-identical rules from a generator | **workaround** | feedback F3: no way to name "the prop that triggered this rule" or loop over props |
| only the three lodestones re-aim gravity (not the dial can or the stolen things) | design | the dial is the radio's knob; letting the stolen things re-aim would make the puzzle collapse onto whatever you carry |
| the Listener appears at a slot (shown/hidden) rather than walking there | engine limit, **not addressed here** | no rule-driven movement of non-prop characters |
| returned things are hidden but stay in the hall as invisible bodies the impulses keep pushing | side effect of the workaround | the `g_*` rules had no `_gone` guard |

Two engine defects found while tracing the path (both fixed in this pass, both independent of the benchmark):
- the shipped single-player client (`re2` offline, `src/bin/re2/frame.rs`) runs the rules but **never applied `fields`** (only `MatchSim::apply_fields` did), so a
  river or conveyor worked in `sim`, `verify` and on a server, and did nothing in the native window;
- `parse_rules` overwrote `needs_props` after `parse_fields` set it, so a scene whose fields existed but whose rules never looked at a prop got no prop views
  and its fields pushed nothing.

## 2. Acceptance specification (written before the engine change)

Observable behaviour, each item a `checks.sim` scenario or an integration test against the real simulation:

1. **Toward the touched thing, in any direction.** Picking up a lodestone makes it the room's weight: each free stolen thing then moves along the straight line from where
   it is toward the stone's *current* position. A thing north-west of a stone in the south-east corner moves south-east (both coordinates change), not along one axis.
2. **The weight moves with the stone.** While the player carries the stone, gravity follows it; set down somewhere else, the room falls toward the new spot, not the
   plinth it came from.
3. **Every touch re-aims the whole room.** Touching another stone turns everything toward that one from the next tick; things already moving change course.
4. **The dial rewrites the rule**: 88.1 gentle pull, 94.5 a rush (measurably faster than 88.1), 101.2 hush (no pull and no noise), 107.7 inverse (things move *away*
   from the stone).
5. **Slots.** A thing that reaches the slot of its own colour while the Listener is elsewhere is returned once (counted, hidden, and out of the hall so it no longer
   moves or blocks); a thing of another colour is not taken. While the Listener stands in a slot, a thing pushed in is handed back (put back where it started) and
   the lean stops (gravity points at nothing until the next touch).
6. **Noise, Listener, win, lose** as before: touching, swinging and dialling make noise unless hushed; noise 6 brings the Listener to the next slot in turn; heard
   three times loses; five returned wins.
7. **Determinism.** The same inputs give the same rule state and the same props: two runs agree tick for tick, and a recorded trace replays with no divergence.

Not accepted as a pass: an axis-aligned approximation of (1), a teleport standing in for the pull, or a scenario that only checks "the prop moved".

## 3. What was built (ADR 2026-10-13-aimed-fields-and-the-rule-s-subject-prop)

Three public concepts, each an extension of something an author already knows:

| concept | form | replaces in Velvet Orchestra |
|---|---|---|
| aimed field | `fields[]`: `toward` (prop followed live, object centre, `[x,y,z]`, `null`) + `speed` (expression, negative = away) + `props` | 45 generated `impulse` rules and the `pull` variable |
| `aim` action | `{"aim": ["gravity", "self" \| id \| [x,y,z] \| null]}` | the `pull = 1/2/3` bookkeeping |
| the rule's subject | `self` in prop built-ins and in hide/show/reset/place/impulse/aim; `"prop": id \| [ids]` beside `prop_enter`/`prop_exit`/`pickup`/`drop`/`prop_hit` | 20 slot rules and 5 `_gone` variables; `if held(knob)` buttons |

Also: any field number but `rate` may be an expression over the variables; a scenario can assert `{"field": id, "aimed_at": prop | null}`;
`describe rules` now lists the math functions and `random`, which existed but were never listed (why the backlog still reported them missing).
Fixed on the way: the offline client applies `fields` (old `re2`, field-only scene, 1.5 s: the log moved 0.00 m; now 1.98 m), and `needs_props`
is no longer overwritten. Field pushes are computed once in `RulesEngine::step_props` and returned as `Effect::Push`, so the server, `sim` and the
offline client push identically.

Not built (and why): per-entity variables and timers (the next gap, `docs/RULES_LANGUAGE_GAPS.md` rank 1; not needed here), `prop_x`/`prop_z`
(the field computes directions itself), a 3D pull or distance falloff (the card's "lean" is horizontal), templates/`for_each` (the subject removes
the duplication without an expansion step, so there is no hidden generated runtime cost).

## 4. Before and after

| | before (`b5b458f`) | after |
|---|---|---|
| rules | 94 (45 gravity, 20 slots) | 33 (1 touch rule, 6 slots) + 1 field |
| vars | 13 | 7 |
| committed file | 57,387 bytes, 4,200 lines (generator output, indent 1) | 34,330 bytes, 690 lines (`red_engine2 fmt` layout) |
| same canonical layout (`fmt`) | 44,179 bytes, 990 lines; rules block 460 lines | 34,330 bytes, 690 lines; rules + field 101 lines |
| generation step | Python generator kept in `/tmp` (lost) | none: written and read by hand |
| scenarios | 7 (76 s of play) | 9 (119 s of play), 10 checks |
| `validate` | 0.024 s | 0.023 s |
| `verify` | 0.40 s | 0.56 s (more scenarios and play time) |
| `sim`, whole run / simulated tick | 0.375 s / 82 us | 0.532 s / 75 us |
| network | rule state and props as before | unchanged: no protocol or trace change (aims are server-side rule state; clients see the props) |
| concepts beyond the shared rule vocabulary | impulse x every x var-gated duplication, plus a generator | aimed field, `aim`, `self` + `prop` lists |

The old 94-rule file still validates and passes all 7 of its own scenarios on the new engine (existing content keeps its meaning).

**Fidelity** (acceptance items of section 2 and their evidence, all in the real simulation):

| item | evidence |
|---|---|
| 1. toward the touched thing, any direction | `tests/fields.rs`: eight off-axis bearings, the crate travels 2.2-4.4 deg off its line to the target (any axis-aligned lean: 22.5 deg); scenario *mechanic*: chair and barrel end within 1.5 m of (7, -4), beside the held stone in the south-east corner, not against a wall |
| 2. the weight moves with the stone | scenario *the weight moves with the stone*: stone_n carried 14 m and set down; the barrel ends within 0.5 m of the new spot; unit test `the_target_is_followed_as_it_moves...` |
| 3. every touch re-aims | scenario *every touch re-aims the whole room*; unit tests for same-tick ordering (last pickup wins) |
| 4. dial | `tests/fields.rs` ordering: gentle moves, rush more than twice as far, hush 0, inverse ends farther than it started; scenarios *hush* (no noise, nothing moved) and *inverse* |
| 5. slots | scenarios *each slot takes only its own things* (crate and plant returned and out of the hall, chair not) and *the listener stands in the west slot* (bounced, field aimed at nothing, nothing returned) |
| 6. noise, Listener, win, lose | scenarios *lose* and *win: carry the weight east, west, then north* (all five returned, victory, 34 s) |
| 7. determinism | `tests/fields.rs`: the win and the listener scenarios and the herding fixture record and replay with no divergence, tick by tick; two runs end bit-identical |

Gameplay changed on purpose, to match the card: a picked-up stone stays in the hand (the old game reset it to its plinth because a fixed direction per
stone needed a fixed stone), so the player carries the weight and sets it down, and the slot rule is "set the stone down at the slot". Balance and comfort
are **not** proven: the scripts prove routes exist; nobody has played it with a mouse and keyboard.

**Reuse fixture** (`recipe whistle_herd`, no engine change for it): four sheep shy away (`speed: -2.5`) from a lantern the player carries; one pen rule
pair counts them with `self` and a `prop` list. Two scenarios: the flock stands still until the lantern is raised; carrying it behind them pens all four
(victory in 2.8 s). It also replays clean.

**Remaining mismatch with the card.** The pull is horizontal (things slide; they do not rise toward a raised stone). The Listener still appears at a
slot rather than walking there (no rule-driven character movement). The 14 lamp/ear show-hide rules are still polling pairs (a state-bound visibility
would remove them; not this pass). A carried prop counts for triggers where it was picked up until it is dropped (unchanged engine behaviour; only the
field follows it into the hands).

## 5. The museum boundary (inspected, not moved)

`src/museum` (5,960 lines) + `museum_gpu.rs` (244) + the client's museum mode (`src/bin/re2/museum.rs`, 889) + the `museum` command (243) + its
content type (61) are about 7,400 lines of one game in the engine crate, with inbound edges from ~25 other files.

- **Reusable services inside it:** the seeded grid maze (`layout`), geometry helpers (`shape`, on `procgen::geo`), seeded text templates (`text`: the
  backlog's missing string tables), `build::Model` (meshes, looks, colliders, lamps as data) and the GPU layer that draws such models
  (`museum_gpu::MuseumLayer`), the painted-surface shader hook (`museum.art` in `atmosphere`/`gpu`), `ambience::Journey::prefer`, `ui::serif`.
- **Game-owned:** the `museum` scene block and collection format (`spec`), what hangs where and the keys (`exhibit`), the eighteen sculpture families
  (`sculpt`), the visit and its progression and saves (`state`, `museum.json` progress under the game's settings key), the plan (`map`), its cues
  (`sound`), the client's pause menu, labels and copy (`bin/re2/museum.rs`), the `museum` command. Words, artists, themes and masterpieces are already
  data (`examples/frameless/collection.json`).
- **Why nothing moved now:** the obstacle is not code layout but shipping. Releases package only `re2`/`re2d` plus data (ADR
  2026-10-11-an-endless-museum-is-a-scene-block-the-engine-builds), so a game cannot ship its own crate; the renderers (`viewer.rs`, `render.rs`) have no
  hook through which an outside crate adds procedural models; the museum's input, pause menu and saves are woven into the `re2` loop (events, frame,
  weapons, online, headless, window, help, dump), which a game crate cannot reuse; and `schema::Scene` parses the `museum` block. Moving files under a
  new name would keep every one of these couplings. Gating it behind a cargo feature would only remove code the dedicated server never runs (the linker
  already drops it) at the price of `validate` no longer checking Frameless in a headless build.
- **Proposed boundary (a project of its own):** (1) a generic procedural-model layer (`Model` + its GPU layer) both renderers expose to games;
  (2) the painted-surface hook under a game-neutral key, `museum.art` kept as an alias; (3) seeded text templates as an engine service; (4) a release
  route that ships a game crate's binary built on `red_engine2::app`; then (5) Frameless moves out with its saves' format unchanged and a replay-style
  parity test of a seeded visit (rooms, plaques, keys) before and after.

## 6. Verification

Run in the worktree on this machine (Linux, 4 cores, software GPU through EGL); exact commands:

| command | result |
|---|---|
| `scripts/dev iterate` (fmt, clippy lib+bins, touched modules' unit tests, red2d) | ok: rules/rules_expr/rules_run/match_sim/kit/interact unit tests 92 passed; red2d 127 passed |
| `cargo test --locked --test fields` | 7 passed (8 bearings 2.2-4.4 deg off; dial ordering; trace replay of Velvet Orchestra x2 and whistle_herd) |
| `cargo test --locked --test client_headless the_offline_client_applies_fields` | passed; the pre-change `re2` on the same field-only scene: the log moved 0.00 m (new: 1.98 m) |
| `red_engine2 verify examples/3d/velvet-orchestra/velvet-orchestra.json` | 10 checks, 0 failed, 0.5 s |
| `red_engine2 sim recipes/whistle_herd.json` | 2 scenarios passed |
| `red_engine2 sim /tmp/vo/original.json` (the old 94-rule file, new engine) | 7 of 7 passed |
| `red_engine2 validate` (every content file) | 99 files, 0 problems, 0.2 s |
| `red_engine2 schema` | `scene` and `blueprint` schemas regenerated |
| `red_engine2 ui-check --scene .../velvet-orchestra.json`; `ui-shot game-start` / `game-hud` (looked at) | every screen passes at every size; start card fixed (`;` has no glyph) |
| `red_engine2 plan .../velvet-orchestra.json` (looked at) | vault closed behind the north wall, hall unchanged |
| `scripts/dev preflight --full` | OK (adr, features, facts, headless, docs, lf, describe budgets, fmt) |
| `python3 scripts/publish_games.py check` | ok |
| `scripts/dev affected --full` (= `scripts/ci.sh`), first run | 1682 of 1683 tests passed; `split_render::four_views_cost_four_views...` failed a wall-clock ratio (4.81x against 4.7x, software GPU, last test under load); alone it passes 3 of 3 (3.75-3.95x). The script stops at a failing stage, so the run was repeated |
| `scripts/dev affected --full` (= `scripts/ci.sh`), second run after the last edit | **CI OK in 3,462 s**: fmt, lock files, headless dependency tree, content (99 files), clippy `-D warnings` (all targets), tests 1683/1683 (5 skipped), benches compile, headless build, headless clippy, headless tests 1563/1563 (incl. real-UDP server tests), external custom client, Killchain, `video` feature, Python tools |

Not run here: the Windows/Direct3D leg (hosted CI only; no renderer or shader changed), a real window with mouse and keyboard (balance, comfort and
whether carrying the weight reads clearly to a player are unproven), a real multiplayer session of Velvet Orchestra (the server runs the same rules step;
`tests/fields.rs` covers determinism and replay, the headless suite covers the server).
