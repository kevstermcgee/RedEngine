# Rules-language gaps from the Idea Forge notes: ranked, with a design for each (2026-10-09)

> A proposal, not documentation of what exists: it lives in `docs/`, not in `docs/analysis/`, on purpose. Analysis notes are embedded in `search`, and a design for rules that do not exist yet ranked above the real reference for a question about movement rules (it broke the `ai_tasks` context budget). For what the rules language does today use `describe rules`.

Written down, **not built** (the hardening pass said so: "not this pass"). The evidence is the Idea Forge backlog (37 issues from seven runs, `docs/analysis/idea-forge/`); the designs are mine and
are proposals to be argued with. Three gaps recur in games the agents could not express and worked around with generated code: per-entity state and timers, rules that change how the player
moves, and a fall-out height. Everything below is about the **3D scene rules** (`src/sim/rules*.rs`, `describe rules`) unless it says 2D (`crates/red2d`, `describe 2d`).

## Ranking

| rank | gap | backlog keys (severity, runs, minutes) | what it cost in the games | size of the change |
|---|---|---|---|---|
| 1 | per-entity state and timers | `no-per-entity-timers` (3, 4 runs, 23 min), `no-entity-origin-memory` (2, 1, 5) | 252 generated rules, 102,527 bytes for a 5x5 board (`gentle-carousel`); 55 rules for five parts (`patient-loom`); two more runs hit the same hole as a timer that cannot be restarted (`every` fires forever, `after` once) and no trigger when a `ttl` expires | a parse-time expansion (no protocol change) then one runtime feature (a protocol change) |
| 2 | a fall-out height | `implicit-ground-plane-hides-voids` (3, 1, 8), `no-void-or-kill-floor` (2, 1, 4), `lint-leak-no-intentional-void` (1, 1, 4) | a whole genre (floating tiles, pits, islands) is inexpressible: after a floor tile is deactivated the player stands on the implicit `y = 0` plane; `lint` calls an intended edge a leak | small: one trigger mirroring `prop_below`, one scene field, one lint rule |
| 3 | rules that change how the player moves | `no-rule-gated-player-movement` (3, 2, 10), `no-runtime-scale-or-camera-input-for-rules` (3, 2, 13), `no-rule-action-sets-player-gravity` (2, 1, 2) | three cards dropped or faked ("move only while a partner moves", "pinch to scale the room", "gravity grows per item") | the most invasive: prediction and the snapshot must carry the values (protocol change) |

Rank is impact first. Build order is by risk: 2 is cheap and unblocks a genre, 1 has a free first step, 3 touches the wire and client prediction and should go last.

## 1. Per-entity state and timers

**The gap.** Rules see global variables, `player_vars` (one copy per player) and a few built-ins on loose props. Anything that is "per tile" has to be a variable per tile and a copy of every
rule per tile; a timer is `every`/`after`, global and never restartable, so a per-tile age, break and reveal cycle is a hand-built state machine per tile (`near_k = time` variables to remember when
something was touched). **Design.** Two steps. First, *groups and templates* as parse-time sugar, like `wall` and `fence`: `"groups": {"tiles": {"ids": ["tile_*"]}}` names a set of objects (or prop ids) and a rule
with `"for_each": "tiles"` is expanded when the scene loads into one rule per member, with `$it` replaced by the member's id and `me`-style names `$it.age` becoming the variable `tiles.<id>.age` declared
from a `"for_each_vars"` template. The 252 rules become about six templates, the replicated state and the checksum are exactly what they are today (the expansion is the old rules), and `validate`/`sim`
report errors against the template with the member that failed. Second, once that is in use, *per-entity variables and a restartable timer as real features*: `"entity_vars": {"tile": {"age": 0}}`
read and written as `tile.age` with `touched` (the entity that caused the trigger) as the implicit subject, and an action `{"timer": ["name", secs]}` with `when: {"timer": "name"}` scoped to
that entity. These live in the rule state, so they enter the match checksum and the snapshot: the snapshot's bounded variable list (`MAX_RULE_VARS = 16`, `MAX_RULE_HIDDEN`) needs either an entity section or a rule that
entity state is not replicated and clients derive what they show from events, and either choice is a `PROTOCOL_VERSION` bump to be argued for in its own ADR.
2D already has per-entity lifetime (`ttl` on a prefab) and `destroy`/`spawn`, so the 2D half of this gap is the timer action and entity vars, not the expansion.
**How we would measure it.** The same board rebuilt on the new rules: rule count (252 now), file bytes (102,527 now), `validate` and `verify` seconds, and the agent's minutes on the backlog key.

