# Idea Forge run: Phantom Harvest (2026-10-10)

- Idea code: `8043356467`; kind 2D; slug `phantom-harvest`; game `examples/2d/phantom-harvest.game2d.json`
- Engine revision: `9c62ea7b3b03`; model: `default`
- The card: YOU stretch a rope between two drifting anchors and play it like a slingshot, BUT up is whichever way you last fell. Pushback: A storm front sweeps across the map and scatters your tools if it catches them. Goal: Leave the level exactly as you found it.

## The game

**Mechanic:** you grab a rope stretched between two drifting anchors and fire yourself along its normal like a slingshot, and "up" is always away from the wall you last landed on, so every shot re-defines which way you fall next.
**Loop (every few seconds):** wait for the rope's tilt to point where you want to go (the anchors drift), press Space to grab (the pouch pulls back as power charges, a dotted arc previews the shot), press Space to release, fly, land on a wall; gravity now points at that wall. Along the way you touch tools to carry one, and touch a marked slot to put it back. **Win:** both tools back in their marked slots (leave the level as found). **Lose:** the storm front scatters placed or carried tools 5 times, or 150 s pass. Best time is saved (`persist`).
**Adapted/cut:** the card's "leave the level exactly as you found it" became restoring 2 tools to slots (a 3-tool version was playable but my bot could not find a winning script in time); "scatters your tools" is a drifting storm band that destroys placed tools (they respawn as loose tools at random spots) and strips the carried one; the story is dropped except the title. Rope is drawn as dots, physics is hand-integrated in rules (no gravity/joint primitive). Not proven: that it feels good in motion; the sim only proves the shots land and the win/lose paths run.

## Score at the time of writing

- pass: the game file exists (examples/2d/phantom-harvest.game2d.json)
- pass: validate passes (OK: 2D game `phantom-harvest` (revision 45f60af180944cc7))
- pass: verify passes (22 passed, 0 failed in 0.5 s)
- pass: at least three scenarios (4 scenario(s))
- pass: one scenario is marked smoke
- pass: a one-sentence description (Slingshot yourself off a rope between two drifting anchors, where up is whicheve)
- pass: a sound or music
- pass: something is saved between runs

## Findings

Ranked: what would save the next run the most first. `severity` 1 = papercut, 2 = cost real time, 3 = blocked this run or would block most runs. `proposal` is an engine change, not a workaround.

