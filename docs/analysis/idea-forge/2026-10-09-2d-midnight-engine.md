# Idea Forge run: Midnight Engine (2026-10-09)

- Idea code: `1073234956`; kind 2D; slug `midnight-engine`; game `examples/2d/midnight-engine.game2d.json`
- Engine revision: `413902f2b8df`; model: `default`
- The card: YOU conduct a crowd of tiny followers with the tempo of a baton, BUT every move costs a coin and you earn coins only by standing still. Pushback: Your tools multiply when unused and swarm you if you hoard them. Goal: Build one unbroken chain from one end of the world to the other.

## The game

**Mechanic:** every beat of the baton (Space) spends a coin to add one follower to a chain, and coins come only from not beating for a second, so the player lives in the tension between waiting and conducting.
Core loop: wait (the idle bar fills, a coin arrives each second), beat when affordable (1 coin; 2 if rushed within 0.5 s of the last beat), watch the coin pile. Hoarded coins multiply (x1.5 every 2 s of idleness) and from 6 coins a pest swarm spawns every second and chases the cat; each bite costs a life. Win: the chain of 29 followers reaches the flag at the far end. Lose: 3 bites.
Adapted: "every move" became every beat because the cat stays put (movement cannot be gated by rules); "tools" are the coins; "unbroken chain" is just a chain reaching the flag (no breaking); the small-god story is dressing only.

## Score at the time of writing

- pass: the game file exists (examples/2d/midnight-engine.game2d.json)
- pass: validate passes (OK: 2D game `midnight-engine` (revision 94ec7a639bc3a3d3))
- pass: verify passes (19 passed, 0 failed in 0.0 s)
- pass: at least three scenarios (4 scenario(s))
- pass: one scenario is marked smoke
- pass: a one-sentence description (A stray cat conducts a worshipping crowd into one unbroken chain across the alle)
- pass: a sound or music
- pass: something is saved between runs

## Findings

Ranked: what would save the next run the most first. `severity` 1 = papercut, 2 = cost real time, 3 = blocked this run or would block most runs. `proposal` is an engine change, not a workaround.

