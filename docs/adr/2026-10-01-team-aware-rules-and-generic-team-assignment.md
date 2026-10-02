# 2026-10-01. Team-aware rules and generic team assignment
Status: accepted
Summary: A scene can opt into team 1/2 assignment outside a loadout shooter match, so who: team1/team2 works in rules for any two-sided game

## Context
Building toward a hide-and-seek (prop hunt) game surfaced a real gap: the data-driven rules engine (`vars`/`rules`)
could already filter `who` a rule applies to, but only by species (`any`/`human`/`rat`), never by team — even
though `MatchSim` already carries a `team: u8` on every player. Worse, there was no way to *get* a team assigned
at all outside a loadout `shooter` match: every team-assignment call site (`pick_team`, the lobby's team choice,
bot-fill balancing) was hard-gated behind `is_loadout()`, even though `set_team`/`team_of`/`add_player_in_slot_team`
themselves had no such gate. A hide-and-seek game needs two asymmetric roles (hiders/seekers) with rules that act
differently per side, and that's exactly what team filtering is for — it just needed to work without dragging in
the whole loadout-shooter arena (weapons, pickups, kill scoring) a non-shooter game has no use for.

## Decision
A new scene-level `"teams": true` (parsed like `flashlight`/`music`) opts a non-shooter scene into team assignment.
`MatchSim::teams_enabled()` is `is_loadout() || teams_requested` — every existing shooter-mode behavior is
preserved exactly (it's an OR against the old gate), and the three `net/server.rs` call sites that assigned teams
(`pick_team`, `on_lobby`'s lobby team-pick, `sync_bots`'s bot-fill balancing, add and remove paths) now check
`teams_enabled()` instead of `is_loadout()`. `bots.roster[].team` lets a scene pin a specific bot to a side
deterministically, on top of the auto-balance path. `RulePlayer` now carries `team: u8` (populated every tick in
`MatchSim::run_rules` from data that was already sitting right there on `ServerPlayer`), and `Who` gained `Team1`/
`Team2` variants alongside the existing `Any`/`Human`/`Rat`, so `"who": "team1"` / `"who": "team2"` works in any
rule's `when: {enter/exit/event}` trigger.

One correction made along the way: `add_bot_in_slot_team`'s loadout-only behavior of reskinning a teamed bot into
the Ridgeback/Nightfall uniform was previously keyed off `team != 0` — which would have forced every teamed bot in
a *non-shooter* teams scene into a team-deathmatch uniform too. That reskin now only happens when `is_loadout()` is
actually true; a non-shooter teamed bot keeps whatever character its roster asked for.

Deliberately not done: `if` expressions still cannot read a player's team (only the declarative `who` filter can) —
`rules_expr`'s `Ctx`/`World` has no player data at all today, only props and zones, and threading the triggering
player through it is a materially bigger change than this scene needed. `who: team1`/`who: team2` covers everything
a hide-and-seek, capture-the-flag or other two-sided game needs for now.

## Consequences
Any future asymmetric-role game gets team-aware rules for free, not just prop hunt — this was deliberately built as
reusable plumbing, not a single-game special case. `net_core`/`net_server` picked up two new tests
(`tests/teams.rs`'s end-to-end non-shooter team-assignment test, plus the rules-engine unit tests in
`rules.rs`/`rules_run.rs`). To undo: remove `Who::Team1`/`Team2` and the `teams`/`teams_requested`/`teams_enabled`
plumbing; `bots.roster[].team` and the `is_loadout()`-gated reskin fix are independently useful and could stay.
