# Idea Forge run: Velvet Orchestra (2026-10-12)

- Idea code: `6859751002`; kind 3D; slug `velvet-orchestra`; game `examples/3d/velvet-orchestra/velvet-orchestra.json`
- Engine revision: `94d7e76249e0`; model: `default`
- The card: YOU tune a radio dial where every frequency rewrites one rule of the room, BUT gravity points toward the last thing you touched. Pushback: Noise draws a patient listener that always arrives from a new direction. Goal: Empty the level until nothing is left but you.

## The game

**Mechanic:** you tune a radio dial (grab the teal can, E) where each frequency rewrites a rule of the room (88 gentle lean, 94 rush, 101 hush = no noise and no lean, 107 inverse), BUT gravity points toward the last glowing stone you touched, so every touch re-aims the whole hall. **Loop:** every few seconds you pick a stone, maybe turn the dial, and watch the room slide its things toward a brass slot; each slot takes only its own colour of thing (rose west, teal east, brass north), so you must re-aim gravity, wait, and re-aim again. Touching, swinging and dialling make noise; at noise 6 the Listener arrives at a new slot each time (west, east, north, west...), stands in it (anything pushed in is handed back and the lean stops) and every extra noise while he listens is a 'heard'. **Win:** all five returned ('nothing left but you'). **Lose:** heard three times. **Adapted:** the thief story is only dressing; gravity is toward one of three walls, not a free point (engine lacked a runtime gravity vector, see F2); noise decays by hand-written rules.

## Score at the time of writing

- pass: the game file exists (examples/3d/velvet-orchestra/velvet-orchestra.json)
- pass: lint passes (PASS lint  0 error(s) (max 0), 0 warning(s) (max 0))
- pass: verify passes (examples/3d/velvet-orchestra/velvet-orchestra.json: 8 check(s), 0 failed, 0.4s)
- pass: at least two scripted playthroughs (checks.sim) (7 playthrough(s))
- pass: the game logic is data (rules) (94 rule(s))
- pass: a declared ui (cards, objective)
- pass: a sound or music (audio)
- pass: a palette of its own (>= 6 roles not the default) (15 of 15 role(s); mean distance from the defaults 202)
- pass: prefabs of its own (>= 3) (5: radio_set, listener, return_slot, lode_plinth, music_stand; shared catalogue 30% of 33 placed)
- pass: its own sound files (1 file(s))
- pass: no default look kept (lint style-*) (none)

## Findings

Ranked: what would save the next run the most first. `severity` 1 = papercut, 2 = cost real time, 3 = blocked this run or would block most runs. `proposal` is an engine change, not a workaround.

