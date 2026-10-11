# Content formats: how each one is loaded (survey 2026-10-10; phase 2 results below)

Phase 1 of making every content format strictly typed. No code changed. **Method:** for every loader, the code was read, and for every format with a CLI
entry point an unknown key (`zzq_probe: 1`) was injected into every distinct object path of every repo file of that type (and of a synthetic scene for
the sections no repo file uses), then the loading command was asked whether it refuses it (`validate`, `verify --no-views`, `catalog --library`, `build`).
"Strict" below means measured: the injected key is refused with its path.

## Summary

Most formats are **already strict**: about 25 hand-written checkers (`strict::check_keys`, `fields::check_fields`, per-module `*_KEYS` tables) refuse an
unknown key with `path.key: unknown field — did you mean ...`. What they are not is **typed** (they walk `serde_json::Value`) or **machine-describable**
(the schema lives in code plus hand-maintained `describe` text; no JSON Schema). Two formats already use the target pattern (serde structs with
`deny_unknown_fields`): audio voices and the kart driver roster. Measured: 76 of 76 scene paths reachable by `validate` refuse unknown keys except the
gaps below, 13 of 13 in the synthetic sections, 17 of 17 `checks` paths under `verify`, all 2D game paths, all blueprint paths.

## Loaders

| content type | files | loader | parsed as | fields read | unknown keys |
|---|---|---|---|---|---|
| **Scene** (map, recipe, generated 3D game, test fixture) | `examples/*.json`, `examples/3d/*/*.json`, `examples/marcel/`, `examples/lantern_walk/`, `recipes/*.json`, `tests/fixtures/{prop_rules,throw,strike_replay}.json` | `schema::parse_scene_in` → `style::resolve`, `prefabs::expand_scene`, `macros::expand`, then per-section parsers | `Value`, hand-walked | 46 root keys (`strict::ROOT_KEYS`), every object type (`strict::object_keys`) | **refused**, with did-you-mean (except the gaps flagged below) |
| ↳ sections parsed by their own module | same | `sim::combat_cfg`, `sim::objective`, `sim::shooter`, `sim::race`, `sim::kart`, `sim::flow` (`match`), `sim::ai` (`bots`), `sim::ai::nav`, `sim::interest`, `sim::spawns`, `terrain`, `ambience` (`audio`), `atmosphere` (`sky`, `ocean`), `daycycle` (`clock`), `procgen`, `style`, `ui_config`, `hud_config`, `weapons`, `sim::rules` (`vars`/`rules`/`phases`) | `Value`, each with its own key table | their section | **refused** (each probed, incl. bots/combat/fields/fog/jump_pads/match/nav/race/shooter via a synthetic scene) |
| ↳ `checks` block | same | `tools::verify` (and the `checks_wellformed` test) | `Value` | lint, reach, walk, objects, views, sim, perf, nav, audio | **refused by `verify`; not read by `validate`** (flag 3) |
| ↳ keyframed tracks | any animated field (`position`, `rotation`, `scale`, pose joints, colours) | `schema` track parser (`{"keyframes": [...]}`) | `Value` | `keyframes[].t/value/ease` | **ignored** (flag 2) |
| **Prefab library** | `assets/{art,decor,food,furniture,karts,kitchen,lamps,office,outdoor,school,store}.json`; scene-local `prefabs` blocks; `catalog --library FILE` | `prefabs::Library::add_json` → `build_def`; objects checked at expansion (`prefabs::expand_instance`) | `Value` | `name tags desc mount params objects collide extends meta` | **def keys and param specs ignored** (flag 1); `meta` refused; objects refused at expansion |
| **Pack registry** | `assets/packs.json` | `tools::catalog::manifest_json` (embedded) | `Value`, read for display | `packs[].id/file/...` | **ignored**, no check at all (flag 5) |
| **Kart driver roster** | `assets/data/drivers.json` | `sim::kart` (`RosterFile`) | **typed serde, `deny_unknown_fields`** | `roster about base drivers[]` | refused (serde message, no did-you-mean) |
| **2D game** | `examples/2d/*.game2d.json`, `examples/patterns/*.game2d.json` | `red2d::game::parse` (+ `effects`, `game3d` for hybrid parts) | `Value`, field tables (`fields::Field`) | 21 root keys (`game::ROOT`) and every nested block | **refused** (420 of 420 object paths; the 2 accepted are user maps: `vars`, `map.legend`) |
| **Blueprint** | written by `new-game`, `build --example` | `tools::blueprint::compile_in` | `Value`, key tables | `blueprint name height ceiling rooms doors spawns fill keep_clear extra prefab_files scene` | **refused** (7 of 7; `scene.vars` is a user map) |
| **Playscript** | `playtest --script FILE` | `playscript::Script::parse` | `Value`, hand-checked | `policy steps schema notes`, every step and `expect` | **refused** (code-read; tests cover it) |
| **Music score** | `assets/audio/*.json`, `examples/*/audio/*.json`, `examples/3d/drowsy-marathon/lullaby.json` | `score::parse_score` | `Value`, key tables | top, `reverb`, `delay`, tracks | **refused** (code-read) |
| **Voice (sound)** | `audio export` output; built-ins in code; `tests/fixtures/audio_golden.json` lists them | `voice_spec::parse_voice` | **typed serde, `deny_unknown_fields`** | the voice spec | refused |
| **Game project** | `game.json` (projects made by `new-game`) | `tools::game` | `Value`, key tables | `game id name engine blueprints maps server capabilities` | **refused** |
| **Match trace / replay** | `tests/fixtures/coin_run.trace.json`, `--record` output | `sim::trace::Trace::from_json` | `Value`, hand-walked | header + `entries` | ignored (machine-written; flag 6) |
| **Settings / saves** | per-user `settings.json`, `vars.json` | `settings` | `Value`, `unwrap_or(default)` | `music sfx ...` | ignored, on purpose (runtime state; forward compatible) |
| Repo metadata (not content) | `docs/features.json`, `games-publish.json`, Idea Forge `vocabulary.json` | `tools::features`, `scripts/publish_games.py`, `examples/2d/idea-forge/build.py` | `Value` / Python | | out of scope |