## 2. A fall-out height

**The gap.** The ground is implicit (`ground_height_at` falls back to `y = 0` where no box, stair or terrain is under the player), so deactivating a floor leaves the player standing on nothing
visible, no rule can see them fall, and `lint` reports the edge of an intended void as a leak. **Design.** A trigger `{"player_below": y}` (and `{"player_below": {"zone": id}}` for a per-zone floor), the
mirror of the existing `prop_below: [prop, y]`, fires once as a player's feet cross below `y` and reuses the rule machinery unchanged (`teleport` to a spawn, `add` to a counter, `end`). With it,
a scene field `"world": {"ground": "none"}` removes the implicit plane for that scene: the player falls under gravity, `ground_height_at` returns negative infinity (so the player keeps falling until a rule
acts), and `lint`'s leak check treats a `ground: none` border as intended (the backlog's `lint-leak-no-intentional-void`). The first step needs no new scene field: documenting the implicit ground in
`describe scene` and `describe rules` and adding the trigger already lets a pit be a floor box with a hole plus a `player_below: -1` rule. Rules state is unchanged, so no protocol change; the client
predicts the fall with the movement code it already has. **How we would measure it.** A pit-and-falling test game (`checks.sim` walks off an edge and expects the respawn event), `lint` clean
with an intentional void, and the three backlog keys closed in `fixes.json`.

## 3. Rules that change how the player moves

**The gap.** `player.walk_speed`, `jump_speed`, `gravity` and the body size are static scene fields (`schema.rs` validates them with `ranged`) and nothing a rule does can change them: "move
only while a partner moves" is a clock variable plus a penalty, "gravity grows per item" was cut, "pinch to shrink the room" has no input or action at all. **Design.** One action, `{"player": {"walk_speed":
EXPR, "jump_speed": EXPR, "gravity": EXPR, "scale": EXPR, "move": BOOL}}`, on the player that triggered the rule (`me`) or a named `who`, writing a per-player *override* of the scene's tuning that
`sim::player` already reads through its `tuning` struct (`tuning.walk_speed`, `tuning.sprint_speed`, ...) and that resets when the player rejoins like `player_vars`. A literal is checked against the
same range the scene field has (`gravity` 1..40) at load; an expression is clamped to that range at run time and the clamp is **counted and shown in the rule-fire diagnostics** rather than hidden (the
opposite of the silent clamps the hardening pass removed). `"move": false` freezes movement input while looking and rules still work; `scale` changes the collision body and eye height together.
Because the client predicts movement with the same code, the override must reach it: the snapshot carries the per-player values that differ from the scene's (a few bytes, only when overridden), which is a
`PROTOCOL_VERSION` bump and a replay-format change, and `tests/net_budget.rs` bounds the cost. Camera input and loudness as inputs to rules (`no-rule-input-actions`, `no-loudness-input-for-rules`)
are a separate, smaller feature that this one makes more useful and should not be bundled with it. **How we would measure it.** The three dropped cards rebuilt as games, the snapshot's size with and
without an override (`net_budget`), and prediction error in a `net-test --profile bad` run with the override changing every second.

## What was not looked at

Random numbers and string tables (`no-random-expression`, `no-string-tables`) are format gaps in the same family and were out of the three the brief named; they rank above rank 3 on minutes lost
(40 and 30) and belong in the next pass. The existing silent-default bugs the same notes report (`negative-pad-ignored`, `sim-hold-accepts-unknown-values-silently`) are fixed in the hardening pass, item 7.
