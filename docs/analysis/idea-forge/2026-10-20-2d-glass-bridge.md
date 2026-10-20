# Idea Forge run: Glass Bridge (2026-10-20)

- Idea code: `4054167824`; kind 2D; slug `glass-bridge`; game `examples/2d/glass-bridge.game2d.json`
- Engine revision: `20e12f4a7b7e`; model: `default`
- The card: YOU pluck strings between walls to launch yourself like an arrow, BUT light bends around the biggest thing on screen. Pushback: Echoes of your earlier attempts haunt the level and block your old routes. Goal: Make the loudest sound the level is able to make.

## The game

The mechanic: you launch yourself like an arrow with the arrow keys, flying until a hedge stops you, and every glass string you cross rings for 9 seconds. The loop (every few seconds): pick a direction, fly, watch which strings rang and where you stopped. It is fun because it is a sliding puzzle that makes music, with a big chord and shake as the reward. A first-time player is taught by a safe corridor where one arrow press rings three strings, then one new rule per level (stopping at hedges, planning shots, red sparks). Winning means all strings ringing at once; sparks reset you, there is no hard fail. Cut: the light-bending BUT and the echo pushback (no way to record earlier attempts).

Stranger pass: I changed the HUD so the goal is on screen, shortened hints to one line, and added the title overlay and a visible `LEVEL n of 5`.

## Score at the time of writing

- pass: the game file exists (examples/2d/glass-bridge.game2d.json)
- pass: validate passes (OK: 2D game `glass-bridge` (revision 9c10679348a27ea5))
- pass: verify passes (34 passed, 0 failed in 0.2 s)
- pass: at least three scenarios (7 scenario(s))
- pass: one scenario is marked smoke
- pass: a one-sentence description (A robot gardener launches itself like an arrow through glass strings in an empty)
- pass: a sound or music
- pass: something is saved between runs
- pass: its generator rewrites the game byte for byte (build.py 3.1 s)

## Findings

Ranked: what would save the next run the most first. `severity` 1 = papercut, 2 = cost real time, 3 = blocked this run or would block most runs. `proposal` is an engine change, not a workaround.

```json findings
[
 {
  "id": "F1",
  "key": "start-needs-workflow-flag",
  "area": "cli",
  "title": "scripts/dev start without --workflow ends 'BLOCKER unrouted' for a plain game idea; had to rerun with --workfl",
  "severity": 1,
  "cost_min": 1.0,
  "evidence": "`scripts/dev start \"...\"` printed `BLOCKER unrouted: no workflow cue matched`",
  "workaround": "reran with `--workflow game-create`",
  "proposal": "default an unmatched task containing 'game'/'2D' to game-create"
 },
 {
  "id": "F2",
  "key": "text-widget-no-expression",
  "area": "format",
  "title": "Builtin 'level' is 0-based index and text widgets cannot do arithmetic ({level+1} not allowed); needed an ever",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "HUD showed `LEVEL 3 of 5` on the fourth level; text placeholders accept only a variable",
  "workaround": "an every-0.05s rule sets `lvn = level + 1`",
  "proposal": "allow {expr} in text placeholders, or document that level is 0-based in describe 2d"
 },
 {
  "id": "F3",
  "key": "sound-dc-offset-hint",
  "area": "verify",
  "title": "verify FAILED sounds thunk/zap with DC offset from a plain decaying sine/sweep; fixed with attack/release + hp",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "`verify`: `FAIL [audio-waveform] sound thunk: DC offset -0.044`",
  "workaround": "added attack/release and highpass noise, then it passed",
  "proposal": "waveform FAIL line should suggest adding attack/release or highpass"
 },
 {
  "id": "F4",
  "key": "no-attempt-history-entities",
  "area": "idea-fit",
  "title": "Card parts BUT (light bends around biggest thing) and pushback (echoes of earlier attempts block routes) dropp",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "the card's BUT (light bends) and echo pushback have no rule-visible counterpart: rules cannot record or replay a past run",
  "workaround": "dropped both; kept the pluck-launch and the loudest-sound goal",
  "proposal": "a 'ghost replay' of past input tracks, or spawn-at-recorded-path action"
 },
 {
  "id": "F5",
  "key": "ttl-and-count-tags-worked",
  "area": "worked",
  "title": "ttl prefab + count_<tag> builtin made 'string rings for 9s, win when count_ring>=count_string' a 3-rule mechan",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "ring prefab with `ttl` plus `count_ring`/`count_string` gave the whole win test in one rule",
  "workaround": "none needed",
  "proposal": "keep as is"
 },
 {
  "id": "F6",
  "key": "validate-messages-exact",
  "area": "worked",
  "title": "touch/hit/velocity/teleport with computed expressions, sound DSL and persist all worked first try; validate er",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "validate said `no tag sv0 \u2014 did you mean s0? (known: ...)`: fixed in one edit",
  "workaround": "none needed",
  "proposal": "keep as is"
 }
]
```

## Idea fit

Expressed: plucking (strings as sensors), the launch (velocity on press), the loudest sound (a layered chord when every string rings). Not expressed: light bending around the biggest object and haunting echoes of earlier attempts: rules cannot store or replay earlier attempts, nor change geometry from what is on screen. Missing: a ghost-replay of recorded input.

## What worked

`validate` messages with did-you-mean, `verify` in 0.2 s with determinism checks, `frame --scenario` pictures, `ttl` and `count_<tag>`, the sound DSL, and `game_generators check`. The whole game was working in one build after the first validate fix.

## Automatic measurements

<!-- automatic-measurements:begin -->
Friction report (`scripts/agent_bench.py summary`):

```json
{
 "commands": 27,
 "failed": 2,
 "documentation_topics_read": [
  "2d",
  "audio"
 ],
 "searches": [
  "level advance next level on win levels",
  "velocity launch on press sets velocity body no gravity",
  "teleport target syntax velocity target tag"
 ],
 "cli_source_exploration": 0,
 "retries_of_a_failed_command_without_a_success_between": 0,
 "repair_cycles": 2,
 "first_failures": [
  {
   "command": "validate examples/2d/glass-bridge.game2d.json",
   "error": "12 error(s)"
  },
  {
   "command": "verify examples/2d/glass-bridge.game2d.json",
   "error": "FAIL  [audio-waveform] sound `thunk`: DC offset -0.044: wastes headroom and clicks when it starts or stops"
  }
 ],
 "used_verify": true,
 "minutes": 4.9,
 "transcript": {
  "engine_files_read_with_tools": 0,
  "engine_files_read_with_the_shell": 0,
  "examples": [],
  "edits": 0
 }
}
```

Agent session: 35 assistant turns; tools {"Bash": 16, "Read": 6}; cost $0.73; wall 5.5 min.

Notes logged during the run: 6 (7 minutes claimed lost). Measured wall time 5.5 min. **The claimed minutes exceed the measured wall time: read `cost_min` as a relative weight, not as minutes.**
<!-- automatic-measurements:end -->