## Flags: implicit or inconsistent schemas

1. **Prefab definitions accept any top-level key, and any key in a param spec**, in the shipped libraries and in scene-local `prefabs` alike (measured:
   `prefabs.fuel_pump.zzq_probe`, `...params.color.zzq_probe` and the same in `assets/*.json` all load). The param spec has two implicit shapes (a bare
   default, or `{default, desc, palette}`), and the defs come as an array (assets) or an object keyed by name (scenes).
2. **Keyframed tracks ignore unknown keys** in the track object and in each keyframe (`objects[].position.zzq_probe` and `...keyframes[].zzq_probe`
   load). A typo such as `"eaze"` on a keyframe is silently dropped.
3. **`validate` does not read `checks`**; only `verify` (and a test) does. A malformed `checks` block passes `validate`.
4. **Library object errors are deferred**: a bad object inside a library prefab is reported as `!! CATALOGUE ERROR` by `catalog <name>` (exit 0)
   and only fails when a scene places that prefab. `catalog` in list mode does not surface it.
5. **`assets/packs.json` has no schema and no check**: read as `Value` for display only.
6. **Traces** are hand-walked and tolerate extra keys. They are written by the engine, so this is low risk; a typed def would make the format explicit.
7. The scene's schema exists three times: the parser, the key tables in `strict.rs` and per module, and the `describe scene` text (a test keeps
   `describe` and `ROOT_KEYS` in step). There is no machine-readable schema for any format.
8. `Cargo.toml` has no `default-run`, so `cargo run -- validate` does not resolve (several binaries); Phase 3 needs `default-run = "red_engine2"` or
   `cargo run --bin red_engine2 --`. A `validate <scene>` command already exists (one scene or 2D game per call; 30 files in 0.3 s).

## Implication for Phase 2

The goal "bad fields are silently ignored" holds for flags 1, 2, 5 and 6 (and 3 for `validate`), not for the formats as a whole. The existing checkers'
messages are better than serde's default (`deny_unknown_fields` gives no did-you-mean and, without `serde_path_to_error`, no JSON path). Replacing ~25
checkers with derived structs is a large change to code that is already strict; the risk is regressing message quality and behaviour (defaults, sugar
such as `wall`/`fence`/`text`/`voxels` macros, `@role` resolution, prefab expansion order) for little gain in strictness.

## Phase 2 (2026-10-11): typed definitions and the drift guard

Every format above now has a typed definition in `src/content/` (`deny_unknown_fields`), and `tests/content_drift.rs` fails CI when a definition and
its checker disagree about any repository file or any key in it (ADR 2026-10-11-content-formats-are-typed-and-checked-twice). The checkers still write
every error message. Fixed in the checkers: flags 1-5 above, plus three gaps the guard found while it was built (unknown keys inside
`style.light.sun/ambient/lamp` and a text's `backing`; 2D `map.legend` entries no row uses were never checked). Traces (flag 6) are typed but keep
their lenient reader; settings and saves stay lenient on purpose. **No content file in the repository failed either definition.**

| format | typed definition | samples the guard uses |
|---|---|---|
| scene | `content::scene::Scene` (+ `objects`, `checks`, `prefab`) | every scene in examples/, recipes/, tests/fixtures/ (incl. `content/every_section.json`), `new-game` maps |
| prefab library | `content::prefab::PrefabLibrary` | `assets/*.json` |
| pack registry | `content::packs::PackRegistry` (also its loader) | `assets/packs.json` |
| 2D game | `content::game2d::Game2d` | examples/2d, examples/patterns, `new-game --kind 2d` |
| blueprint | `content::blueprint::Blueprint` | `build --example`, `new-game`, `tests/fixtures/content/every_key.blueprint.json` |
| playscript | `content::playscript::Script` | the `describe playtest` example, `tests/fixtures/content/every_step.play.json` |
| score | `content::score::Score` | assets/audio, examples/*/audio |
| voice | `content::voice::Voice` | every 2D game's `sounds` |
| `game.json` | `content::project::Project` | `new-game` of each kind |
| kart drivers | `content::drivers::DriverRoster` | `assets/data/drivers.json` |
| trace | `content::trace::Trace` | `tests/fixtures/*.trace.json` (files only; the reader is lenient) |
