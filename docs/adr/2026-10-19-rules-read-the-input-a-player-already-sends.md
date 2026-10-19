# 2026-10-19. Rules read the input a player already sends
Status: accepted
Summary: A rule can fire when a player presses or releases a key and can read whether it is held, from the `PlayerInput` the server already receives; rules games send the right mouse as a free key, and 2D gets a change-of-speed built-in. No protocol change.

## Context
Idea Forge's finding `no-rule-readable-held-input-or-breath` was filed by five runs, including three of the last four:

| Run | What the card needed | What the agent did instead | Severity |
| --- | --- | --- | --- |
| Borrowed Foundry (3D, 2026-10-11) | Hold your breath to stay in another body; swing while holding | A jar picked up stood for the breath. Carrying it blocked the bat swing, so the design had to change | 3 |
| Velvet Orchestra (3D, 2026-10-12) | Swing at the radio to tune it | `pickup` of a can, then `reset` (fixed since: `pickup`, `drop` and `prop_hit` carry their prop) | 2 |
| Restless Tide (3D, 2026-10-17) | Swap bodies for as long as you hold your breath | `pickup` is the swap and `drop` lets go; the breath itself is gone | 2 |
| Copper Choir (2D, 2026-10-18) | A guest who panics at sudden moves | Each dial turn counts as a jolt: rules see `self_vx` now but not how fast it changed | 1 |
| Crooked Ferry (3D, 2026-10-18) | Whistle to call an echo | The whistle became the bat swing with empty hands | 3 |

The 3D rules see where players are (zones, `enter`), props and their events, and the bat's `swing`. They see no key. Yet every tick the server
already receives each player's `PlayerInput`: `jump`, `sprint`, `crouch`, `interact`, `attack`, `reload`, `switch_weapon`, plus `aim` and `drop`,
which only the loadout shooter uses. `MatchSim::run_rules` builds a `RulePlayer` (slot, feet, body, character, team) and drops the input.
2D already has `press` triggers and `held_*` values; what Copper Choir lacked is the change in speed.

## Decision
1. **`RulePlayer` carries the player's input this tick.** `MatchSim::run_rules` copies the held bits of the input it just simulated. The rules
   engine keeps last tick's bits per slot to see presses and releases. Online the rules run on the server, which has the input, and
   offline they run in the same `MatchSim`. So: **no protocol change, no new client message**, and replays work, because traces already
   record inputs.
2. **A trigger and an expression**, named after the keys a player knows:
   - `"when": {"input": "crouch"}` fires on the press and `{"input": "crouch", "release": true}` on the release. `me` is that player, as for `enter`.
   - `input(crouch)` is 1 while the rule's player (`me`) holds it.

   The names are `jump`, `crouch`, `sprint`, `interact`, `attack` and `alt`. `alt` is new to rules games: the right mouse button and the left
   trigger. The client sends it as `aim`, which it hard-wired to `false` (`build_input`). Built: it sends the key's state only in a scene whose
   rules read `alt` (`rule_input::reads`), because the server's loadout kit aims with `aim`, and that must not change. `attack` fires even while
   carrying, so a press is not lost because the bat cannot swing. Offline, E and the click act at once instead of travelling as input, so the
   client hands those presses to its rules too.

   "For as long as you hold" is a press starting a timer and a release cancelling it, with the timers from ADR
   2026-10-17-rule-timers-and-prop-variables-are-server-state. No held-seconds built-in is needed.
3. **2D: `self_ax` and `self_ay`**, the change in the running rule's thing's velocity over the last tick, per second, next to `self_vx` and
   `self_vy`. "Sudden moves" becomes `abs(self_ax) > 400`.
4. `checks.sim` scripts can already hold `crouch`, `sprint`, `interact` and `attack` (`SimHold`). They gain `alt`, so every input a rule reads
   can be proved headless. `describe rules` and `describe 2d` list the names, and `validate` names the right one for a misspelt input.

How that covers the runs:
- Breath (Borrowed Foundry, Restless Tide): `{"input": "alt"}` plus a timer, then `release`.
- The whistle (Crooked Ferry): `{"input": "alt"}`, with no weapon in hand.
- Swing while carrying (Borrowed Foundry): `{"input": "attack"}`.
- The guest's panic (Copper Choir): `self_ax`.
- Velvet Orchestra is already covered.

Not in this decision:
- **Looking at an object.** Borrowed Foundry and Restless Tide asked for it; `pickup` with `"prop"` already covers "the thing you point at" for
  props.
- **The player's coordinates in expressions.** Zones answer "where". `y(me)` is a separate small step if a run needs it.
- **A new key or wire bit.**

## Consequences
- **Built (owner's OK, 2026-10-18):** 404 lines added outside the generated schemas, about 160 of them tests; estimated 150 of code. The offline client, which runs its own rules, was the part the estimate missed. The 3D side is the bits on `RulePlayer`, edges in `RulesEngine`, the trigger and `input()` in the rule parser and its typed definition and schema, and `alt`
  in `SimHold`. The `checks.sim` harness's `aim` flag was dead (not in `HOLD_KEYS`) and became `alt`. The 2D side is two
  built-ins in red2d. Server cost: a byte of input bits per player per tick, kept for one tick.
- **The checksum gains nothing for a scene without input rules.** A scene with them hashes its rule state as now, and inputs are already in
  traces.
- **A key the game gives a meaning to must be named in its controls** (wiki, HUD, start card). In the loadout shooter `alt` stays aiming:
  rules there may read it, but it still aims.
- **Undo:** drop the trigger and the expression. The client's `alt` bit is harmless on its own.
