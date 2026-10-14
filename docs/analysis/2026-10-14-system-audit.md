# The system audit (2026-10-14): which game-sized systems are the engine's

ADR 2026-10-13-the-next-phase, item 5: "audit the ~30K lines: a reusable system stays and its `//!` line names the games that use it; one game's code moves
to its crate or an example". The rule this applies is ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or: a system stays when two or more
games use it or the engine offers it to new games (a `new-game` kind, an engine example or recipe); otherwise it moves to its one game.

## Method

`python3 scripts/system_users.py --games <a RedEngineGames checkout>` (the script, its system list and its rules for what a game is, are in the
repository). For each system: its lines (the files it lives in), the games whose scenes turn it on (its scene keys) or whose crate names it in Rust,
the engine examples and recipes that use it, and the `new-game` kind that scaffolds a game on it. A game is counted once wherever its copies are
(`examples/3d/x` and RedEngineGames `projects/x`); the fixture that exercises every scene key is not a game. Run at RedEngine `fa464ba` (the move of
Frameless, PR #151) and RedEngineGames `35f3b6a`.

## The table

| System | Lines | Scene keys | Games that use it | Offered by | Call |
|---|---:|---|---|---|---|
| Loadout shooter (arsenal, kit, ordnance, killcam, `sim::shooter`, objective modes, `shooter_world`, `objective_world`) | 5,112 | `shooter` | 1: Killchain (its project and its crate) | nothing | **moves to Killchain**, in steps (below) |
| Museum (`museum`, its layer, mode, tool and collection type) | 7,251 | `museum` | 1: Frameless | nothing | **moved** to `games/frameless` (PR #151) |
| Weapons and guns (`weapons`, `firearms`) | 1,496 | `weapons`, `combat` | 8: Borrowed Foundry, Drowsy Marathon, Gravity Gauntlet, Killchain, RedDM, RedEngine Sandbox, Riftraze, Velvet Orchestra | | stays |
| Karts and races (`sim::kart`, `sim::race`, `kart_camera`, `kart_sound`) | 2,252 | `race` | 1: Great Outdoors | `new-game --kind race` | stays (offered) |
| Endless meadow (`procgen`, `nature`, `terrain`, `stream_gpu`) | 5,049 | `procgen` | 4: Endless Meadow, Lantern Walk, Marcel, and Frameless (its shape kit, `procgen::geo`) | examples/endless_meadow, lantern_walk, marcel | stays |
| Day and night (`daycycle`) | 437 | `clock` | 4: Day Cycle, Endless Meadow, Lantern Walk, Marcel | examples/day_cycle and three more | stays |
| Wrapping world (`expanse`) | 322 | `world` | 1: Endless Shore | examples/endless_shore | stays (offered) |
| Music and soundscape (`music`, `listen`, `ambience`, `soundscape`) | 2,423 | `audio`, `music` | 16: Borrowed Foundry, Castle Crash, Chroma, Drowsy Marathon, Endless Shore, Frameless, Gentle Carousel, Great Outdoors, Killchain, Lantern Walk, Marcel, Plate Chamber, Redline, Skyline Stacker, Velvet Orchestra, Wormhole | examples/chroma, wormhole, marcel, lantern_walk, endless_shore | stays |

Of the ~30K lines the review counted, 12,363 are one game's (the museum, moved; the loadout shooter, moving); the rest stay, each with a module line that
names its games (`//! Used by ...`, pointing here).

## The loadout shooter, re-checked

ADR 2026-10-09-killchain-is-a-game-crate-on-the-public-engine-api kept "the reusable loadout-shooter simulation, protocol and kit" in the engine; the brief
asked that this be re-checked ("stays unless no second game would use it"). Measured: one game uses it; no `new-game` kind, engine example or recipe has a
`shooter` block; the owner's queued games (a prop hunt, air hockey, golf) are not loadout shooters; eight games use weapons and guns, which are a separate
system and stay. So it moves to Killchain, in steps:

1. **Killchain's wire structs behind a game-message extension** (a protocol version bump): the snapshot carries a tagged payload of the game's own
   (`ArenaSnap` today: the receiving player's kit, dropped weapons, projectiles, zones, effects, flags, objective events) and a per-player byte, which
   the core encodes and decodes as bytes without naming their types. The shooter's own code encodes them.
2. **The simulation moves with a Killchain server**: `sim::shooter`, `sim::kit`, `sim::ordnance`, the objective modes, `arsenal`, `killcam`,
   `shooter_world` and `objective_world` go to `games/killchain`, with a server binary of its own that hosts loadout matches (the engine's `red_server`
   then hosts the engine's kinds of match; ADR 2026-10-14-the-server-binary-stays-in-the-engine-crate-while-red foresaw per-game servers).

## Not in the table

Systems under 300 lines, the engine's shared kits (UI, audio synthesis, physics, rules), and the 2D crate (`crates/red2d`, a player for every 2D game)
are not game-sized systems.
