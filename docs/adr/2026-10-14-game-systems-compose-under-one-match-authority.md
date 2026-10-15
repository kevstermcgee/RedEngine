# 2026-10-14. Game systems compose under one match authority
Status: accepted
Summary: A game crate writes reusable stateful behaviors as `sim::systems::System`s with namespaced definitions (`sim::defs`) and a transactional resource ledger (`sim::ledger`), and runs them as one host inside its `MatchGame`, so one authority, the match checksum, `sim --inspect`, scenario `game` expectations, versioned saves and the snapshot part all cover them; Workshop (`games/workshop`) is the reference

## Context
The owner's brief (2026-10-14) asks for games with many independently authored, interacting, persistent systems (directionally: Factorio,
Minecraft, Dota 2) without losing the engine's authoring loop (ask, get a small example, change it, inspect it, verify it), and for one bounded
milestone first: "reusable stateful composition without scene duplication", proven by a workshop (two differently configured machines, shared
storage, recipes, a player's routing decision) and a second tiny fixture (an ability with a cost and a cooldown).

What existed (main at cfff882): scene `rules` (vars, events, actions; ADR 0020) for data-only logic; scene blocks a crate defines (ADR
2026-10-13-a-scene-block-can-be-defined-above-the-core); one `MatchGame` per match (ADR 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the)
with a snapshot part and a client decoder; `persist` variables (numbers only); `checks.sim` scenarios that can read scene variables and events
only; `app::LocalSession` running the server's `MatchSim` in a custom client. Missing: per-instance game state with stable ids and provenance,
resource transfers that cannot duplicate or lose, a way to compose several behaviors with explicit dependencies and bounded events, versioned
game state with migration, the game's state in the checksum, and a way for a tool or a scenario to ask the game why something is stuck.

Considered: more rule actions (the brief: no giant JSON rule array, no second expression language); several `MatchGame`s per match
(competing authorities, the brief's explicit "no"); an entity-component store in the core (a general mechanism nothing yet needs).

## Decision
- **The primitive (red_core, headless, game-agnostic):** `sim::defs` (an `Id` is `namespace:name`, a bare reference means the file's namespace;
  `Defs<T>` keeps definitions in id order with their `file#/pointer`, refuses a second definition naming both places, replaces only on purpose,
  resolves with a did-you-mean; every problem is a `Diagnostic` with a stable code and a fix). `sim::ledger` (stores with an explicit owner and
  a capacity; a `Tx` of moves and of consumption/production for a declared recipe, source or sink applies whole or not at all; `audit` proves
  conservation; removing a store, or several in one transaction (`remove_stores`), hands their contents on or fails). `sim::systems` (a `System` declares its dependencies, events and commands;
  the `Systems` host orders them by dependency with ties by id, delivers at most a budget of events per tick and halts visibly on overflow,
  routes commands and restores the ledger and the system's state when one is refused (the host enforces "refused changes nothing", amended 2026-10-14), keeps one seeded random stream, saves each system with its own version, migrates older state step by step, refuses newer
  or unknown state, rolls back on failure, and explains an instance: status, the first reason it is blocked with the fix, its definition
  and where that is written). `write_atomic` writes a save so a crash leaves the old file, kept as `.bak`.
- **One authority:** a game runs its `Systems` from its `MatchGame::tick`. New hooks, all with defaults that change nothing for other games:
  `MatchGame::state_hash` (folded into the match checksum), `MatchGame::inspect` (`MatchSim::game_inspect`); `MatchSim::set_var` publishes
  the game's numbers to the scene's declared variables, so the existing HUD, objective, rules and scenarios read them.
- **Scenarios read game state:** `{"game": ID, "path": "a.b", <cmp>}` (a number, or text with `eq`/`ne`) and a scenario's `"inspect": [...]`
  (and `sim --inspect ID`) put the game's own answers in the result.
- **A block's files are relative to its scene** everywhere a scene file is loaded for play: `sim`, the server program, `app::LocalSession`,
  `net::bot::ClientWorld` (they parsed without the scene's folder before).
- **A game part that does not fit a snapshot is left out whole and counted** (`ServerStats::game_parts_omitted`, logged once) instead of
  being cut (a cut part failed to decode and cost the client the whole snapshot).
- **Workshop is a consumer**, `games/workshop` (client) and `games/workshop/server` (`workshop_server`: content, systems, match game, the
  server program and its `cli`), on the public API only. The custom-client layer gains `HudState::layout_scene` (the scene's `ui` block, as
  `re2` paints it).

## Consequences
A game with state of its own gets one checksum, one save format with migrations, one inspection and one scenario vocabulary for it, and an
agent can ask the engine "why is this stuck and what would fix it" in one bounded command. Measured costs and limits (probe:
`cargo test -p workshop_server --release --test scale -- --ignored --nocapture`, Intel N97): the host's tick grows about 1.5 microseconds per
machine (1000 machines: p50 1.54 ms, p95 1.73 ms); about half of it is string-keyed lookups and per-tick allocation (perf), which compiling
content into indexed runtime data removes; a whole-state snapshot part is 8 bytes per machine and stops fitting a snapshot at about 170
machines, which replication relevance and deltas remove. Systems are composed by each game; sharing a system between games is for when two
games use one (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or). Undo: the primitive is additive; the hooks default to
nothing.
