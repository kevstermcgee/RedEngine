# 2026-10-14. A system stays in the engine when two games use it or the engine offers it
Status: accepted
Summary: A game-sized system stays in the engine when two or more games use it or the engine offers it to new games (a new-game kind, an engine example or recipe), and its module docs name its games; otherwise it moves to its one game, as the museum did and the loadout shooter will

## Context
ADR 2026-10-13-the-next-phase, item 5: "audit the ~30K lines: a reusable system stays and its `//!` line names the games that use it; one game's code
moves to its crate or an example; Killchain's wire structs go behind a game-message extension (a protocol version bump). The rule gets its own ADR."
The review behind it (docs/analysis/2026-10-13-next-phase-review-verified.md) counted about 30K lines of showcase-game systems in an engine of 135K: the
loadout shooter, the museum, the endless meadow, the karts, the music. "Reusable" had no test, so each system's place was argued case by case: ADR
2026-10-09-killchain-is-a-game-crate-on-the-public-engine-api kept "the reusable loadout-shooter simulation, protocol and kit" in the engine, and ADR
2026-10-11-an-endless-museum-is-a-scene-block-the-engine-builds put the museum in it, each with one game using it. The museum has since moved out
(ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library), which is what made the question concrete.

Measured on 2026-10-14 (`python3 scripts/system_users.py --games <RedEngineGames checkout>`, docs/analysis/2026-10-14-system-audit.md): the loadout
shooter is 5,112 lines used by one game (Killchain) and offered by nothing (no `new-game` kind, no engine example or recipe has a `shooter` block); the
karts are 2,252 lines used by one game (Great Outdoors) and offered by `new-game --kind race`; every other system is used by four or more games or
shown by an engine example.

## Decision
- **The rule.** A game-sized system (a scene key's worth of simulation, client and wire format) stays in the engine when two or more games use it, or when
  the engine offers it to new games: a `new-game` kind scaffolds a game on it, or an engine example or recipe (`examples/`, `recipes/`) uses it. Its
  module docs (`//!`) then name the games that use it and point at the audit. A system only one game uses, and that the engine does not offer, moves to
  that game: its crate (a game with Rust of its own), or an example (a data game). Measured, not argued: `scripts/system_users.py` counts the users.
- **The calls.** Stay: weapons and guns (8 games), music and soundscape (16), the endless meadow (4, and engine examples), day and night (4), the wrapping
  world (an engine example), the karts (one game, but `new-game --kind race` offers them). Moved: the museum (Frameless). **Moves: the loadout
  shooter** (arsenal, kit, ordnance, killcam, `sim::shooter`, the objective modes and their wire format) to Killchain. This replaces the clause of ADR
  2026-10-09-killchain-is-a-game-crate-on-the-public-engine-api that kept it in the engine: no second game uses it, none is planned (the owner's next
  games are a prop hunt, air hockey and golf), and the engine does not offer it.
- **In steps, each its own pull request:** first Killchain's wire structs go behind a game-message extension, a protocol version bump: the snapshot
  carries a game's own payload, tagged, that the core does not interpret, so the core protocol no longer names the arena's types; then the simulation
  moves with a Killchain server (the engine's `red_server` stops hosting loadout matches, as ADR
  2026-10-14-the-server-binary-stays-in-the-engine-crate-while-red foresaw for item 5's per-game servers).
  *Step 1 as built (2026-10-14, protocol v16):* `protocol::GameMsg { kind, bytes }` is a snapshot's game part (`Snapshot::game`: a wire flag, then
  its kind, a two-byte length and its bytes; `PlayerSnap::extra` rides with it); the arena's types and codec are `net::arena_wire` (kind 1,
  `ArenaSnap::to_game_msg` / `from_game_msg`); a client that cannot decode the arena drops the snapshot as a bad packet, as before. An arena snapshot
  costs three bytes more than at v15.
- A system that falls below the bar later (its second game gone, its example removed) is a candidate to move; one that gains a second game stays.

## Consequences
The engine's size tracks what more than one game needs, and "is this engine code?" has a measured answer. A game that brings a system of its own pays
for a crate and, for a networked one, a server of its own; the engine pays nothing for it. The `//!` lists can go stale as games come and go: the
script is the source, the lists are a pointer. Undo: keep a system in the engine by giving it a second user or an example, or revert the move.
