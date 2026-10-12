# 2026-10-12. 2D platformers are data: contact sides, become, levels, and a solver that proves a level
Status: accepted
Summary: Side-scrolling platformers (Honeybound) are written in the 2D game format like every other 2D game: a few general verbs were added (contact sides, hit, become, walk and orbit movers, platformer feel, levels, a forward camera), and a beam search over the real simulation proves each level can be finished.

## Context
A request for a game "as deep as Super Mario Bros., with its own characters" (Honeybound, `examples/2d/honeybound/`) needed what the 2D format could not say:
whether Tobin landed on an enemy or ran into it, whether a block was hit from below, a thing changing into another kind (small to big, an enemy to its shell,
a block to a spent block), walkers that turn at walls, running and variable-height jumps, eight levels with their own music, a camera that never scrolls back,
and enemies that wait until they are on screen. Writing the game as a Rust client (ADR 0043) would have put its rules out of reach of `verify`, `sim` and the
other tools, and the release pipeline ships only `re2d` plus data. Every 2D level so far was proven by a hand-written script; a platformer level is long and its
enemies move, so a fixed script breaks with every change.

The first build also showed the simulation spending 92% of a tick comparing tag names (`pairs_for`, `refresh_vars`): a level is about a thousand tiles.

## Decision
General verbs in the format (`describe 2d` lists them), each a small piece of `red2d::sim`:
- a rule `side` on `touch`/`touching`/`hit` (`Sim::side_of`: judged from where both things were before the tick's move) and the `hit` trigger (a mover stopped by
  a solid: `Sim::integrate` records each contact with the mover's side);
- `become` (same thing, same scene id, a new prefab; its feet stay put), `walk` (`edges: "turn"`) and `orbit` movers, the platformer `Feel` (run, accel, decel,
  cut, run_jump, coyote, buffer; the defaults are the old mover exactly), `asleep`, rider carrying on moving platforms (`Sim::carry`);
- `levels` with the `level` action (immediate, so later actions of the same list see the new world), per-level world size, background and music, the `music`
  track and `"level"` commands (the native player renders tracks on a background thread), `camera.forward_only`, and built-ins for the running rule's two things
  (`self_x` ...) and held keys (`held_down` ...); a `press` rule's `self` is the thing the keys move;
- `red_engine2 solve`: a beam search (`red2d::solve::solve`) over the keys a player can hold each tenth of a second, every candidate a clone of the real `Sim`,
  dropping states that meet `--avoid`; the answer is printed as `hold`/`wait` steps that replay exactly as a scenario (consecutive holds never release a key for
  a tick). Honeybound's `dev.py solve` stores the runs and its build puts them into the per-stage scenarios, so `verify` proves every stage can be finished.

Speed, with no change in behaviour (every scenario hash identical before and after): tags are indices per prefab, contact pairs come from per-tag lists sorted
by x (the same pairs in the same order), tag counts are refreshed only when something was added, removed or changed kind, and things that never move by
themselves sit in a grid for collision tests (`SolidGrid`; the lowest index that overlaps is the answer, as before). A 1000-tile level went from about 0.5 ms
to about 15 microseconds a tick.

## Consequences
A platformer is a game file and a builder script; its rules stay inspectable and provable. A level change needs a new run (`dev.py solve`, about 10 s a level on
four cores); `verify` fails loudly when an old run no longer fits. The solver searches inputs, not the rules: a goal it cannot reach in `--max-seconds` with
`--beam` candidates says so, which is evidence, not proof, that a level is impossible. Undo: the verbs are additive and default to the old behaviour; `solve`
is a separate command.
