# 2026-09-29. Prop-aware rules and scenarios: the game logic and its proofs can see loose props
Status: accepted
Summary: Rules and sim expectations see loose props: prop_enter/prop_exit/prop_below triggers, prop_y/tilt/held/mass/moved/props_in built-ins, reset/place actions, swing/prop_hit events, and {prop: ...} expectations with prop rest poses in the report.

## Context
Three physics games were built on the engine as pure data (Knockdown Alley, Domino Halls, Fling Delivery) and all three hit the same wall: rules only
saw players. `RulesEngine::inside` tested a player's body against a volume, `ENGINE_EVENTS` were pickup/drop/shot/hit/kill/respawn, expressions read
only variables, and `sim` `expect` had no prop clause. "Score = props in the pit" and "the bell fell" could not be written, so both games changed shape
(a carry-them-back claim phase; carry the fallen bell to an altar), delivery was detected from the player instead of the parcel, a lost parcel could
not be respawned, and the physics proofs lived in Python scripts parsing trace dumps instead of in `checks.sim`. The rule engine is deterministic,
checksummed and shared by the server, `sim` and the offline client (ADR 0020, 0035), so the answer has to keep that shape: no per-tick allocation in
the common case, no strings at evaluation time, and the same view of the props on every side.

## Decision
- **Rules see props as data.** `RulesEngine::step_props` takes, beside the players, one `RuleProp` per loose prop in the physics world's order
  (origin, height, tilt from the authored orientation, holder, mass, distance moved; `RuleProp::of` builds it from a `PropWorld`). `MatchSim`
  and the offline client build that list only when `RuleSet::needs_props` says a rule looks at props, into a buffer reused every tick.
- **Ids resolve once.** A `RuleSet` carries the scene's loose prop ids and zones (sorted tables; `Refs::prop_ids` comes from
  `physics::classify`), and every prop trigger, action and built-in compiles to an index into them. `RulesEngine::bind_props` maps those to physics
  prop numbers once, when the world exists. Naming a solid object where a loose prop is expected is a validate error with a did-you-mean.
- **Triggers** `{prop_enter: VOLUME}` / `{prop_exit: VOLUME}` (any loose prop, or one named by `prop: id` beside the trigger) and
  `{prop_below: [id, y]}`; edge-triggered like `enter`/`exit`, with no triggering player. A prop is *inside* when its origin is inside in x/z
  and its height band overlaps in y (`rules_run::prop_inside`); the occupancy maps are part of `RulesEngine::checksum` (only when non-empty, so
  every existing checksum and trace is unchanged).
- **Expressions** gain built-in functions `prop_y(id)`, `tilt(id)`, `held(id)`, `mass(id)`, `moved(id)`, `props_in(zone)` (`rules_expr::Func`,
  compiled by `parse_in` against a `Scope`, answered at evaluation time by a `World`; `eval` without a world reads 0 so nothing else changed).
- **Actions** `reset: id | [ids] | {zone}` (back to the authored pose, at rest, taken from any holder; `PropWorld::reset_prop`) and
  `place: [id, [x,y,z]]` (`PropWorld::place_prop`), applied by `MatchSim` and the offline client alike. A reset prop stays a (sleeping) dynamic
  body rather than being demoted to a static instance: demotion would have to unwind entity slots that network snapshots refer to.
- **Events** `swing` (a bat swing started) and `prop_hit` (a bat or a bullet struck a loose prop; once per shot) join `ENGINE_EVENTS`.
- **Scenarios** gain `{prop: id, in_zone | not_in_zone | below_y | y_lt | y_gt | tilt_gt | tilt_lt | moved | near | held_by}` expectations and
  report where every loose prop ended (`ScenarioResult::props`, printed for the props that moved, all in `--json`). The offline client's state dump
  gains `/rules` (vars, outcome, last event, hidden) and `/props`.
- Proven by `tests/fixtures/prop_rules.json` + `tests/prop_rules.rs`: a crate launched off a 2 m deck scores through `prop_enter` and fails without
  the impulse; one swing topples a three-domino line (`tilt(d3) > 60`) and fails without the swing; `reset` stands it up for a second swing that
  topples it again; `place` and `held` work; the busiest scenario is deterministic and its trace replays clean; the real client runs the same rules.

## Consequences
- The three games' workarounds become removable: Knockdown Alley's claim phase and `prove_knockdown.py`, Domino Halls' carry-the-bell win and
  `cascade_check.py`, Fling Delivery's player-side delivery detection and no-pit design can all be `checks.sim` entries and rules (not edited here).
- `RulesEngine::step` (no props) still exists for callers and tests that have none; every scene without prop rules costs nothing new per tick.
- The trace format, the wire protocol and existing checksums are unchanged; a scene that uses prop rules has a different rule checksum by design.
- A `place`d or `reset` prop is asleep, not static, so it wakes on contact like any promoted prop; its second cascade matches the first to a few
  centimetres, not bit for bit, because the player who swings again stands a little differently.
- Left for later: reading the *triggering* prop's mass in `if` (use one rule per prop or `mass(id)` for now), `prop_hit` naming the prop, and
  `hold: {look_at}` / `interact: {target}` aiming sugar for scenarios.
