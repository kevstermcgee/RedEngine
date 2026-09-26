# Working on Red Engine 2 (for AI agents)

Red Engine 2 is a Rust + wgpu engine whose maps are **JSON scene files** — and whose **game rules are data too**.
`re2` walks you around one in first person; `red_server` runs the same simulation headless as an authoritative UDP
server; `red_engine2` (the CLI) validates, renders, **analyzes, edits and plays scripted matches** on maps. You almost
never need to read Rust to change a map or a game: read [`SPEC.md`](SPEC.md) for the scene language, then use the tools below.

**Read `red_engine2 describe --brief` first (about 1 KB)**, then ask `search "<your question>"`; open SPEC/AGENTS only for a topic
you cannot get from `describe <topic>`. Every command takes the global `--json` for one stable envelope (`describe diagnostics`).

> **The one rule:** never trust a map edit you haven't run through `lint`, and never judge a
> layout you haven't *looked at* (`plan` / `tour`). The tools use the game's real collision code, so
> "lint is clean and the walk test passes" means the level is playable.

## Current direction (ADR 0015): engine first, Test Lab as the dev map

Engine quality, the shared headless simulation (`src/sim/`), testing and extensibility come before map work.
Curated games, prototypes, test content and demos are published to RedEngineGames
through `games-publish.json`. Run `python scripts/publish_games.py check` after
changing that manifest or any published source. See docs/GAMES_PUBLISHING.md; never
publish build output, logs, credentials or unreviewed scratch files.
`examples/test_lab.json` (the **Red Test Lab**) is the primary development map: basic visuals, one room per
system under test (spawns, clearance gaps, stairs/ledges, static + dynamic props and stacks, hitscan lane,
character sizes). `tests/test_lab.rs` + its own `checks` guard it. `house`/`school`/`office`/`store` are
**legacy reference content**: keep them working when cheap, never let them block a better engine design, and
record any deliberate incompatibility in ADR 0015. Before pushing: `scripts/ci.sh` — `cargo fmt --check`, clippy `-D warnings`,
`cargo test --locked`, benches compile, and the **headless server** build/lint/test with `--no-default-features` (no wgpu/winit/rodio;
the same as `.github/workflows/ci.yml`, whose `headless-linux` job installs no graphics/audio libraries). Performance:
`cargo bench --bench sim` then `python benches/check.py` (`benches/README.md`); allocation, bandwidth and packet-size budgets are
ordinary tests (`tests/alloc_budget.rs`, `tests/net_budget.rs`), and `tests/ai_tasks.rs` budgets the *context* canonical AI tasks may use.

## Multiplayer (ADR 0016, 0017, 0021, 0022, 0028, 0029, 0031, 0034)

