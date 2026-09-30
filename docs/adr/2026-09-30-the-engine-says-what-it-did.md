# 2026-09-30. The engine says what it did: per-prop zone tests, atomic door actions, stated caps, explained pick-up failures
Status: accepted
Summary: `in_zone(prop, zone)` and `deactivate`/`activate` remove the two workarounds every rules game wrote; the 14 m/s prop cap, the prop collider model and a failed pick-up are now stated by `describe`, `lint` and `sim` instead of discovered by measuring; `info` reports the mass `mass(id)` reads.

## Context
Three physics games and ten minigames were built on `origin/main` (docs/analysis/2026-09-30-physics-mini-games-feedback.md,
docs/analysis/2026-09-30-ten-minigames-feedback.md), then two of the ten were rebuilt on real props. The expensive problems were not missing
features. The engine accepted something, did something other than the author meant, and said nothing:

- `throw_speed` (schema range 0-30) and `impulse` speeds saturate at `MAX_SPEED` (14 m/s); 20 and 30 behave like 14.
- `hide` is render state; the door stays solid unless `collision` is paired with it. Every game that opened a door hit this.
- A `movable: true` prop over the carry limit (0.45 m^3, 1.25 m) does nothing on E; the report showed an `interact` with no `pickup`.
- `info` estimated mass from the bounding box (about 11 kg for a crate `mass(id)` reports as 8.7), because a prefab crate is slats.
- `props_in(zone)` counts every loose prop, and no expression tests one prop against a zone, so a weighed scale needed enter/exit bookkeeping
  for each prop (16 rules and 8 flag variables for four crates), which also drifts on a bounce.

## Decision
- **`in_zone(prop, zone)`** is a two-argument built-in: 1 when that loose prop's origin is inside the zone, by the same test as `props_in`
  and `prop_enter` (`prop_inside`). A weighed pan is `in_zone(a, pan) * mass(a) + ...`, recomputed each reading, so a bounce cannot leave it wrong.
  Parsed to `Expr::Call2`; the lexer gained a comma; both ids are checked with did-you-mean.
- **`deactivate: id` / `activate: id`** do `hide` + `collision: false` and `show` + `collision: true` in one action on a top-level object.
  `hide` and `collision` keep their separate meanings.
- **The caps are stated.** `describe physics` lists `max_prop_speed_mps` and the prop model (one box collider, one density, friction 0.7,
  restitution 0.2, nothing rolls); the `impulse` action text names the cap; lint code `speed` (warn) fires for a `throw_speed` or an `impulse`
  speed above it. The schema range is unchanged: a scene that works today still loads.
- **`info` reports the collider's mass** (`physics::collider_mass`, the summed collider pieces at `PROP_DENSITY`), which is what `mass(id)` reads.
- **Lint code `carry` (info)** names a `movable: true` prop a human cannot lift, with its measured size against the limits. An info, not a warning:
  a shove-only barrel is a legitimate design.
- **A failed pick-up explains itself in `sim`.** `PropWorld::why_no_pickup` says too big (with the numbers), carried by another player, out of
  reach (with the distance), something solid in the way, or nothing under the crosshair. `MatchSim` keeps the first 8 as diagnostics (not
  checksummed, not traced, so replays are unchanged); the scenario report prints them as `note:` lines and `--json` has `pickup_misses`.
- `CLAUDE.md` notes that with `CARGO_TARGET_DIR` set the binaries are not under `target/`; a stale copy there keeps running and hides a change.

## Consequences
- No scene changes meaning. `deactivate`/`activate`/`in_zone` are new names; the new lint findings are an info and a warning.
- Weight & Balance in RedEngineGames went from 18 rules to 8 with identical weights.
- Proven by unit tests in `rules_expr`, `rules_run` and `lint`, and `tests/pickup_misses.rs`, which drives four real scenarios (a pick-up that
  works prints no note; too big, too far and nothing aimed at each print their reason).
- Not done, deliberately: a `rotate` action (physics and rendering work, needs its own ADR), `sim --sweep` / `--watch`, door-aware lint reachability.
