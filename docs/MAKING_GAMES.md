# Making games and maps with Red Engine 2

The game-maker's page: what to run, in what order, to build a map or a game without reading Rust. The entry point is [`AGENTS.md`](../AGENTS.md); changing
the engine itself is [`ENGINE_WORK.md`](ENGINE_WORK.md).

## Start here (you should never need to read Rust)

The engine describes itself. In this order, cheapest first:

```bash
red_engine2 describe --brief            # under <!--fact:brief-kb-->2.5<!--/fact--> KB: binaries, workflow, commands, topics (read this first)
red_engine2 describe                    # overview: what exists, every command, topics (~50 lines)
red_engine2 search "<question>"         # best fragments across docs, assets, lint codes, recipes, Rust symbols
red_engine2 catalog [words|name]        # 39 props + ~155 JSON prefabs (incl. wall art, sculptures, unlit lamps); `catalog apple_red` = params + paste-ready snippet
red_engine2 catalog --category food --sheet out/food.png   # SEE the assets (labelled contact sheet)
red_engine2 recipe [name] [--new my.json]   # known-good complete maps to copy from
red_engine2 describe objects|scene|lint|physics|conventions|rules|sim|diagnostics   # exact fields, codes, numbers, rules, syntax
red_engine2 src map | find <words> | show <symbol> | refs <symbol> | deps   # only if you MUST touch Rust
red_engine2 describe glossary           # vocabulary: prop vs prefab, zone, body band, the four maps, "tire iron" naming
red_engine2 describe decisions          # ADR index: WHY it is built this way (docs/adr/); `search <topic> --kind adr`
```

Then the loop: `add/set/move` (auto-validated) -> `lint` -> `plan`/`tour`/`frame` (**look**) -> `verify`.
A scene's `checks` block (lint budget, reachability, real-physics walks, object assertions, golden
views) makes "did I break anything?" one command; `diff a.json b.json` / `diff scene.json --git`
shows what changed by object id. For the 4-map plan start from `recipe two_floor_house`
(house), `classroom_wing` (school), `convenience_store`, `rooms_and_door` (any small interior).


## Making a game: blueprints and game projects (ADR 0024)

A game is its own directory that **uses** the engine; do not fork this repo. `red_engine2 new-game ../mygame --name mygame --engine-path ../red-engine-2`
creates `game.json` (pins the engine), `blueprints/main.blueprint.json`, the built `maps/main.json`, `CLAUDE.md`, `STATUS.md`, `scripts/red` (finds/builds
the pinned engine), an assets/gameplay.json linked local asset incubator and a CI workflow; it is green from the first commit (`scripts/red check`). The loop: edit the blueprint, `scripts/red build-all`,
`scripts/red check`, `scripts/red plan maps/main.json` (look), `scripts/red serve`. A **blueprint** (`build --example`, SPEC "Blueprints") is rooms as
rectangles, doors between them, spawn groups and prop fill; it compiles to walls, floors, lamps, zones, spawns, `portals` + `interest`, fill that never
seals a door, and a `checks` block that already passes (lint, reach per room, auto-planned walks). Game logic goes in the blueprint's `scene` block as
data (`vars`/`rules`, `describe rules`) and is proven with `checks.sim`. `build --check` and `game check` fail when a committed map no longer equals what
its blueprint builds. Put custom Rust in a crate that depends on `red_engine2` as a library, never a copy of it.

**Characters, dialogue, barks, a story?** Write `.story` text and compile it (`describe story`, `docs/STORY.md`: the workflow, the format, the
craft, the proof levels); a game crate runs it in its own systems (`games/tidewatch` is the reference). Editing a story never compiles Rust.

**Not a first-person game?** (top-down, strategy, a spectator view) Keep the gameplay in scene rules and write only a client: `describe custom-client`
(ADR 0043). `red_engine2::app` gives a `LocalSession` over the real simulation, any `ViewCamera` with pointer picking, input, the world renderer, the
rules HUD and offscreen presentation checks; `examples/external/topdown_switch` is a complete one in its own crate.


## The editing loop

```
red_engine2 ls   map.json                 # what's there, where (world bounds per object)
red_engine2 info map.json sofa_1          # one object: JSON, bounds, neighbours, its lint findings
   ... edit (set / move / add / rm / clone / array / scatter / line, or edit the JSON directly) ...
red_engine2 lint map.json                 # exit 1 on errors; fix everything it says
red_engine2 plan map.json --all-floors    # top-down plan PNG per floor (open it with an image viewer)
red_engine2 tour map.json out/tour.png    # rendered views of every room + cutaway of every floor
red_engine2 walk map.json --path "0,-8; 0,1; -0.8,2; -0.8,7.6"   # replay a route with the real physics
```

Every editing command re-validates the whole scene and **refuses to write an invalid file**
(`--dry-run` previews, `--force` overrides), and prints the follow-up `lint` command.
