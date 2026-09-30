# 2026-09-30. Killchain loadout shooter
Status: accepted
Summary: A scene with a shooter block runs a loadout team deathmatch: 31 weapons with per-gun ammo, two teams, pickups, grenades and rockets, a killcam, and its own client shell.

## Context
The engine had ten hitscan firearms sharing one ammunition pool, no teams, no projectiles and a client that was half offline prototype. Killchain (a CS:GO-style team deathmatch for friends) needs per-gun magazines, a two-gun/melee/two-grenade kit, rockets and grenades, weapons lying on the map, twelve players, a killcam and a menu. Forking the engine was ruled out (ADR 0024); a separate client crate would have duplicated `re2`'s networking glue.

## Decision
- A top-level scene block `shooter` (`sim::shooter`) switches a match into loadout mode; scenes without it are untouched (legacy weapon code paths stay).
- `arsenal` holds every weapon as a `KitSpec` row tuned to a real counterpart; `Weapon::ROSTER` (31) extends the legacy eleven without renumbering them.
- `sim::kit` (what a player does), `sim::ordnance` (projectiles, blasts, smoke, fire, flash), `sim::interact::damage_ex` (teams, friendly fire, headshots, drops) run authoritatively. Protocol v14 adds two input flag bytes, a stance byte, an `ArenaSnap`, team in lobby/roster/status, 12 players.
- `Character::{Ridgeback, Nightfall}` are the two uniforms (`uniforms`), shared by third-person bodies and first-person sleeves and gloves.
- Killcam, stats, the screens and the world objects are pure library modules (`killcam`, `stats`, `ui::killchain`, `shooter_world`); the windowed client is `bin/re2/kc`, started by `re2` when the map has a `shooter` block.
- Hosting for friends: `LocalHost` with `PublicOptions` speaks QUIC with a saved identity and UPnP; friends paste a join code (`net::join_code`).

## Consequences
New modes can be data plus one sim module instead of a fork. Cost: the wire protocol bumped, `MAX_PLAYERS` is 12 for every match, and legacy `Weapon` matches grew. To undo, drop the `shooter` block from a map.
