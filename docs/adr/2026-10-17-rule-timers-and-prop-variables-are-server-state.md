# 2026-10-17. Rule timers and per-prop variables are server state
Status: accepted
Summary: Restartable named timers (`timer`, `cancel`, `when: {timer}`; `self.name` per loose prop) and per-prop variables (`prop_vars`, `self.name`) live in the rule state and the match checksum but never on the wire, so no protocol bump; one per static object comes from a `for_each` template

## Context
Phase 2 item 1, second step (`docs/RULES_LANGUAGE_GAPS.md` gap 1). The evidence is about time: `no-per-entity-timers` in five runs ("`every` has no
once-after-reset timer; re-activating every 1 s is a polling workaround", "a global `every: 2` tide breaks every crate at once instead of a per-crate
timer", "global timer vars per wasp"). `every` fires forever and `after` once per match, so "two seconds after the player last stepped here" was a
timestamp variable and a polling rule per thing. The first step (ADR 2026-10-17-rules-repeat-over-groups-by-parse-time-expansion) already gives a
static object one variable or one rule per member. What it cannot give is a timer, or state for a loose prop that a single rule can reach through its
subject (`self`) the way `me.` reaches the player's.

The gap note expected either choice of replication to be a protocol bump. That holds only if clients receive the new state. They do not need it:
clients draw hidden/shown objects, collision, variables, events and the outcome, and a timer or a prop's variable shows only through what the rules
do with it.

## Decision
- **Timers** (`sim::rule_timers`): `{"timer": [name, secs]}` starts one, or restarts it if it is running (`secs` is a number of seconds or an
  expression; a negative literal is an error). `{"cancel": name}` stops it. A rule with `when: {"timer": name}` fires once when it runs out. Every rule
  waiting for it fires that tick, even after an earlier one restarted it, because a timer is marked as run out at the start of the tick. A restart
  lands at least one tick later, so a timer cannot loop within one tick. Names are collected from the rules before they are parsed. A timer that is
  only started, only waited for, or only cancelled is an error with a did-you-mean.
- **Per-prop variables:** `"prop_vars": {"heat": 0}` gives every loose prop its own copy (at most 8). `self.heat` reads and writes the subject
  prop's copy in expressions and in `set`/`add`, mirroring `me.name` for players. `self.fuse` is each prop's own timer, and its trigger fires once
  per prop, with that prop as the subject. Using `self.` where a rule has no subject is the same error as `self` there today. Scenarios check one
  with `{"prop_var": "heat", "of": "crate", "eq": 1}`.
- **One per static object:** a template: `"close_$door"` timers and `"_s_$tile"` variables (step 1). No runtime notion of an entity was added for
  objects that never move.
- **State, not wire:** both are part of `RulesEngine::checksum`, which folds them in only for a scene that has any, so every older checksum and
  recorded trace is unchanged. Replays re-derive them from inputs. Nothing is added to `RuleState` or the snapshot: `PROTOCOL_VERSION` stays 16.

Not chosen: replicating timers so clients could draw countdowns (a rule can copy what a HUD needs into a variable; a `timer_left` function can come
later). A runtime trigger over a group's volumes with an implicit subject (it would duplicate step 1 and need a new subject kind). Timers in 2D (red2d
has its own runtime).

## Consequences
The polling pattern becomes two rules and no variable. A per-crate fuse is one rule for every crate. `tests/prop_rules.rs` proves both on the real
simulation, with a clean trace replay. The engine-level tests in `sim::rule_timers` cover restart, cancel, every waiter firing, and each prop
keeping its own. A future HUD countdown or a networked timer would be a protocol change with its own ADR. Undo: remove the module and the four
`When`/`Action` arms; scenes that use them would fall back to `every` and timestamp variables.
