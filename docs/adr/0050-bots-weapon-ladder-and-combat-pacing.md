# 0050. Bots, the weapon ladder and combat pacing
Status: accepted
Summary: Bots (AI players with a brain), the server fills empty slots, a nav graph, the weapon ladder (Gun Game), the `combat` pacing block

## Context
The engine had twelve authoritative weapons, momentum movement, jump pads, lobby/rounds/results and prop physics, but nobody to shoot at unless other people joined,
and no way to pace a fight (respawn delay, spawn placement, protection, health). Every game built on it so far (a 30-second pickup run, an elevator mystery) is
data-only for that reason. A first-person game that is fun alone needs opponents that move, aim, miss and die by the same rules as a player.

## Decision
**Bots are ordinary players with a brain** (`sim::ai`). A bot occupies a player slot in `MatchSim`; every tick, before inputs are consumed, `MatchSim::run_bots` asks each
`Brain` for a `PlayerInput` and queues it like any client's. So bots use the shared movement (`step_player_tuned`), the same weapons, damage, death and respawn, and they
appear in snapshots and the roster with no protocol change beyond one roster flag (`ROSTER_BOT`). The recorded trace holds their inputs, so a match with bots replays
bit for bit *without* running any brain (`tests/combat_rules.rs`, replay); a brain is a pure function of the world it is shown plus a seeded SplitMix64.

A brain (skill and style are data, `sim::ai::skill`):
- perceives with a real ray, a field of view, hearing distance and a memory of where it last saw someone; targets the nearest visible enemy (humans slightly preferred, sticky);
- aims with a human-limited swing speed and a wobble that shrinks with time on target and grows with the target's sideways speed. It does not know its own wobble, so it fires
  "when aligned" and misses honestly. Reaction time, burst and rest for automatics, fresh pulls for semi-autos, bat only in arm's reach;
- fights at the distance its weapon and personality like (`WeaponProfile`, `Style`), strafes, jumps, hunts when it has nothing to shoot at, keeps its distance from other
  players (nobody collides, so bots make room themselves), and steers by rolling the *real movement function* forward a few ticks for candidate headings, which also keeps it
  off ledges and out of corners. Stuck detection breaks the rest.

**The server fills empty slots** (`bots` scene block, `red_server --fill N --bot-skill LEVEL`, `RED_FILL`, `RED_BOT_SKILL`; `ServerConfig::{bot_fill, bot_level}`): `fill` is the
number of players, humans included, the match aims for. Bots take the highest free slots (humans keep small ids), a joining human takes a bot's place (the weakest, last-added bot
leaves), a leaver hands it back, and a once-a-second sync covers expiring resume reservations. In a match flow the lobby lists the bots that will play (always ready, so only the
humans are waited for), the round is fought by them and can be won by them (`best_score` and the results include bots), and `clear_world` removes them with the bodies.

**A waypoint graph** (`nav` scene block, `sim::ai::nav`): nodes on floors and `walk` / `jump` / `pad` / `drop` edges, A* over it, nearest-node lookup. It is data the engine
checks: `red_engine2 nav check` replays every edge with the real movement code and the real jump pads (see `tools::nav`).

**A weapon ladder** (`weapons.ladder`, Gun Game): a player carries `ladder[kills]`; a kill hands the killer the next rung at once, a respawn restores the rung, and switching by hand
is disabled. With `match.score_to_win` equal to the ladder length the first player to finish it wins the round.

**A `combat` block** (`sim::combat_cfg`): `respawn_secs`, `spawn: round_robin | farthest` (the spawn farthest from every living opponent and out of their sight), `spawn_protect_secs`
(ended by the first shot), `regen_delay_secs` + `regen_per_sec`. All quantised to ticks and part of the match checksum.

**Feedback counters** on `Combat` (`shots`, `hits`, `hurt`, `hurt_bearing`) record what a client needs for sounds, hit markers and damage direction; a later ADR puts them on the wire.

## Consequences
- A match with `bots.fill` needs no other people: `red_server --fill 8` is a game. The whole thing is deterministic and replayable, and `examples/bot_match.rs` plays a headless bot match
  at hundreds of times real speed, printing kills, accuracy, distance and time spent standing still, which is how bot behaviour and arena balance are measured rather than guessed.
- The wobble model makes hitscan bots fair: accuracy falls with range and target speed, and scoped weapons are steadier. Difficulty is one number (`skill`, 0..1) plus a style.
- Limits: a match still holds at most 8 players; bots do not pick up props or use pads except through the `nav` graph; there is no team logic.
- Undo: everything is opt-in by scene block; a scene without `bots`, `nav`, `combat` or `weapons.ladder` behaves exactly as before.