```json findings
[
 {
  "id": "F1",
  "key": "cold-worktree-build-and-preflight-rebuild",
  "area": "tooling",
  "title": "scripts/dev start + describe 2d ran as one command: cold build (no seed from sibling target) blew 120s tool ti",
  "severity": 2,
  "cost_min": 14.0,
  "evidence": "first `scripts/dev start` reported `executable MISSING`; the cold build took ~10+ min and hit the 120 s tool timeout",
  "workaround": "ran the build in background and waited",
  "proposal": "start should auto-seed target from a sibling build or print ETA and run build in background"
 },
 {
  "id": "F2",
  "key": "sim-script-no-repeat",
  "area": "verify",
  "title": "sim scripts have no loop/repeat: a 29-beat playthrough needs 58 generated steps (I generate them with python)",
  "severity": 1,
  "cost_min": 2.0,
  "evidence": "win scenario needs 29 press+wait pairs; checks.scenarios[3].script is 59 steps",
  "workaround": "generated the steps with python",
  "proposal": "add {repeat:n,do:[steps]} step"
 },
 {
  "id": "F3",
  "key": "ui-show-no-outcome",
  "area": "idea-fit",
  "title": "ui show only accepts 'ended': can't show a win-only vs lose-only banner (no string vars either); used neutral ",
  "severity": 1,
  "cost_min": 2.0,
  "evidence": "ui text `show` accepts only \"ended\"; a banner shows on both win and lose",
  "workaround": "neutral text THE END",
  "proposal": "ui show: 'win'|'lose'"
 },
 {
  "id": "F4",
  "key": "touch-other-order-undocumented",
  "area": "docs",
  "title": "touch:[a,b] 'other' destroyed the wrong entity: order of tags in touch decides what 'other' is, docs don't say",
  "severity": 1,
  "cost_min": 4.0,
  "evidence": "`touch:[\"pest\",\"cat\"]` + `destroy:\"other\"` destroyed the cat (frame showed no cat, lives 2); swapping to [\"cat\",\"pest\"] fixed it",
  "workaround": "swapped tag order",
  "proposal": "describe 2d: say 'other' = the second tag's entity? and warn on destroy other of a player"
 },
 {
  "id": "F5",
  "key": "no-rule-gated-player-movement",
  "area": "idea-fit",
  "title": "no-rule-gated-player-movement: avoided by making the cat stationary and the baton a discrete press; 'every mov",
  "severity": 3,
  "cost_min": 0.0,
  "evidence": "card: every move costs a coin; move.keys cannot be gated by a var",
  "workaround": "cat stationary; the baton is a discrete `press:action` beat that spends a coin and adds a chain link",
  "proposal": "gate move.keys by a var"
 },
 {
  "id": "F6",
  "key": "sim-fired-rules-report",
  "area": "worked",
  "title": "validate/verify/sim/frame loop fast and clear; failing scenario lists fired/never-fired rules; sim --every gav",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "verify failure text: `rules that never fired: bitten...; fired: tick_idle x8...` pointed straight at the bug",
  "workaround": "",
  "proposal": "keep the fired / never-fired rule report and `sim --every`"
 }
]
```

## Idea fit

Expressible: coins earned only when idle (a `idle` var reset by presses, `every` rules), multiplying hoard, swarm spawner, chain via `spawn` at expression positions. Not expressible: gating the player's own movement by coins, breaking a specific link of the chain (no per-entity state/ids for spawned entities), per-outcome banners.

## What worked

`validate`/`verify` in about a second; failing scenarios list fired and never-fired rules; `sim --every` tuning table; `frame` caught the vanished cat that the numeric checks missed; `spawn.at` accepted variable expressions; `persist` and `play2d` under xvfb worked first time.

## Automatic measurements

<!-- automatic-measurements:begin -->
Friction report (`scripts/agent_bench.py summary`):

```json
{
 "commands": 23,
 "failed": 3,
 "documentation_topics_read": [
  "2d",
  "2d"
 ],
 "searches": [
  "spawn at expression variable position"
 ],
 "cli_source_exploration": 0,
 "retries_of_a_failed_command_without_a_success_between": 1,
 "repair_cycles": 2,
 "first_failures": [
  {
   "command": "validate examples/2d/midnight-engine.game2d.json",
   "error": "2 error(s)"
  },
  {
   "command": "verify examples/2d/midnight-engine.game2d.json",
   "error": "FAIL  [simulation] scenario `rushing_costs_double`: expected links eq 1, found 2; rules that never fired: bitten, dead, won, record; fired: tick_idle x8, earn x2, multiply x1, beat_rushed x1, beat_calm x1, swarm x1 (a rule that never fired has a trigger that did not happen or an `if` that was false)"
  },
  {
   "command": "verify examples/2d/midnight-engine.game2d.json",
   "error": "FAIL  [simulation] scenario `rushing_costs_double`: expected coins lt 2, found 3; rules that never fired: bitten, dead, won, record; fired: tick_idle x8, earn x2, multiply x1, beat_rushed x1, beat_calm x1, swarm x1 (a rule that never fired has a trigger that did not happen or an `if` that was false)"
  }
 ],
 "used_verify": true,
 "minutes": 11.6,
 "transcript": {
  "engine_files_read_with_tools": 0,
  "engine_files_read_with_the_shell": 0,
  "examples": [],
  "edits": 0
 }
}
```

Agent session: 36 assistant turns; tools {"Bash": 17, "Read": 4}; cost $0.50; wall 0.0 min.

Notes logged during the run: 6 (22 minutes claimed lost). Measured wall time 22.7 min.
<!-- automatic-measurements:end -->
