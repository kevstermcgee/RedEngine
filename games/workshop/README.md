# Workshop (a reference game for composed game systems)

A small workshop: a mine fills shared storage with ore, a furnace smelts it into ingots, a press turns ingots into gears, and the dock ships
gears. The press's output goes to storage until the player steps on the lever and sends it to the dock; the order is 12 gears. It exists to
show and test how a game owns state beyond `rules` on the engine's public API (ADR 2026-10-14-game-systems-compose-under-one-match-authority,
`red_engine2 describe systems`). It is not part of the release (`[package.metadata.red] release = false`).

```bash
cargo run -p workshop                          # play (WASD or click to move; walk onto the lever; F5 saves, Esc saves and quits)
cargo test -p workshop_server -p workshop      # systems, conservation, commands, content errors, saves, migration, online, client, agent task
target/debug/workshop_server cli sim games/workshop/content/workshop.json --inspect ws:press_a   # the scenarios, and why the press is stuck
cargo test -p workshop_server --release --test scale -- --ignored --nocapture                   # the 10 / 100 / 1000 machine probe
```

`workshop_server` is the server program and the engine's command line with Workshop's blocks (`workshop_server cli <command>`): the plain
`red_engine2` does not know the `workshop` block and refuses the map.

| path | what it is |
|---|---|
| `content/workshop.json` | the room, the lever rule, the objective (`ui`), `checks.sim`, and the `workshop` block naming the definition files |
| `content/defs/base.json` | namespace `ws`: resources, recipes, the furnace and the press (one machine behavior, two parameter sets), stores, the mine, the dock and the salvage bin, the two machines, the lever's switch |
| `content/defs/expansion.json` + `content/workshop_plus.json` | namespace `wx`, added with no code: an assembler and a recycler (the same behavior), widgets, a market; recycling gears back into ore is an intentional cycle |
| `content/saves/v1.json` | a save in version 1 of the machines' state, kept to prove the migration |
| `server/src/content.rs` | reads the definition files: two passes (define, then resolve), `file#/pointer` on every definition and problem |
| `server/src/machines.rs` | the machine behavior (`ws:machines`): feed, cycle, craft in one transaction, drain; `ws:route`, `ws:stop`, `ws:dismantle`; version 2, migrates version 1 |
| `server/src/flow.rs` | sources (`ws:supply`) and sinks (`ws:shipping`) |
| `server/src/game.rs` | Workshop as the match's game: the systems in `tick`, the lever's variable, `show` variables, inspection, saves |
| `server/src/wire.rs` | its snapshot part and the client's decoder |
| `src/lib.rs` | the client (`app::LocalSession`): lamps and HUD lines from the game's inspection, atomic saves in the game's save folder |

**Add a machine:** a definitions file with an `instances` entry naming a kind (`{"id": "furnace_b", "machine": "ws:furnace", "from":
"ws:storage", "to": "ws:storage"}`), listed in the block's `defs`; `workshop_server cli validate` names a wrong reference with its fix; `sim
--inspect <id>` shows it working, with the file it came from. A new kind is a `machines` entry; a new recipe a `recipes` entry.

**Measured** (Intel N97, release; `--test scale`): 1.5 microseconds per machine per tick (1000 machines: p50 1.54 ms, p95 1.73 ms), half of it
string-keyed lookups and allocation; the snapshot part is 8 bytes per machine and is left out of snapshots above about 170 machines
(`ServerStats::game_parts_omitted`). These are probe sizes, not capacities.
