# Engine feedback from building three physics mini games (2026-09-30)

Three games were built with data and rules only (no Rust), each with a Python map generator and `checks.sim` scenarios: **Skyline Stacker** (stack crates, a barrel wrecks the tower), **Castle Crash** (throw ammo at a crate pyramid), **Plate Chamber** (three pressure-plate puzzle rooms). The projects were local at `/home/kevin/workspace/mini-games/*` (not published); their per-game `FEEDBACK.md` files are folded in here. Engine under test: `origin/main` at 2579c64 (item 1 was fixed in b84ad01 afterwards; the rest were checked still present at b84ad01, e.g. `MAX_SPEED` is unchanged). Everything below was measured, not assumed.

This note is a work list. Items are ranked by how much agent time they cost, each with the evidence and a suggested fix.

## Bugs and silent behaviour (fix first)

1. **[FIXED upstream in b84ad01 while this was being written; verify, nothing to do] `_`-prefixed names in `vars` were silently dropped.** `src/strict.rs:136` treats any key starting with `_` (also `x-`, `x_`) as an annotation and ignores it everywhere, including inside `vars`. The docs (`describe rules`, SPEC) say `_` variables are the internal ones the HUD hides. Result: a rule using `_in_1` fails with "unknown variable `_in_1` (declare it in `vars`...)" although it is declared. Fix: exempt the contents of `vars` from the annotation rule (or reject `_` names in `vars` with a message that says why). Add a test that a `_x` var can be declared, set and hidden from the HUD.
2. **`throw_speed` (0-30) and rule `impulse` speeds are silently capped at 14 m/s** by `MAX_SPEED` in `src/physics/mod.rs:73` (applied around line 489). Values 20 and 30 behave exactly like 14; found only by tracing a throw whose speed saturated at 13.6. Fix: clamp the schema range to the real cap (or raise the cap), and say so in `describe scene` / `describe physics`.
3. **A movable prop just over the carry limit fails to pick up with no message.** A crate scaled 1.37 is 0.451 m^3 against the 0.45 m^3 limit (`HUMAN_CARRY` in `src/physics/classify.rs`); the `sim` report showed an `interact` event with no `pickup`. Fix: a lint warning ("movable prop too big to carry: X m^3 > 0.45"), and a `sim` note when an `interact` picked nothing up (distance, aim or weight). `classify` already has the reason text ("too big or heavy for a human to carry"); surface it in `ls`/`lint`.

## Missing physics facts in `describe physics`

Measured by tracing (all should be listed, so an agent does not have to measure): every loose prop is a box collider with one density (mass = volume), friction 0.7, restitution 0.2, damped (a crate thrown flat travels only about 8 m; effective horizontal speed about 11.7 m/s at a 14 m/s release); a tall barrel tips instead of sliding; sliding friction removes about 7 m/s^2 (a barrel shoved at 6 m/s from 7.6 m stopped 2.7 m out); carry limit 0.45 m^3 and 1.25 m; the 14 m/s cap; a released prop leaves with the holder's velocity plus `throw_speed` along the look (so with `throw_speed` above 1 a plain "drop" is a throw). A sphere or cylinder prop is a box collider too, so nothing rolls (a bowling game is not possible today; worth stating).

## Lint and tools