```json findings
[
 {
  "id": "F2",
  "key": "no-runtime-gravity-or-joints",
  "area": "idea-fit",
  "title": "No runtime gravity direction: 'gravity toward the last thing touched' needs 45 generated impulse rules (also: `fields` are static)",
  "severity": 3,
  "cost_min": 10.0,
  "evidence": "describe rules: impulse takes a fixed dir and speed; no prop x/z built-in, so no aiming at an anchor; `fields` (SPEC 'Force fields') have no rule toggle. The card's BUT became 3 stones x 5 props x 3 dial settings = 45 `every 0.25` impulse rules gated on `pull`/`freq`; world shrunk to a hall with 3 axis-aligned exits so a fixed direction suffices.",
  "workaround": "Generated rules with a Python script (/tmp, not in repo); gravity is 'toward the W/E/N wall' not a true point. Works in sim (props slide and return).",
  "proposal": "{gravity:[x,y,z]} or {gravity_toward:object_id} as a rule action (global, per match), and/or {field_set:[id,{velocity}]} so a rule can switch a `fields` entry"
 },
 {
  "id": "F3",
  "key": "no-rule-iteration-over-props",
  "area": "format",
  "title": "Rules cannot loop over props or read a prop's x/z: every per-prop behaviour is generated rules",
  "severity": 3,
  "cost_min": 8.0,
  "evidence": "94 rules in the scene, ~85 of them are the same 3 templates repeated per prop/zone (impulse, return, bounce); the 5-prop game is already a generated file.",
  "workaround": "Wrote the scene from a Python generator, kept outside the repo; checked results with checks.sim.",
  "proposal": "rule `for_each` over a tag/prop list binding `self`, plus prop_x(id)/prop_z(id); then impulse toward an object needs no per-case rule"
 },
 {
  "id": "F1",
  "key": "cold-worktree-build-and-preflight-rebuild",
  "area": "tooling",
  "title": "Cold worktree build: `scripts/dev seed --from` copied 0 entries; red_engine2 build 3m43s inside `describe --brief`",
  "severity": 2,
  "cost_min": 5.0,
  "evidence": "seed_target: copied 0 third-party artifact entries (0.00 GB) ... 2305 already there; first `describe --brief` printed the 'building from source, about 7 minutes' line then built in 223 s (RED_TIMEOUT raised to 3000 beforehand). `scripts/dev build` (re2) took only 10 s after.",
  "workaround": "Ran describe --brief in the background and read examples meanwhile.",
  "proposal": "ship prebuilt; seed should also copy workspace crate artifacts"
 },
 {
  "id": "F4",
  "key": "no-rule-readable-held-input-or-breath",
  "area": "idea-fit",
  "title": "No way to tell which prop was struck or what is held beyond held(id) in a pickup event",
  "severity": 2,
  "cost_min": 4.0,
  "evidence": "prop_hit carries no id; the dial and lodestones are `pickup` + `if held(id)` + `reset id` (pattern copied from borrowed-foundry). Works, but a swing at the radio cannot be the dial.",
  "workaround": "Interact (E) on a carried-then-reset can instead of swinging at it.",
  "proposal": "event payload readable in rules: `event.prop`, or prop_hit with a `prop:` filter like prop_enter"
 },
 {
  "id": "F5",
  "key": "props-have-no-scale",
  "area": "format",
  "title": "type:prop has no scale; boulder (1.26 m) is too big to carry so lodestones became traffic cones",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "lint: INFO [carry] `stone_w` is loose but too big to pick up: longest side 1.26 m (limit 1.25) and volume 0.714 m^3 (limit 0.45)",
  "workaround": "Swapped to traffic_cone tinted with the palette.",
  "proposal": "allow 'scale' on type:prop or list small stone props in catalog"
 },
 {
  "id": "F6",
  "key": "undocumented-limits-found-by-failure",
  "area": "docs",
  "title": "Hard caps found only by failing validate: 16 palette roles, 400-char start text",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "validate: style.palette: 19 roles; at most 16 (6-8 is the useful range); ui.start.text: 417 characters is too long for a screen (at most 400). Each cost one edit-validate cycle (the lint then said 'scene is invalid' with the same message).",
  "workaround": "Dropped 3 custom roles (used standard roles wall/floor/trim) and cut the text.",
  "proposal": "state caps (16 roles, 400 chars) in describe style / describe ui"
 },
 {
  "id": "F7",
  "key": "playtest-needs-second-build",
  "area": "tooling",
  "title": "playtest needs the re2 client built separately",
  "severity": 1,
  "cost_min": 1.0,
  "evidence": "`playtest` -> 'cannot find the `re2` client next to this program: build both with `scripts/dev build`'. The message states the fix; `scripts/dev build` then took 10 s and playtest worked headless (EGL warning only).",
  "workaround": "ran scripts/dev build",
  "proposal": "mention the one-time `scripts/dev build` in `describe playtest`; fine otherwise"
 },
 {
  "id": "F8",
  "key": "sim-and-examples-worked",
  "area": "worked",
  "title": "Worked: borrowed-foundry example + sim scripts + verify made the mechanic fast to prove",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "`sim` ran 7 scripted playthroughs (dial, lean, hush, inverse, listener block, lose, win) in under 1 s total and caught a lean that emptied the room in 4 s and a bounce loop that lost in 5 s.",
  "workaround": "none needed",
  "proposal": "keep"
 }
]
```

## Idea fit

Expressible: the dial (pickup + reset + modulo var), the lean (repeating impulses), noise and the Listener (vars, show/hide, prop_enter blockers), win and lose. Not expressible: a true gravity direction toward an arbitrary last-touched object (no prop positions, no aimed impulse, `fields` not switchable), a listener that physically walks (no rule-driven movement of non-props), per-prop loops. Faithful subset: axis-aligned lean toward three walls.

## What worked

`recipe`/examples (borrowed-foundry pattern for pickup knobs), `sim` with look_at/interact and rich expectations (`prop moved`, `hidden`, events), `verify` in 0.4 s, `tour` and `catalog --local --sheet` pictures, `playtest` working headless on this machine, validate messages that name the cap and the fix.

## Automatic measurements

<!-- automatic-measurements:begin -->
(the CLI fills this in after the run: command trace, failures, repair cycles, documentation read, engine source read)
<!-- automatic-measurements:end -->
