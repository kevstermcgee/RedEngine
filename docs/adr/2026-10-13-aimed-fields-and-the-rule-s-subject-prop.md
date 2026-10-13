# 2026-10-13. Aimed fields and the rule's subject prop
Status: accepted
Summary: A field can pull toward a target a rule re-aims (toward, speed, aim), a prop-triggered rule names its prop self, and fields run in the rules step so every driver pushes the same

## Context
Velvet Orchestra (Idea Forge, 2026-10-12) is a card whose one mechanic is "gravity points toward the last thing you touched". The rules language could not
say it: `impulse` takes a fixed direction, no rule can read where a prop is, `fields` were fixed volumes no rule could switch, and a rule fired by a prop
could not name that prop. The game shipped as 94 rules written by a generator kept in `/tmp`: 45 `every 0.25 s` impulses (5 things x 3 stones x 3 dial
settings) leaning toward one of three walls, and 20 slot rules (one per thing per slot, each guarded by its own `_gone` variable). The world was shrunk to a
hall with three axis-aligned exits so a fixed direction would do. Two engine defects were on the same path: the offline client (`re2`, which drives the
rules itself) never applied `fields`, and `parse_rules` overwrote `needs_props`, so fields in a scene whose rules never looked at a prop pushed nothing.
Measurements and the acceptance specification: `docs/analysis/2026-10-13-aimed-fields-and-subject-props.md`.

## Decision
Three public concepts, all extensions of what exists (no new interpreter, no command surface, no game-specific branch):

- **Aimed fields.** A `fields` entry may have `toward` (a loose prop, followed wherever it is including a player's hands; a top-level object's centre;
  `[x, y, z]`; or `null`) and `speed` (m/s, negative = away) instead of `velocity`, and `props` (only these loose props; any field). Each prop is pulled
  along its own horizontal line to the target: only the component toward it is driven up to `speed` (no braking, sideways motion left to friction), fading
  to zero within `AIM_ARRIVE_M` (0.5 m); the target and carried props are never pushed (`Field::pull`). Any field number but `rate` may be an expression
  over the variables, evaluated once per field per tick (`field_num`); non-finite results read 0. At most `MAX_FIELDS` (32).
- **`aim [field, target]`** moves an aimed field at run time (target: prop, object, point, `self`, `null`). It takes effect from the next tick.
- **The rule's subject, `self`.** `prop_enter` / `prop_exit` / `prop_below` and the engine events in `SUBJECT_EVENTS` (`pickup`, `drop`, `prop_hit`) bind
  the prop they are about; `self` names it in the prop built-ins (compiled to `rules_expr::SUBJECT`) and in `hide`/`show`/`reset`/`place`/`impulse`/`aim`.
  `prop: id | [ids]` beside the trigger (now also beside such an event) filters on it (`Rule::props`). An `emit` carries its subject on, like the player for
  `me.`. A trigger that cannot have one is a validate error; at run time a missing subject makes `self` actions no-ops and `self` functions 0. `self` is a
  reserved id.

Where things live, and the invariants:
- **One computation, every driver.** `RulesEngine::step_props` computes the field pushes first (`push_fields`) and returns them as `Effect::Push`;
  `MatchSim::run_rules` applies them with the same unrecorded `shove` as before (its `apply_fields` is gone) and the offline client applies them with the same
  `strike_impulse`. Fields keep acting after the match ends, as before. `needs_props` is now true whenever a field exists.
- **State ownership and bounds.** The current aim of each field is `RulesEngine::aims` (one `Aim` per field: `Off`, `Prop(index into prop_ids)` or `Point`),
  initialised from `toward` every match. A subject is transient: it lives in one firing and in the bounded event queue (64 engine events, `MAX_CHAIN`).
  Pushes are O(fields x props) per tick.
- **Ordering.** Fields in declaration order, props in physics (scene) order; rules in declaration order; prop triggers in physics order; events in the order
  they were raised; so with two pickups in a tick the later one's `aim` wins, deterministically.
- **References.** Props are never destroyed in this engine; a target or subject the physics world does not have (an unbound id) leaves the field idle and
  `self` empty. A hidden prop is still a prop (hide is presentation): it can still be a target. `RuleProp::at` is where a prop really is (the carry pose while
  held, `PropWorld::held_pose`); triggers and built-ins keep reading `origin` (a carried prop counts where it was picked up until dropped), so existing games
  keep their meaning, and only an aimed field follows a prop into the player's hands.
- **Checksum, replay, saves, network.** Aims are folded into `RulesEngine::checksum` only in scenes that have an aimed field, so every older scene and
  recorded trace keeps its checksum; field pushes are re-derived on replay, not recorded (ADR 2026-09-29-replay-applies-each-shove-once). Aims are not saved by
  `persist` (like hidden objects) and are not in the online `RuleState`: rules run on the server and clients see the props it moves in the ordinary
  snapshots. No protocol or trace format change, so no version bump.
- **Compatibility.** Velocity fields push bit-identically (literals compile to `Expr::Num`; the same `drag` arithmetic in the same order). A one-id `prop` filter
  watches and fires exactly as before. The `When::PropEnter`/`PropExit` variants lost their inline `prop` (now `Rule::props`) and `hide`/`show`/`impulse`/
  `place` take `Obj`/`PropSel`; these are crate-internal types with no outside users.

Proof: `tests/fields.rs` (eight off-axis bearings through the real physics, dial ordering, trace replay of both games), `tests/client_headless.rs`
(the offline client pushes), the rules unit tests, `examples/3d/velvet-orchestra` (10 checks) and `recipe whistle_herd` (an unrelated fixture, no engine
change).

## Consequences
Velvet Orchestra is 33 hand-readable rules and one field instead of 94 generated rules, with its card's mechanic: the pull points at the stone wherever it
is. Magnets, whirlpools, herding and "follow the lantern" are one field and one rule. Still not expressible: a 3D pull (things rising toward a raised
target), a distance falloff, a field acting on players, per-entity variables and restartable timers (`docs/RULES_LANGUAGE_GAPS.md`, rank 1), rule-driven
movement of a non-prop character. To undo: remove `toward`/`speed`/`props`/`aim` from the parser and `Effect::Push` users; the `needs_props` and offline
`fields` fixes should stay.
