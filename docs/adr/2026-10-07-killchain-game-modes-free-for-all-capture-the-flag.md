# 2026-10-07. Killchain game modes: free for all, capture the flag, search and destroy, duels
Status: accepted
Summary: A loadout match has a mode (tdm, ffa, ctf, snd) and a team size chosen by the host; the objective modes are pure state machines fed by the match tick, carried by protocol v15, and played by bots.

## Context
Killchain shipped with one mode, team deathmatch, six a side. Its first feedback (docs/ENGINE_FEEDBACK.md in the game) said a new mode cost about 4,000 lines of
source reading because there was no named place for a mode to live. Two friends in different states wanted duels, a free-for-all, capture the flag and search
and destroy on more than one map, and a way to join that needs no setup. Scoring, the lobby and the HUD all assumed "two teams, kills".

## Decision
**A mode is data on the map and a choice by the host.** `shooter.mode` is `tdm`, `ffa`, `ctf` or `snd`; `shooter.team_size` (1 to 6) is the most people a side holds, so 1
is a duel. The host overrides both without touching the file (`shooter::with_overrides`, used by `LocalHost` and `red_server --mode/--team-size/RED_MODE`), and
the map hash is still taken from the file, so a joiner's copy matches. Mode data lives beside the pickups: `flags` (one per team), `sites` (bomb sites) and
`objective` (limits and timings), all strictly validated with named paths.

**One scoring concept.** `MatchSim::team_score()` is what a team is playing for: kills (tdm), captures (ctf) or rounds won (snd). Free for all has no team score;
players score. `MatchSim::score_limit()` replaces the map's kill limit for ctf and snd. The flow takes it as `FlowInput::score_limit`.

**The objective modes are pure state machines** (`sim::objective`): `CtfState::step` and `SndState::step` take where everyone is and answer with events and commands
(`Cmd::NewRound`, `Cmd::Explode`). `sim::objective_run` feeds them from the match tick and carries the commands out; the world never learns mode rules. Search and destroy
holds players at their spawn during the freeze (`MatchSim::input_locked`), blocks respawns (`respawn_blocked`), and spawns attackers at the `team1` points and defenders
at the `team2` points whichever team they are, so swapping sides swaps who starts near the sites.

**Protocol v15.** `Status` carries the mode and team size; an `ArenaSnap` ends with an `ObjSnap` (a flag's state and place, the round and bomb, plus the last four events by
id so a missed snapshot still announces them). The kill modes pay one byte.

**The client** draws what the snapshot says (`objective_world`), words it (`ui::objective`) and plays a cue for each event. The host's mode comes from `Status`, never from the
map file the client holds.

**Bots play the objectives** (`MatchSim::bot_objective_goal`): carriers run home, defenders guard, attackers carry the bomb to a site and hold Interact, defenders rush a
planted bomb. A bot still fights anyone close. Equal-skill bots on a map with one shared central route trade kills evenly and rarely score; a human or a harder side breaks it.

**Joining.** `net::relay::DEFAULT_RELAY` is the project relay, so HOST gives a six-character code and JOIN takes one with no setup (`RE2_RELAY=off` disables it). A
server plays one map and a joiner must hold the same file, so JOIN tries the installed maps in turn until the server accepts.

### How a mode is wired (the checklist the first feedback asked for)
1. `sim/shooter.rs`: add the `ModeKind` variant, its parse, wire byte and title; add any config keys to `SHOOTER_KEYS` and parse them (`sim/objective.rs` for objectives).
2. `sim/objective.rs` (or a sibling): the pure state machine and its tests. `sim/objective_run.rs`: feed it actors, carry out its commands.
3. `sim/shooter.rs` `ArenaState` (state, and `state_hash`), `MatchSim::team_score`/`score_limit`/`teams_enabled`, `net/server.rs` (flow, winner, log line).
4. `net/protocol.rs`: the snapshot block and its codec and size accounting, and the version bump; `net/snapshots.rs` builds it.
5. Client: `objective_world.rs` (what is drawn), `ui/objective.rs` (the words), `ui/killchain.rs` (setup, lobby, HUD, results), `bin/re2/kc/game.rs` (data and cues).
6. `sim/ai` for bots, `tests/objective_modes.rs` for the integration, the map generators (`tools/mapkit.py` in the game) for data.

## Consequences
Adding a fifth mode touches a named, short list of places instead of an afternoon of reading. Maps carry flags and sites whether or not a host plays them, so the objective
objects are added to the client scene when a map has them (hidden until a snapshot shows them). The wire changed (v15), so clients and servers must match. The lobby
and results screens branch on the mode in two places each. Search and destroy rounds live inside one flow round, so the flow's own "round" is the whole match.
To undo: set `shooter.mode` back to `tdm`; nothing else about team deathmatch changed.
