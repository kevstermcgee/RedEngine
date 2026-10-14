# 2026-10-14. A game plugs into the server's match simulation; the loadout shooter becomes Killchain's
Status: accepted
Summary: The match simulation holds one MatchGame chosen from the scene by game definitions registered with the scene blocks, each hook where the shooter's code runs today; the server becomes a library a game's own server binary runs; the shooter moves behind the seam and then to Killchain, its play unchanged per tests/loadout_parity.rs

## Context
ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or decided that the loadout shooter (5,112 lines: `arsenal`, `killcam`,
`sim::{shooter, kit, ordnance, objective, objective_run}`, `shooter_world`, `objective_world`, and since protocol v16 `net::arena_wire`) moves to
Killchain, its one game: first its wire structs (done, PR #153), then its simulation with a server of Killchain's own. The owner chose to finish
this move before Phase 2 (2026-10-14).

Measured at 2ce9c7a (`red_engine2 src deps`, and the references to the arena, the kit, the objective and the shooter in the shared modules): the
shooter is not a module beside the match but inside it. `sim::kit`, `sim::ordnance` and `sim::objective_run` are `impl MatchSim` blocks; the
match simulation (`match_sim`, 21 references: the start kit on joining, the arena's tick, team scores), the interactions (`interact`, 22: the
kit's tick and actions, friendly fire, dropping a kit on death, team kills and the kill record), the bots (`ai`, 44: weapon choice, grenades,
pickups, the objective), the server (`net::server`, 14: joining by team, the winner, the status), the snapshots (`net::snapshots`, 28), the scene
parser (`schema`, 13: `shooter` is a built-in key), the hosted server (`net::host`) and `red_server` (mode and team-size overrides), and the
client (`net::session`, `net::client`, `avatar`) each name it. The per-player kit is a field of the combat state (`Combat::kit`) and is in the
match checksum through it.

The museum had the same shape on the client side and moved behind `client::GameMode` (ADR
2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library); this is the server's.

## Decision
- **`sim::game::MatchGame`** (red_core): the match simulation holds `Option<Box<dyn MatchGame>>`, chosen when the match is built from the scene by
  game definitions registered with the scene blocks (`GameDef { name, build }`, `define_games`, as `world_layer::LayerDef`). Its hooks are the
  places the shooter's code runs today, called there and in the same order: a player joins or respawns (the start kit), a player's tick, a
  player's actions, whether friendly fire hurts, a death and a kill, the world's tick, team scores, the largest team, a viewer's game part of a
  snapshot (`protocol::GameMsg`) and each player's game byte, the winner, a bot's input and goal, and the game's part of the checksum.
- **A hook runs with the game taken out of the match** and is handed `&mut MatchSim`, whose public methods are the game's door to the match (as a
  client mode's `ModeContext` is); what the shooter needs that is not public becomes a named public method, not a field.
- **Per-player game state is the game's** (the kits by slot, not `Combat::kit`); the checksum mixes the game's hash for a player where the kit's was,
  so the match checksum, and every trace that records it, stays what it was.
- **The server is a library** (`server::run(games)`, as `client::run(modes)`): `red_server` runs the engine's games, Killchain's own server binary
  runs Killchain's, and a game that hosts in its own process (`net::host::LocalHost`, Killchain's SOLO and HOST) passes its definitions. The
  `shooter` scene key becomes Killchain's block; `red_server` then refuses a loadout map and says which server plays it.
- **In steps, each its own pull request, structural moves never mixed with behaviour:** (1) the parity harness, `tests/loadout_parity.rs`: bots
  play each mode and every tick is fingerprinted (the match checksum, the arena's own hash, and the encoded snapshot each player is sent); (2) the
  seam and the shooter's simulation behind it, still in red_core; (3) the bots' loadout brain behind it; (4) the server's flow (teams, winner,
  overrides) and the `shooter` block; (5) the server as a library; (6) the move of the shooter, its client pieces and the harness to
  `games/killchain`, with Killchain's server binary. The harness's four fingerprints stay the same through every step.

- *Step 2 as built (2026-10-14):* `sim::game::MatchGame` (`spawn`, `left`, `runs_weapons`, `player_tick`, `actions`, `friendly_fire`,
  `on_death`, `on_kill`, `tick`, `input_locked`, `respawn_blocked`, `spawn_side`, `move_mult`, `player_hash`), held as `MatchSim::game`;
  `ShooterGame { arena, kits }` is the shooter, its kit, ordnance and objective code its own methods taking `&mut MatchSim`. `MatchSim::damage_in`
  and `respawn_in` take the game explicitly (the shooter passes itself; the match's own calls take it out and pass it), so a death in the middle
  of a shot still drops the kit and scores the kill where it always did. `Combat::kit` is gone (`MatchSim::kit(slot)` reads it);
  `Combat::state_hash_with` folds the game's per-player hash where the kit's was.

- *Step 3 as built (2026-10-14):* `MatchGame::bot_mind` gives a new bot the game's part of its mind (`sim::game::BotMind`: `weapons`,
  `objective`, `pickup`, `reached_pickup`), kept in its brain; the brain asks it where the shooter's code ran (after aiming and firing; in its
  movement), with a `BotCtx` that lends it the brain's own random numbers, so the draws come in the same order. `ShooterBot`
  (`sim::shooter_bots`) is the shooter's: what the brain's `kit_management`, `pickup_goal` and `MatchSim::bot_objective_goal` did, and the four
  fields they remembered. `BotGoal` replaces the objective's `ObjGoal`.

- *Step 4a as built (2026-10-14):* the server's flow and what a client is sent ask the game: `teams`, `team_score`, `team_kills`,
  `score_limit`, `team_size`, `mode_wire`, `body` (a teamless or bot player's body), `log` (the objective events the server logs),
  `snapshot_part`, `player_byte`, `swinging`; `MatchSim::team_scored` replaces "a loadout match with teams". The arena's snapshot builders are
  `ShooterGame` methods (`sim::shooter_snap`). `MAX_TEAM` is the match's (`MAX_PLAYERS / 2`). The engine's `net::server` and `net::snapshots`
  no longer name the shooter; the `shooter` scene key is next (4b).

## Consequences
The engine's match simulation no longer knows a loadout match; any game with a server-side rule set of its own (a mode, a kit, an objective) plugs
in the same way. Killchain gets a server binary of its own, which its hosting (and RedEngineGames, for its download) learns: the engine's
`red_server` no longer hosts Killchain matches. The `MatchSim` public surface grows by what the shooter needs and has to stay deliberate. Undo per
step: the seam is one trait object; inlining it back is mechanical.