```bash
cargo build --release --bin re2 --bin red_server --bin red_bot        # everything (default features)
cargo build --release --no-default-features --bin red_server --bin red_bot   # the server on a bare box: no graphics/audio crates
target/release/red_server --map examples/test_lab.json [--spawn-group duel] [--record out/match.json] [--no-interest]
target/release/re2 --connect 127.0.0.1:27015 examples/test_lab.json     # graphical client (run two); E pick up/drop, click, wheel, R reload
target/release/red_bot --server 127.0.0.1:27015 --behavior forward:0    # headless scripted client (JSON output)
red_engine2 replay out/match.json                                        # replay a recorded match: first divergent tick + state diff
powershell -File scripts/play_multiplayer.ps1        # server + two tiled windows      (Windows)
```
The server is authoritative for **everything**: movement (`sim::player::step_player`, shared with single-player and prediction),
props, **pick-up/drop** (per-player carry; a contested prop goes to exactly one player), **bat and revolver** (cooldown, ammo,
damage, death, respawn: `sim::interact`, numbers in the scene's `weapons` block) and the scene's **rules**. Clients send inputs
and buttons only. **Interest management** (`sim::interest`): a client hears about its own room and rooms one open portal away
(zones + portals in the scene; `--no-interest` turns it off); the server acknowledges moving props per client, so a prop that
changed while you were away is delivered when you walk into range. Code: `src/sim/{match_sim,interact,interest,player,rules*,scenario,trace,replay}.rs`,
`src/net/{protocol,server,sessions,snapshots,limits,client,interp,predict,bot,session}.rs`. Hostile-network behaviour is tested
(`tests/net_abuse.rs`: garbage, floods, hostile values; rate limits and no panics are enforced in `src/net` and `src/sim` by lint).
Tests: `net_e2e` (real UDP, lossy proxy), `net_processes` (separate processes), `net_interactions`, `net_interest`, `net_budget`, `sim_replay`.
Debug env for `re2`: `RE2_WINDOW=x,y,w,h`, `RE2_AUTOWALK=forward|circle[:deg/s]`. Limits: `re2` single-player still runs its own
weapon logic (the online path uses the server's), but it runs the shared rule state machine and presents its variables, events,
hidden objects and outcome. Rule state is not replicated to network clients yet.

**Joining and the lobby (protocol v3).** `red_server --key SECRET|auto` makes joining need a key: the client proves it (HMAC challenge /
response, the key is never sent) and every datagram after the handshake is authenticated, so forged, replayed or injected packets are dropped
(ADR 0028; authentication, **not** encryption; use `--key auto`, not a short word). `--lobby` (or a scene `"match"` block, see `describe
scene`) turns on the flow lobby -> ready-up -> countdown -> timed round -> results -> rematch (`sim::flow`, ADR 0029; `--min-players
--countdown-secs --round-secs --results-secs --score-to-win` override). The graphical client shows the connect form (`re2`, then the O key or
PLAY ONLINE), the lobby, the HUD and the results (`src/ui/online.rs`, audited by `ui-check`); `re2 --connect HOST:PORT --key K --name N`
skips the form. `red_bot --key K --name N --ready` takes part headlessly. `scripts/lobby_demo.ps1` drives a real window through the whole loop.

**Prove and ship it.** `red_engine2 net-test scene.json --profile bad|all` puts a real server and real clients behind a seeded bursty-lossy laggy
proxy and judges what a player would notice (it found the freeze-then-snap of remote players after lost snapshots; ADR 0034). `red_engine2
perf scene.json` and `checks.perf` make tick time, bandwidth and promoted props a budget (ADR 0030). `red_server --upnp` /
`red_engine2 portmap` open a home router's UDP port (ADR 0031; tested against a fake router, not a real one). `red_engine2 package out.zip` /
`package --verify` make a reproducible release with a SHA-256 manifest and headless binaries proven graphics-free (ADR 0032). `red_engine2
impact --git` says which tests and docs a change touches; `features --check` keeps that index true (ADR 0033).

## Game rules, headless play, replay (ADR 0020, 0021)

`vars` + `rules` in the scene declare gameplay (triggers, conditions, actions); `checks.sim` scenarios play scripted players through
the real simulation and assert the outcome, with no window (`red_engine2 sim scene.json`); `red_engine2 replay trace.json` re-runs a
recorded match and names the first divergent tick. `describe rules` and `describe sim` have the syntax with runnable examples;
`recipe coin_run` is a complete game proven by its own scenarios. In single-player `re2`, the same rules drive hidden/shown objects,
teleports and prop impulses; a compact generic HUD shows scene variables, recent events and the terminal outcome.

## Start here (you should never need to read Rust)

The engine describes itself. In this order, cheapest first:

```bash
red_engine2 describe --brief            # ~1 KB: binaries, workflow, commands, topics (read this first)
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
the pinned engine) and a CI workflow; it is green from the first commit (`scripts/red check`). The loop: edit the blueprint, `scripts/red build-all`,
`scripts/red check`, `scripts/red plan maps/main.json` (look), `scripts/red serve`. A **blueprint** (`build --example`, SPEC "Blueprints") is rooms as
rectangles, doors between them, spawn groups and prop fill; it compiles to walls, floors, lamps, zones, spawns, `portals` + `interest`, fill that never
seals a door, and a `checks` block that already passes (lint, reach per room, auto-planned walks). Game logic goes in the blueprint's `scene` block as
data (`vars`/`rules`, `describe rules`) and is proven with `checks.sim`. `build --check` and `game check` fail when a committed map no longer equals what
its blueprint builds. Put custom Rust in a crate that depends on `red_engine2` as a library, never a copy of it.

## When a walk or a route fails (ADR 0023)

`verify` and `walk` print the object that stopped the player (`BLOCKED BY 'crate_a' [prop:crate] gap 0.00 m ...`), the passage width against the 0.7 m body,
whether `reach` agrees the target is reachable, and write an image (`out/verify/<scene>_walk<N>_explain.png`; `walk --explain out.png` for a manual run).
A straight leg that is obstructed while `reach` passes is the normal case: `walk --auto --from X,Z --to X,Z[,Y]` plans a route with the same physics,
prints waypoints and a paste-ready `checks.walk` entry (or put `{"from": [..], "to": [..], "auto": true}` in `checks.walk` and it plans every run).
`ray --from x,y,z --to x,y,z` answers line-of-sight questions (can the seeker see the hiding spot?).

## Setup

```bash
cargo build --release          # once; then use target/release/red_engine2(.exe) and re2(.exe)   (or just `scripts/dev red <command>`, any OS, any directory)
alias re='./target/release/red_engine2'      # the examples below write it as `red_engine2`
scripts/dev test               # the whole suite (catalogue, recipes, verify, search, docs-vs-code checks, netcode, sim replay...): a summary, full log in out/logs/
```

Play a map: `cargo run --release --bin re2 -- examples/house.json` (`RE2_STATS=1` prints FPS).

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

## Tool reference

| Command | Use it to | Notes |
|---|---|---|
| `validate <scene>` | Schema check with `object.field: message` errors | run by every edit already |
| `lint <scene>` | Find layout bugs (list below) | `--json`, `--strict` (warnings fail), `--cell 0.05` (finer walkability grid) |
| `reach <scene>` | Floors reached, per-zone coverage, doorways between zones, stairs, drops, perimeter leaks | `--to x,z[,y]` asks "can the player get there?"; `--from x,z` sets the start |
| `walk <scene> --path "x,z; x,z"` | Replay a route with the game's per-tick physics | prints where you actually end up (and at what floor height), or where you get stuck |
| `walk <scene> --auto --from X,Z --to X,Z[,Y]` | Plan a route (A* + real physics) instead of guessing waypoints | prints `--path` and a `checks.walk` entry; `--explain out.png` draws route, stop ring and blockers; a failed `--path` names the blocking object |
| `ray <scene> --from x,y,z --to x,y,z` | Line of sight against the real shapes | `clear`, or the first object in the way (id, kind, distance, point); `--skip id` |
| `patch <scene> '<json>'` / `--file` | Many edits (add/set/move/rm/clone/rename) as one atomic, validated call | any failing op aborts the whole patch and is named (`ops[3] (move): ...`) |
| `build <blueprint> [--out f] [--check]` | Compile a blueprint into a complete, lint-clean, self-checking map | `--example` prints a starter; `--check` = stale-map detector |
| `new-game <dir>` / `game check\|build-all\|info\|serve\|play` | Scaffold and run a game project that pins the engine | `scripts/red` wraps these |
| `status` | Resume in one screen: derived facts, git, STATUS.md | `--init`, `--note "..." --section next`, `--sync-docs CLAUDE.md` |
| `doctor` | What this machine can do (GPU/software rendering, audio, ffmpeg, UDP, output dir, git) | exit 1 only if UDP or the output dir is broken |
| `ui-shot <screen> out.png` / `ui-check` | Render and audit the 2-D screens (`menu`, `pause`, `connect`, `lobby`, `countdown`, `hud`, `rules`, `results`) with no window | `--size WxH --hover resume\|ready\|leave\|connect --message "..."`; `ui-check` audits 9 sizes |
| `plan <scene> [out.png]` | Labelled top-down plan: walls, props (ids), stairs (arrow + height), walkable area (cyan), lights, spawn, findings | `--y 3.0` picks a floor, `--all-floors`, `--ascii` (text, cheap), `--bounds=x0,z0,x1,z1` to zoom, `--scale`, `--labels all` |
| `render <scene> out.mp4` / `storyboard <scene> out.png` | Full MP4 of an animated scene (needs ffmpeg) / a multi-frame contact sheet | offline renderer; `--frames N` for the sheet |
| `frame <scene> out.png` | One rendered frame | `--eye x,y,z --at x,y,z --fov 70` free camera, `--hide 'roof' --hide 'wall2_*'`, `--cut-above 5.7` (peel off roof/upper floors) |
| `tour <scene> out.png` | Contact sheet: exterior, cutaway per floor, 2 views per zone | `--only kitchen`, `--cols 3`, custom `--view "name:ex,ey,ez:tx,ty,tz"` |
| `ls <scene>` | Objects + world bounds | `--filter sofa`, `--kind prop\|box\|stairs\|wall\|<prop name>`, `--all` (pieces), `--json` |
| `info <scene> <id>` | Everything about one object | includes nearby solids and lint findings |
| `describe [topic]` | The engine describing itself (commands come from the real CLI definition) | `--brief`; topics: brief overview commands objects scene lint physics conventions glossary decisions diagnostics rules sim multiplayer all |
| `search <words>` | Best fragments across docs/assets/lint/recipes/commands/Rust symbols | `--kind doc\|adr\|glossary\|asset\|lint\|rule\|type\|recipe\|command\|src`, `--limit` |
| `catalog [words\|name]` | Asset catalogue (props + prefabs) with tags, real sizes, params, snippets | `--tag`, `--category`, `--kind`, `--long`, `--sheet out.png --cols 5` |
| `recipe [name]` | Known-good example maps; `--new out.json` copies one, `--print` dumps it | each is lint-clean and passes its own `verify` (test-enforced) |
| `verify <scene>` | Run the scene's `checks` block (lint, reach, walk incl. auto routes, objects, views, **sim**, **perf**); PASS/FAIL with evidence and timings; exit 1 on failure | `--bless` (record golden views), `--no-views`, `--only walk[1]` / `--only "name text"` |
| `sim <scene>` | Play scripted players through the real simulation, headless: the scene's `checks.sim` or `--scenario file.json` | `--only name`, `--trace out.json` (record), `--dump-every 1` |
| `perf <scene>` | Real players walk the scene in an in-process server: tick p50/p95/p99/worst, bytes per client, largest datagram, promoted props, judged against `checks.perf` (or `--budget file`) | `--players N --secs S --windows N`; best-of windows because noise only adds time; failing output gives advice |
| `net-test <scene>` | Real server + clients behind a seeded bursty-lossy laggy UDP proxy, judged on what a player notices | `--profile lan\|wifi\|4g\|bad\|awful\|all --players N --secs S --seed N`; exit 1 on a failed check |
| `portmap status\|enable\|remove\|keep` | Open the game's UDP port on a home router via UPnP (SSDP discovery, safe ownership rules, lease renewal) | `--port --lease --router IP --allow-permanent`; also `red_server --upnp` |
| `package <out.zip>` / `package --verify <zip>` | Reproducible release zip with SHA-256 manifest, commit, dirty files; headless binaries proven graphics-free | `--allow-dirty`, `--no-build`; never overwrites |
| `features [name\|words]` / `features --check` | The feature index (`docs/features.json`): what exists, who owns which file; `--check` fails if it drifted | also a test |
| `impact <files>` / `impact --git [ref]` | What must pass when files change: features, dependents, exact `cargo test` and verification commands, docs to update | `--json` |
| `replay <trace>` | Re-run a recorded match with no renderer/socket; first divergent tick + state diff | `--scene map.json`, `--against other.json` |
| `diff a b` / `diff a --git` | Semantic diff by object id (added/removed/changed fields) | ignores formatting + float noise |
| `src map\|find\|outline\|show\|refs\|deps\|coverage` | Navigate the engine's Rust without reading files | scans on demand (never stale); `show` prints one item, bounded; `coverage` lists pub items missing a `///` doc |
| `props` | The prop library: sizes, collision, conventions | check dimensions here before placing |
| `set <scene> <id> path=value...` | Change fields: `position.1=3.0`, `material.color=#aa3322`, `rotation=[0,90,0]` | dotted paths, JSON values |
| `move <scene> <id> --to x,y,z \| --by dx,dy,dz` | Move (walls/fences shift their endpoints) | |
| `add <scene> '<json>'` / `--file f` | Add object(s); `--into lights\|zones` for those arrays | id must be unique |
| `rm <scene> <id\|glob>...` | Remove (`'fence_*'`) | |
| `clone <scene> <id> <new> --by dx,dy,dz` / `array <scene> <id> --count N --step dx,dy,dz` | Duplicate | |
| `rename`, `fmt` | Rename an id; normalize formatting | first edit of a hand-written file re-formats it once |
| `scatter <scene> --zone yard --kind tree_oak,bush --count 8 --seed 3` | Seeded random planting that avoids walls, props, and each other | `--rect`, `--exclude`, `--color`, `--scale 0.9:1.3`, `--clearance`, `--min-gap`, `--lint-ignore unreachable` |
| `line <scene> --kind hedge --from x,z --to x,z --spacing 1.8` | Evenly spaced props along a line | |

**Every command accepts the global `--json`**: one document `{schema, command, ok, exit, data, diagnostics:[{code, path, message, fix?}], stderr}`
(`data` is what the command printed, as JSON when it has a JSON form). Scene errors are `path: message` with a stable code and a did-you-mean;
**unknown fields are errors** (put notes under `x-…`/`_…`/`notes`; SPEC "Strict fields").

Negative coordinates work as values (`--eye -3,1.6,2`). Coordinates are `x,y,z` with **+Y up**;
`plan` draws **+X right and +Z down**. A prop/wall list's numbers are always meters.

### What `lint` catches (stable codes; silence one on one object with `"lint_ignore": ["code"]`)

`overlap` (prop/stairs interpenetrating another prop, wall or floor) · `floating` / `sunk` (a prop
not resting on the surface under it) · `headroom` (a ceiling/door header lower than 2.05 m over
walkable floor) · `stairs-top` / `stairs-bottom` / `stairs-narrow` / `stairs-steep` (stairs that
lead nowhere, into a wall, or are unusable) · `drop` (walkable edge with a big fall and no railing —
e.g. an open stairwell) · `leak` (the player can walk off the map: a gap in the perimeter) ·
`zone` (a declared zone is unreachable / partly sealed) · `floor` (a floor slab nobody can get to) ·
`unreachable` (a prop the player can't get near) · `door-blocked` (a doorway with furniture in
front of it or a wall behind it) · `door` (a connection narrower than 0.9 m) · `spawn` · `light`
(a lamp inside a wall) · `z-fight` (coplanar overlapping planes) · `duplicate-id`.

`lint` is for *walkable maps*. The cinematic examples (`hello_world`, `orbit_walk`) trip it
(they have no perimeter) — that's expected.

## Building or changing a map — checklist

1. **Lay out on a grid of wall centerlines.** Decide room rectangles first; write them into
   `zones` (`id`, `rect: [x0,z0,x1,z1]`, `y`). Zones are how the tools name things.
2. **Walls:** one `wall` object per straight run (see SPEC). Give every door/arch `at` (distance
   along the wall from `from`) and `width` >= 0.9; a door is >= 2.05 m tall. Exterior walls
   0.24 thick, partitions 0.15. Walls meeting at corners just work (`extend`).
3. **Floors:** one `plane` per room at `y+0.01` (tile them centerline-to-centerline so they don't
   overlap), plus a slab `box` (0.2 thick) between levels *with a hole for the stairs*.
4. **Stairs:** follow the rules in SPEC (`### stairs`). Then `walk` up them.
5. **Props:** check `props` for sizes. Origin = middle of the base; front = local +Z (see SPEC).
   Keep ~0.9 m clear in front of doors and between furniture the player must pass.
6. **Perimeter:** a closed `fence` (or walls) around the whole playable area, and trees/hedges
   outside it so the horizon is never bare.
7. **Lights:** <= 16 point lights; a lamp per room at ~0.4 m below the ceiling, `range` 6–8. One
   directional `sun` with `cast_shadows` and `shadow_center` on the map's middle.
8. **`lint`, then `plan`, then `tour`.** Fix, repeat. Add a `walk` test for new routes (see
   `tests/house_walk.rs`) so a later edit can't silently seal a door.

### Prefabs, the catalogue, and adding new assets

Place a prefab exactly like a prop: `{"id":"bowl_1","type":"prefab","prefab":"fruit_bowl","position":[3,0.78,1]}`
(see SPEC `### prefab`). Things to remember: **put items on a surface at that surface's height** (table
0.78, counter 0.95, checkout 0.99 — `lint` flags `floating`/`sunk`), wall-mounted ones (`mount: wall`)
sit on the wall's face and face into the room, small ones are walk-through by default.

**To add a new asset you do not need Rust.** Add a def to the matching `assets/<category>.json`
(`name`, `tags`, `desc`, `params` with defaults, `objects`; variants are `"extends"` + new defaults),
`cargo test --release` (it checks: expands, has tags/desc, floor prefabs start at y=0, unique names),
then `red_engine2 catalog <name> --sheet out/x.png` and *look* at it. Categories are the file names
listed in `src/prefabs.rs::BUILTIN_FILES` (a new file must be added there). For one-off items, define
them in the scene's own `"prefabs"` instead. Props (Rust, `src/props.rs`) are for things needing
custom collision.

### Recipes

```bash
# Add a room's worth of furniture, then check it
red_engine2 add house.json '{"id":"sofa_2","type":"prop","prop":"sofa","position":[-4,0,2],"rotation":[0,90,0],"material":{"color":"#3f6f8f"}}'
red_engine2 lint house.json

# Move / recolor / remove
red_engine2 move house.json sofa_2 --by 1,0,0
red_engine2 set  house.json sofa_2 material.color=#aa3322
red_engine2 rm   house.json 'bush_west_*'          # a whole scattered group, by prefix

# Re-roll a garden (same command, different --seed) — names carry the prefix so it's removable
red_engine2 rm      house.json 'flowers_back_*'
red_engine2 scatter house.json --zone back_yard --kind flower_patch --count 14 --seed 99 \
    --id-prefix flowers_back --color "#e0587a,#f2c230,#9b6be0" --exclude=-4.6,12.1,4.6,16.8

# "Why is this room unreachable?"  ->  look at it
red_engine2 plan house.json --bounds=-8,-1,8,13 --scale 60 out/debug.png
red_engine2 reach house.json --to 5.75,10.4            # is that spot reachable, and at what height?
red_engine2 frame house.json out/look.png --eye 2,1.6,2 --at 5,1,6
```

**Name things with prefixes** (`bed_master`, `bush_west_1`, `ring_n_3`) — removing or re-rolling a
feature is then one `rm 'prefix_*'`. Keep ids unique and stable.

## Lessons baked into these tools (bugs found the hard way)

- **A staircase needs a floor to land on.** Stairs whose top meets a wall, or a slab that starts
  a meter too late, "lead nowhere". `lint` says exactly which and why.
- **Slab edges are walkable steps.** The engine's rule: a collider blocks only if its top is >
  0.35 m above the player's feet (else it's stepped onto). Before that rule was unified, no
  staircase could physically be climbed onto a floor slab.