```json findings
[
 {
  "id": "F1",
  "key": "no-runtime-gravity-or-joints",
  "area": "idea-fit",
  "title": "No runtime gravity direction / rope/spring joint: 'up is whichever way you last fell' emulated with own vars g",
  "severity": 3,
  "cost_min": 15.0,
  "evidence": "The core of the card (up = last fall direction; rope slingshot) has no engine primitive: body.gravity is fixed per prefab and there is no joint. `grep` of describe 2d shows no gravity action.",
  "workaround": "Own vars gx,gy; an `every` rule integrates velocity and sets it with the `velocity` action; 4 landing rules flip gravity; rope = 8 teleported dots, aim preview = 5 more: about 45 of the 63 rules are plumbing.",
  "proposal": "per-body gravity vector settable from rules (`gravity:[x,y]` action), a rope/distance-joint prefab and a line shape"
 },
 {
  "id": "F2",
  "key": "sim-no-clock-sync",
  "area": "verify",
  "title": "Wrote a python beam-search bot (~100 sim runs per stage, each ~20ms) to find a deterministic winning script fo",
  "severity": 2,
  "cost_min": 20.0,
  "evidence": "No way to find a winning script: I wrote /tmp beam-search (~3000 sim runs of 20 ms) to find press/wait timings for `tour-win`; stages 5-6 of a 3-tool version were never found, so I cut to 2 tools.",
  "workaround": "Python beam search over (wait, hold) pairs reading vars from `sim --every 1000` output.",
  "proposal": "a `solve`/`fuzz` command that searches press/wait scripts for an `expect`, and a way to feed `--scenario` to 2D"
 },
 {
  "id": "F3",
  "key": "no-release-or-held-trigger",
  "area": "idea-fit",
  "title": "No key-release / key-held trigger: only press:action, so slingshot 'pull and release' needed a two-press grab/",
  "severity": 2,
  "cost_min": 6.0,
  "evidence": "Only `press:action` exists; a rope pull-and-release needs press/release.",
  "workaround": "Two presses (grab, then fire) with a `did` flag so the rule order does not fire both on one press.",
  "proposal": "release:/held: triggers or an input_x/input_y builtin"
 },
 {
  "id": "F4",
  "key": "every-trigger-tick-quantized",
  "area": "docs",
  "title": "every:0.02 fires on tick boundaries (60Hz => every 2nd tick, 0.033s), so my physics integration ran at 300/s2 ",
  "severity": 2,
  "cost_min": 5.0,
  "evidence": "`every:0.02` ran every 2nd tick at 60 Hz: velocity changed 10 per 0.033 s, gravity looked like 300 not 500 in `sim --every 0.05`.",
  "workaround": "Used every:0.016 and dt=1/60 in the formulas.",
  "proposal": "document tick quantization; add an every-tick trigger and a `dt` builtin"
 },
 {
  "id": "F5",
  "key": "cold-worktree-build-and-preflight-rebuild",
  "area": "tooling",
  "title": "scripts/dev start said executable MISSING and 'unrouted' without --workflow; seed defaulted to nonexistent mai",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "`scripts/dev start` printed 'executable MISSING', 'unrouted: no workflow cue matched'; `scripts/dev seed` said no donor (main checkout target does not exist) though 4 sibling worktrees have target/. Build then took 201 s.",
  "workaround": "Ran scripts/dev build in the background.",
  "proposal": "seed from any sibling target automatically; route 'game' tasks to game-create"
 },
 {
  "id": "F6",
  "key": "describe-2d-action-args-expressions",
  "area": "docs",
  "title": "read crates/red2d/src/game.rs (teleport/velocity arg parsing) because describe 2d does not say whether to/v ac",
  "severity": 1,
  "cost_min": 2.0,
  "evidence": "describe 2d does not say whether teleport.to / velocity.v / spawn.at take expressions (they do).",
  "workaround": "Read crates/red2d/src/game.rs `fn place` and the velocity arm.",
  "proposal": "state which action args take expressions in describe 2d"
 },
 {
  "id": "F7",
  "key": "no-random-expression",
  "area": "format",
  "title": "No sqrt/abs/min/max/sign in expressions: rope normal sign and charge clamp done with (a>0)*2-1 and charge*(cha",
  "severity": 1,
  "cost_min": 4.0,
  "evidence": "Expressions lack min/max/abs/sqrt: used `(a>0)*2-1` for sign and `charge*(charge<1)+(charge>=1)` for clamp; comparisons yielding 1/0 are undocumented but work. Random placement via {x:[..],y:[..]} ranges worked.",
  "workaround": "Boolean arithmetic.",
  "proposal": "add min,max,abs,sqrt,rand and document 0/1 comparisons"
 },
 {
  "id": "F8",
  "key": "sim-scenario-flag-2d-ignored",
  "area": "cli",
  "title": "sim --scenario FILE is rejected for 2D ('no scenario in game (add checks.scenarios)'), so my bot search had to",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "`sim G --scenario f.json` -> 'no scenario in examples/2d/... (add `checks.scenarios`)'.",
  "workaround": "Rewrote a temp copy of the game per candidate script.",
  "proposal": "honour --scenario for 2D"
 },
 {
  "id": "F9",
  "key": "wait-until-after-ended",
  "area": "diagnostics",
  "title": "Win scenario script ending in wait_until state==0 failed ('waited 6 s and never held') because the game had al",
  "severity": 1,
  "cost_min": 2.0,
  "evidence": "scenario `tour-win`: 'FAILED: waited 6 s and it never held: expected state eq 0, found 2' although the game had ended in a win mid-flight.",
  "workaround": "Ended the script on wait_until {ended:win}.",
  "proposal": "name the end when a wait_until times out after the game ended"
 },
 {
  "id": "F10",
  "key": "worked-describe-2d-validate",
  "area": "worked",
  "title": "validate + frame worked first time with good messages; describe 2d single screen was enough to write a 40-rule",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "validate passed the 63-rule file first try; frame showed rope, storm and sprites correctly; verify ran 22 checks in 0.5 s.",
  "workaround": "",
  "proposal": "keep"
 }
]
```

