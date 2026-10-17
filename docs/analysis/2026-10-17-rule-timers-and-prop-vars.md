# Restartable timers and per-prop variables (2026-10-17)

Phase 2 item 1, second PR (ADR 2026-10-17-rule-timers-and-prop-variables-are-server-state). Stacked on the `groups` + `for_each` PR. Measured on
the 4-core N97.

## What a game writes, before and after

The patterns the five `no-per-entity-timers` runs worked around, written both ways (rule and variable counts are what the author writes):

| pattern | before (polling, timestamp variables) | after |
|---|---|---|
| a door shuts 2 s after the player last stood on the plate | 1 variable (`last_step`), 2 rules, one of them `every: 0.1` (10 condition checks a second for the whole match) | 0 variables, 2 rules (`timer` on the plate, `when: {timer}`); nothing runs until it runs out |
| each of N crates explodes 1 s after it lands in the pit | N variables + 2N rules (or 2 templates over a group), a polling rule per crate | 1 per-prop variable (`self.heat`, if wanted), 2 rules for any N (`self.fuse`) |
| a tile crumbles 3 s after it is chalked, restarted if stepped on again | a global `every` tide that breaks every chalked tile at once (Electric Tide, Gentle Carousel) | `"crumble_$tile"` in a `for_each` template: one timer per tile |

## Proof

- `sim::rule_timers` tests (6): names collected in order; a timer only started, only waited for or only cancelled is an error with a did-you-mean;
  restart replaces the deadline; cancel stops it; a zero-second timer runs out on the next tick, never the same one; every rule waiting for a timer
  fires the tick it runs out, even after an earlier one restarted or cancelled it; each prop keeps its own fuse and variable, and `self` is the
  prop whose fuse ran out; `self.` without a subject, a negative literal, an unknown `self.name` (did-you-mean) and `prop_vars` without loose props
  are errors.
- `tests/prop_rules.rs::a_crate_keeps_its_own_heat_and_fuse_and_a_restarted_timer_runs_out_once`, on the real simulation (physics, `place`,
  `prop_enter`): crate_b, placed in the pit, gets `heat = 1` and its own fuse hides it after 0.5 s, while the other crate stays at 0. Two steps on
  the pad 1.1 s apart restart the 3 s door timer, which runs out once. The trace of the fuse scenario replays clean.
- **Older scenes are unchanged:** the checksum folds prop variables and timers only for a scene that has them. Gentle Carousel's `sim --json`
  output, checksums included, is byte-identical between the step-1 build and this one. The recorded traces in the repository replay (`sim_replay`).
- **Wire:** unchanged. `PROTOCOL_VERSION` 16, `RuleState` and the snapshot untouched: clients see what the rules do (hidden, collision,
  variables, events), never a timer.
- **Per tick:** `Timers::advance` walks the scene's timers in place, so a scene without timers walks an empty list and nothing is allocated per
  tick (`alloc_budget`).

## Not done

- A HUD countdown has to copy `time` into a variable; there is no `timer_left(name)` function yet.
- Gentle Carousel keeps its global tide on purpose: switching it to per-tile timers would change the board.
- 2D (red2d) has its own runtime and gets none of this here.