- **A door header inside the 2.0 m body band blocks the doorway.** Doors must be >= 2.05 m.
- **Furniture parked in front of a door/arch** is the most common "the room feels broken" bug;
  `door-blocked` finds it. 0.9 m clear each side of an opening.
- **Prop origins are at their base.** (Several legacy props used to be centered, silently sinking
  half into the floor; they're normalized and `props::tests::props_rest_on_the_floor` guards it.)
- **Upper-floor walls are separate objects** at the upper `y`; they don't inherit from below.
- **Point lights ignore walls.** A bright lamp lights props on the far side of a wall that face it.
- **Clarity:** two similar tones in contact (white fridge, cream wall) read as one blob — vary
  lightness, and use trims/baseboards (`wall` has both). The renderer already adds contact AO and
  outlines (`post`), but contrast is the cheapest fix.

## Reference map: `examples/house.json`

A 2-story house (14 x 12 m) on a 24 x 36 m fenced lot, ~280 objects, lint-clean. Wall centerlines
at x = -7, -1.5, 1.5, 7 and z = 0, 6/7, 12. Hall down the middle (x -1.5..1.5) with a 16-step
straight staircase (x -1.4..-0.2, z 2.5 → 7.0, +Z to climb); ground: living + study (west),
kitchen + dining + powder room (east); upstairs: master, bedroom 2, bedroom 3, bathroom; the
stairwell opening is railed. Front yard (spawn at 0,-8), side yards, back yard with patio and shed,
tree line outside the fence. It is both the demo and a worked example of every tool; its
`tests/house_walk.rs` walks every room with the real physics.

## The other three maps (`examples/school.json`, `office.json`, `store.json`)

Each has its own layout, a `checks` block (lint budget, real-physics walks, golden views in
`examples/golden/<map>/`) and is run by `tests/maps_verify.rs`. They were generated by scratch
scripts, so the JSON is the source of truth: edit it with the tools, not by re-running a generator.

- **school** (48 x 30 m, one story, ~660 objects): a long E-W corridor with lockers between four
  classrooms on the north side (rows of desks / desk pods / an art room with easels and sculptures /
  a library) and, south of it, the gym (7 m walls, court lines, hoops, climbable bleachers), the
  entrance lobby (trophy pedestal, reception) and the cafeteria with a kitchen behind a serving
  pass-through. Outside: front plaza with flag + sign, a school bus, a soccer field, a patio.
- **office** (32 x 26 m, two stories, ~610 objects): ground = open-plan cubicle bullpen with a
  collaboration corner, breakroom, two restrooms, huddle room, server room, meeting room B, lobby
  with reception; a stair core in the SW corner (landing + 18 steps + landing) leads up to the
  boardroom, two executive offices, a lounge with a pool table and bar, meeting room C, a restroom.
  Parking lot with cars in front. `walk[0]` climbs the stairs, `tests/maps_verify.rs` keeps it honest.
- **store** (20 x 14 m + gas forecourt, ~230 objects): sales floor (gondola aisles, drink wall,
  freezers, checkout, coffee station, ATM, customer restroom) and a **backroom** (stockroom with racks
  and a back door, walk-in cooler, back office with a safe). Fuel canopy with scene-local `fuel_pump`
  prefabs, price sign, cars, dumpster, propane cage.

Lessons from building them (all bit at least once):
- **Props need a colour.** A `prop` without `material.color` renders default grey/white (trees and
  bushes look like snow); the house sets one on every prop.
- **Round decor needs a collision box.** Spheres/cylinders never block the player, so a plant or
  beanbag made only of them is walk-through *and* trips `headroom`. The catalogue's big round things
  carry a hidden `core` box inside their geometry — but its top must be > 0.35 m or the player just
  steps onto it (the soil-coloured `core` in `plant_*` sticks 7 cm out of the pot for that reason).
- **Stairs need floor at both ends**: >= ~1.5 m free at the bottom (you can only enter from the bottom
  end) and a landing beyond the top (the office building is 4 m deeper than it "wanted" to be for this).
  Wall the void around them and rail the open edge; `stairs-bottom` / `stairs-top` tell you which.
- **Furniture in front of a door** and gaps < 0.75 m (a player needs ~0.7 m) cause most `door-blocked`
  / `unreachable` findings; `plan --bounds` around the door shows it instantly.
- A `walk` check starts at the scene spawn unless it has `"from": [x, z]` — routes that begin
  indoors need it.

## Engine internals (when you must change Rust)

```
src/schema.rs     JSON -> Scene (validation, macro expansion hook, `post`, `zones` ignored here)
src/macros.rs     `wall` / `fence` expand to groups of boxes at parse time (add new sugar here)
src/props.rs      prop library: parts, `collision()` policy, `lifted()` for origin-at-base
src/strict.rs     unknown-field detection with did-you-mean; the extension namespace (`x-`, `_`, notes)
src/collide.rs    static-world collision + ground height (stairs ramp, box tops), interactables — renderer-free (was in viewer.rs)
src/geometry.rs   `trs` + stair treads, shared by renderer, physics and tools
src/weapons.rs    the demo weapons as data (`weapons` in the scene: damage, revolver ammo) + timings
src/physics/      loose props on rapier: `mod.rs` (world, step), `classify.rs` (what is loose/carriable), `interact.rs` (per-player carry, strikes), `fixed.rs`
src/net/          UDP multiplayer: protocol, server (+ sessions, snapshots, limits), client, interpolation, prediction, headless bot (ADR 0016)
src/sim/          headless sim core (no wgpu/winit): clock, `match_sim` (authoritative world), `interact` (pick-up/combat), `interest` (rooms/portals),
                  `rules*` (game rules as data), `scenario` (headless play-throughs), `trace`/`replay`/`checksum` (deterministic replay, ADR 0021)
src/viewer.rs     live renderer (feature `gfx`): camera, held models, frustum culling
src/player.rs     player constants + `step_horizontal` / `vertical_step` (shared by re2 and tools)
src/render.rs     offline renderer; gpu.rs pipelines (incl. the clarity `PostFx`); shaders/*.wgsl
src/prefabs.rs    JSON prefab templates: params, `$x` / `=expr` substitution, `extends`, parse-time expansion
src/tools/        world (MapWorld) reach lint plan font edit gen inspect shots walk pathing sight patch (analysis/edit)
                  blueprint game newgame status doctor (the framework layer, handoff, environment probe)
src/ui/           headless pixel-UI kit: `Layout` of widgets drives painting, hit-testing and the audit; `screens.rs` = launch + pause menus
                  catalog describe search symbols recipes verify diff simrun envelope (AI-facing: self-description, feedback, `--json`)
assets/*.json     the built-in prefab catalogue (embedded); recipes/*.json + recipes/golden/ the example maps
docs/GLOSSARY.md  vocabulary; docs/adr/NNNN-*.md  architecture decision records (both embedded + searchable)
src/bin/re2/      the windowed game: `main.rs` (App state, setup), `frame.rs` (fixed step + update + draw), `events.rs` (winit input), `weapons.rs`, `avatar.rs`, `window.rs`
src/main.rs + src/cli/   the `red_engine2` CLI: `args.rs` (clap definition), `analyze.rs`, `editing.rs`, `info.rs`, `render_cmds.rs`, `util.rs`
Cargo feature `gfx` (default) = everything that draws or plays sound; without it the server, bot and analysis CLI still build (ADR 0017).
```

- **Docs cannot drift:** `describe` reads the CLI's own clap definition; its object-type examples,
  lint-code table and the recipes are all parsed/linted by tests. If you add an object `type`, a lint
  code or a CLI command, `cargo test` tells you what to update (`src/tools/describe.rs`).
- **Add a prop:** add a `PropKind` variant + `ALL` + `name()` + `prop_parts()` arm + parts fn in
  `props.rs` (origin at base!, front toward +Z), set `collision()` if it isn't a plain solid, add
  it to `SPEC.md`. `props_rest_on_the_floor` and the mesh tests will tell you if it's off.
- **Add sugar (`wall`-style):** write `expand_x` in `macros.rs` producing JSON, add the type to
  `MACRO_TYPES`, unit-test it, document it in `SPEC.md`. Nothing downstream needs to change.
- **Physics rule changes** go in `collide.rs`/`player.rs`/`sim/player.rs` and must keep `re2`, the server and the tools on the
  same functions. Re-run `cargo test` — `tests/house_walk.rs` is the regression net.
- **WGSL struct changes:** `Globals` and `ObjectUniform` live once, in `src/shaders/common.wgsl`, which `gpu.rs` concatenates in front
  of scene/shadow/background at compile time; `gpu::tests` checks the Rust mirrors against it without a GPU. Test on an *open* scene.
- **Verify rendering** with `frame`/`tour` (offline, no window) and, for the live path, launch
  `re2` and screenshot it (`RE2_STATS=1` for FPS). Don't rely on simulated key input for anything
  beyond a short interaction; use `walk` and the unit tests for movement.
- Commit hygiene: `cargo test --release` before committing; scene files are rewritten in a
  canonical layout by the edit tools, so diffs after the first edit of an old file are noisy once.

## Keeping the codebase cheap for the next AI

Every task should be doable from `describe`/`search`/`src show`, not by reading whole files. When you change Rust:

- **New module = a `//!` line saying what it is** (`src map` prints it) and `///` on every `pub` item
  (`red_engine2 src coverage --file <path>` lists gaps; `src show` and `search` print docs, not bodies).
- **Simulation rules are pure functions** (like `player.rs`): input state in, new state out, no window/GPU
  types. Do not add game/physics logic to `App` in `bin/re2/` — the tools and the headless server
  (ADR 0016, 0017) can only reuse what is callable without a renderer; put it in `sim/`.
- **Prefer a new file over growing a big one.** `schema.rs`, `props.rs`, `viewer.rs` are ~1000 lines; `re2`, the CLI and `physics` were
  split by subsystem (see the map above). Navigate with `src outline`/`src show`, and put new subsystems in their own module.
- **A decision that a future reader would otherwise have to re-derive gets an ADR** (`docs/adr/`, template in
  its README; a test makes you register it). A new term gets a line in `docs/GLOSSARY.md`.
- **Behaviour worth keeping is a test or a `checks` entry**, not prose: `tests/house_walk.rs` and each
  recipe's `verify` block are the living documentation of the physics and the maps.

## Characters, the launch menu, hit-testing (`re2`)

`re2 [map.json] [--as human|rat]` — without `--as` (or `RE2_CHARACTER`) a menu asks; keys `1`/`2`, click, or
arrows + Enter. The human swings the bat; **Cheddar the rat** (`type:"rat"` in a scene too) is tiny, has no
bat, has one pace of 4.0 m/s (`player::RAT_SPEED`; a human walks 3.2 and sprints 6.5), fits through 0.25 m gaps and runs under anything with a 0.25 m clearance (tables, platforms: `RAT_BAND_TOP`, `props::has_clearance`). Body numbers live in
`player::Character::body()`; models in `src/characters.rs` (`human_parts`, `rat_parts`; `frame` renders
them offline — put a `humanoid`/`rat` in a scene to look at them). Swings use `hit::raycast_shapes` (real
shapes, not bounding boxes): no hit -> no thunk. `tests/melee_hits.rs` guards it. Debug: `RE2_VIEW=third`
starts in third person. See ADR 0011.

## Weapons (bat + silver revolver) — `weapons.rs`, `revolver.rs`

Mouse wheel switches (human only); left-click swings the bat or fires the revolver (hitscan, `probe(eye, reach)`
in `bin/re2/weapons.rs` merges exact static shapes with `PropWorld::ray_props`). Ammo is the scene's `weapons.revolver.ammo`
(`"infinite"` by default, or `{loaded, capacity, reserve}`; `R` reloads; an empty cylinder clicks). Online, the same weapons run on the server (`sim::interact`). Held models are `HeldPart`s tagged with
their `weapon` (and `muzzle_flash`/`emissive`); `FrameOptions.weapon/muzzle_flash` pick what draws. Debug env:
`RE2_WEAPON=revolver`, `RE2_FREEZE_SHOT=<s since shot>` (0.02 = flash + kick) for screenshots. ADR 0013.

## Loose props (pick up / drop / knock over) — `src/physics/`

A carried prop follows where you look (`PropWorld::hold_pose`): lowered when looking down, chest height ahead, held
overhead when looking straight up, never through a ceiling or floor. Struck or shot objects make a sound but do not
change colour. `Esc` frees the mouse and opens a small pause menu (Resume / Quit game: `menu::paint_pause`,
`bin/re2/window.rs::enter_pause`); Esc or Enter resumes.

In `re2`, `E` picks up the (green-crosshair) prop in front of you and drops it again; dropped props fall,
tumble and knock things over. `physics::classify` decides what is loose (lift-able prop or floor-mounted
prefab, not a fixture; override with `"movable": true|false` on the object); the rest is a fixed collider.
`PropWorld` is a headless rapier world — loose props start as **static instances** (fixed colliders at their
authored pose, no rigid body, no entity, so an idle map costs nothing and never shuffles) and are **promoted** to
dynamic entities when disturbed (ADR 0014, `src/sim/statics.rs`). The player's own walking is
*not* rapier (ADR 0003): loose props are removed from `collect_box_colliders_except` /
`collect_ground_candidates_except` and the player is a kinematic cylinder that shoves them. The tools
(`lint`, `reach`, `walk`, `plan`) still see loose props as solid furniture at their authored spot. Tests:
`physics::tests` (pick/drop/knock/push), `tests/prop_physics.rs` (all four maps stay put when idle; lifting a
table drops what was on it). Gotchas learned: rapier's broad phase needs one `step()` before ray queries;
sleeping bodies get re-woken by `set_position`, hence *static until promoted* instead of "asleep"; props on a
non-box static (a pedestal) fall through unless round primitives are solid to props (they are, here).

## Viewer debugging helpers (`re2`)

`RE2_STATS=1` prints FPS; `RE2_FREEZE_SWING=<seconds>` holds the bat swing at that time (0.05 = windup,
0.17 = strike) so a screenshot can show the pose. The held item (bat + fist + sleeve, each its own
material) is built in `src/viewer.rs::build_held_parts`; third-person attaches it to the body's
right-hand (rig-left, part 3) forearm and orients it from the body yaw + the same pitch as first person.

The held item is placed with a `(right, up, forward)` basis, which is a **mirror** of the right-handed
world, so its meshes are wound the opposite way on purpose (`build_held_parts` flips them; the test
`held_parts_are_wound_for_the_mirrored_viewmodel_basis` guards it). Without that the pipeline's
backface culling drew the *insides* of the bat/fist and the bat showed through the hand.

`re2` is a Windows GUI-subsystem binary (no console window on launch). It re-attaches to the parent
terminal when there is one, so `RE2_STATS=1` and panics still print when started from a shell; a
scene that fails to load pops a message box when there is no terminal.