## Idea fit

YOU+BUT were expressible only by rebuilding physics in rules: no runtime gravity direction, no rope/joint, no press/release input, expressions without min/max/abs. It works because `velocity`, `teleport` with expressions and `every` rules exist, but it is ~45 rules of plumbing and ticks are quantized. The storm/scatter pushback and the restore goal fit the rule/touch/spawn model well. Missing capability ranking: settable gravity vector, release trigger, joints/lines, expression helpers.

## What worked

describe 2d on one screen; validate/frame/verify fast and accurate (22 checks in 0.5 s); `sim --every` made the tick-quantization bug visible; deterministic sim made a brute-force bot search practical (20 ms per run); spawn/teleport range places gave randomness; play2d under xvfb-run --max-ticks 120 ran clean.

## Automatic measurements

<!-- automatic-measurements:begin -->
Friction report (`scripts/agent_bench.py summary`):

```json
{
 "commands": 106577,
 "failed": 201,
 "documentation_topics_read": [
  "2d"
 ],
 "searches": [
  "change gravity direction at runtime",
  "rope or spring between two entities",
  "apply impulse velocity to player from rule",
  "move entity position from rule or set velocity of drifting anchor",
  "touch wall sets variable which side"
 ],
 "cli_source_exploration": 0,
 "retries_of_a_failed_command_without_a_success_between": 184,
 "repair_cycles": 16,
 "first_failures": [
  {
   "command": "sim /home/kevin/workspace/RedEngine-factory-idea-2d-phantom-harvest/examples/2d/phantom-harvest.game2d.json --scenario /tmp/ph/s.json",
   "error": "no scenario in /home/kevin/workspace/RedEngine-factory-idea-2d-phantom-harvest/examples/2d/phantom-harvest.game2d.json (add `checks.scenarios`)"
  },
  {
   "command": "sim /home/kevin/workspace/RedEngine-factory-idea-2d-phantom-harvest/examples/2d/phantom-harvest.game2d.json --scenario /tmp/ph/s.json",
   "error": "no scenario in /home/kevin/workspace/RedEngine-factory-idea-2d-phantom-harvest/examples/2d/phantom-harvest.game2d.json (add `checks.scenarios`)"
  },
  {
   "command": "sim examples/2d/phantom-harvest.game2d.json --scenario /tmp/ph/s.json",
   "error": "no scenario in examples/2d/phantom-harvest.game2d.json (add `checks.scenarios`)"
  },
  {
   "command": "sim /tmp/ph/g.game2d.json --every 1000",
   "error": "FAIL  scenario `s`: 1219 ticks (20.3 s), ended win, hash bc099e25de63ef1b"
  },
  {
   "command": "sim /tmp/ph/g.game2d.json --every 1000",
   "error": "FAIL  scenario `s`: 1216 ticks (20.3 s), ended win, hash 26ebc9d00d993cb8"
  }
 ],
 "used_verify": true,
 "minutes": 61.1,
 "transcript": {
  "engine_files_read_with_tools": 0,
  "engine_files_read_with_the_shell": 3,
  "examples": [
   "grep -rn \"teleport\" crates/red2d/src/*.rs | head -15; grep -rln \"teleport\\|velocity\" examples/2d | head",
   "sed -n 1495,1620p crates/red2d/src/game.rs; python3 /home/kevin/workspace/RedEngine-factory/scripts/idea_forge.py note \"",
   "grep -n \"fn place\" -A40 crates/red2d/src/game.rs | head -70; grep -n \"ttl\\|\\\"hold\\|down:\" crates/red2d/src/reference.rs "
  ],
  "edits": 0
 }
}
```

Agent session: 83 assistant turns; tools {"Bash": 40, "Monitor": 1, "Read": 3}; cost $1.36; wall 66.2 min.

Notes logged during the run: 11 (60 minutes claimed lost). Measured wall time 66.2 min.
<!-- automatic-measurements:end -->
