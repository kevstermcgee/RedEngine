# 2026-09-29. Authoring ergonomics from the physics games: the small things three builders lost time to
Status: accepted
Summary: Scenario hold steps take look_at, blueprint scene.zones merge by id, vars starting with _ stay off the generic HUD, info says whether an object is loose and why, checks.lint takes ignore, and describe/SPEC state zone edge order and topple direction.

## Context
Beside the four engine changes the physics games asked for (ADRs 2026-09-29-replay-applies-each-shove-once, -prop-aware-rules-and-scenarios,
-one-release-velocity, -verification-honours-the-map), their reports listed a dozen small frictions, each cheap and each learned the slow
way: `describe sim` hid half of `hold`'s keys, so nobody could script a pickup without reading Rust; aiming a swing or a pickup meant
hand-computing yaw and pitch from geometry, and the view of the press tick lags a step; a blueprint's `scene.zones` silently replaced the
generated zones and then failed with `portals[0] ... no zone a`; the generic HUD showed every variable raw (`T_START: 12`); which props are
loose (`movable`, fixtures, carry limits) and how heavy they are was only in the source; a pit's honest `drop` edges had to be budgeted as
warnings; the order of `enter`/`exit` and the direction a tall prop falls were measured by trial and error.

## Decision
- **`hold: {look_at: [x, y, z]}`** (`sim::scenario`): each tick the yaw and pitch are computed from the player's standing eye to the point,
  the same numbers a player looking there would send; documented with the full key list in `describe sim` (and a unit test keeps the
  list complete). Pickups are scripted as `look_at` the prop's origin plus about half its height.
- **Blueprint `scene.zones` merge by id** (`tools::blueprint::compile_in`): a zone with a generated room's id replaces it, a new id is
  appended, so the generated portals keep their rooms; every other `scene` key, `spawns` and `camera` included, replaces the generated
  value, and SPEC says so.
- **`_`-prefixed variables are internal**: `ui::rules::hud_layout` skips them, so a game's timers and phase flags stay off the screen
  without a HUD block.
- **`info <id>` says `loose: yes|no (why)`** with the prop's mass at `PROP_DENSITY` and whether a human can carry it
  (`physics::why_not_loose` mirrors `classify`: pinned, animated, a fixture prop, a non-floor prefab, too big, not a prop).
- **`checks.lint.ignore: [codes]`** drops those findings before the budgets and `forbid` are applied and says how many it skipped.
- **Measured facts become text**: `describe rules` and SPEC state that a body walking from one volume into the next gets the next
  volume's `enter` before the previous one's `exit`, that starting inside a volume fires no `enter`, and that `_` vars are hidden;
  `describe physics` gains `topple_direction` (front = local +Z, a struck prop falls toward local -Z, yaw Y falls toward
  (-sin Y, -cos Y), Domino Halls' spacing and corner recipe); `describe sim` notes the walker's forward-only steering and 0.25 m waypoint
  tolerance; the JSON form of `describe rules` lists the prop triggers, built-in functions and engine events.

## Consequences
- Domino Halls' `author.py` aim and corner recipes, Knockdown Alley's hidden `pit`/`loaded` vars and `lint_ignore` workarounds, and
  Fling Delivery's `t_start`/`started` HUD noise are answered in data.
- Left for later (each needs a design of its own): a failed pickup saying why (`no prop in reach`, `ray missed`), `interact: {target}`,
  a `chain` placement command, `weapons.bat.impulse`, persistent vars and text labels, `sim --sweep`, and lint treating props smaller
  than a body as not-floor for `drop`.