4. **Lint cannot know a rule opens a door.** Everything behind a closed rule-opened door is reported as an unreachable-zone error, and zones have no `lint_ignore` (the engine's own `examples/external/topdown_switch` notes the same). Plate Chamber had to set `max_errors: 4` and name the four gated zones, which weakens the budget for real problems. Fix: a `gated_by`/`opens_by` on a door object (or a zone-level `lint_ignore`), so lint can treat the door as open for reachability.
5. **`frame` and `plan` ignore rule state.** A `start` rule that `hide`s the "pressed" plates only runs in the game, so `frame` draws both plates. Fix: an authored initial `"hidden": true` on objects (and `frame`/`plan` honouring it), or apply `start` rules in `frame`.
6. **`sim` scenarios are chaotic and there is no way to see how fragile one is.** Scripted stacking flipped between pass and fail when only an unrelated barrel's size changed; scripted throws hit at 16 and 18 degrees and missed at 17. Every game needed a hand-made parameter sweep (env vars in the generators) and the middle of a passing band. Fix: `sim --sweep NAME=a,b,c` or a scenario `repeat` with jitter, reporting the pass rate.
7. **No cheap way to watch one prop.** Finding why a barrel never reached a tower meant `sim --trace --dump-every N` and parsing dump arrays by index. Fix: `sim --watch <prop>` printing that prop's position/tilt/speed per interval.
8. **A scenario ends when its scripts end**, so timer rules need explicit trailing `wait`s (a "clock runs out" scenario ended after 2.5 s until a `wait` of the full time was added). Fix: an option to run to `max_seconds`, or document it in `describe sim`.
9. **Scripted aiming has no ballistic helper.** `look_at` aims straight at a point; a throw needs the arc. A `hold {aim_lob: [x,y,z], speed}` (or a documented drop/damping model) would remove the hand sweeps.

## Rules language

10. **`props_in(zone)` counts every loose prop** (hazards, toppled crates, the barrel) and there is no per-prop zone test in expressions, so a wrecked tower still scored. The workaround was long sums of `(tilt(c) < 30) * (prop_y(c) > 0.15) * (held(c) == 0)` plus a `wrecker_in` flag maintained by `prop_enter`/`prop_exit` rules. Fix: `props_in(zone, upright)` / a filter argument, or `in_zone(prop, zone)` as a function.
11. **`prop_enter` fires on a bounce.** In Plate Chamber a crate that skimmed the plate opened the door for good (`once`) but was not resting in the zone at the end. Fix: a dwell trigger, e.g. `{prop_in: zone, for: secs}`.
12. Comparison results are 1/0 and usable in arithmetic (good, and it made the score expressions possible); document it in `describe rules` (it is only visible in `rules_expr.rs`).

## Scaffolding

13. **`new-game` assumes a blueprint map.** All three projects deleted `blueprints/`, set `"blueprints": []`, and edited the scaffolded `CLAUDE.md` (still describes blueprints). Fix: `new-game --kind blank` (or `physics`): rules and a hand/generator-made map, no blueprint, with a `CLAUDE.md` that says how to use `sim` scenarios and a `tools/gen_map.py` stub.
14. A hand-held carried prop sweeps sideways as the player turns and shoves its neighbours (ammo spaced 1 m apart was scattered by the first pickup); worth one line in `describe physics` (`hold_pose`).

## What worked (keep)

The rule language reading physics directly (`tilt`, `prop_y`, `held`, `mass`, `moved`, `prop_below`, `prop_enter` with a `prop:` filter, `collision`, `reset`), `sim` scenarios that drive real pick-up, drop, throw and push through the authoritative simulation (`hold {look_at, interact}`, `hold {forward, yaw_deg}`), `sim --trace --dump-every` (found real bugs), `plan` (caught a design flaw at a glance), and `frame --cut-above`. Effort: Plate Chamber about 4 rounds, Castle Crash about 9, Skyline Stacker about 14, almost all of it physics tuning rather than rules.

## Status (checked 2026-10-04 against `red_engine2 describe rules|lint` and `sim --help`)
- **Done:** lint and rule-opened doors (item 4: scene `phases`, ADR 2026-10-03); a per-prop zone test (item 10: `in_zone(id, zone)`); the atomic `deactivate`/`activate` action (RedEngine patch R-1 of the ten-minigames note).
- **Still open:** `sim --sweep` (6), `sim --watch` (7), a ballistic aim helper (9), a dwell trigger for `prop_enter` bounces (11), `new-game --kind blank` (13); from the ten-minigames note a `rotate` action (R-4) and zone `enter` for a player who spawns inside (R-3).
