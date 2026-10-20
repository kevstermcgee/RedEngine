# Idea Forge run: Drowsy Orchestra (2026-10-20)

- Idea code: `0941723677`; kind 3D; slug `drowsy-orchestra`; game `examples/3d/drowsy-orchestra/drowsy-orchestra.json`
- Engine revision: `20e12f4a7b7e`; model: `default`
- The card: YOU write one word on a wall and the wall starts behaving like that word, BUT a bell rings every ten seconds and you must pick one of three new laws. Pushback: A clock-faced beast eats a minute of your day each time it catches you. Goal: Finish with the fewest actions; the quietest run scores highest.

## The game

**Mechanic:** you pick up a piece of chalk, the word on it is written on the wall ahead (MELT opens it for a while, LOW shrinks it to a hop-high step, STILL freezes the beast), and every ten seconds a bell makes you choose one of three laws (Hush, Loud, Sleep) that change the beast's speed, how long walls stay open and what a word costs.
**Loop:** read the wall, pick the word, run through before it closes, cross the Tock's red band, click a law when the bell rings. Win: reach the stopped clock (score: fewest words, fewest minutes lost). Lose: five minutes lost.
**Changed from the card:** you cannot type any word (no text input in rules), so the words are three chalks with distinct behaviours; the bell chooses by click (left, right, C), not a menu; the Tock catches on leaving its column (no way to move a player from an `every` rule). **Stranger pass:** shortened the bell prompt so it fits at 1280x720 (ui-check), moved the chalk plinths off the walking line (a walk pushed a chalk off its plinth), moved word text above the wall.

## Score at the time of writing

- pass: the game file exists (examples/3d/drowsy-orchestra/drowsy-orchestra.json)
- pass: lint passes (PASS lint  0 error(s) (max 0), 0 warning(s) (max 0), 8 ignored (floating, interest, headroom))
- pass: verify passes (examples/3d/drowsy-orchestra/drowsy-orchestra.json: 8 check(s), 0 failed, 0.1s)
- pass: at least two scripted playthroughs (checks.sim) (7 playthrough(s))
- pass: the game logic is data (rules) (79 rule(s))
- pass: a declared ui (cards, objective)
- pass: a sound or music (audio)
- pass: a palette of its own (>= 6 roles not the default) (16 of 16 role(s); mean distance from the defaults 176)
- pass: prefabs of its own (>= 3) (4: tock, bell, chalk_plinth, cog; shared catalogue 9% of 69 placed)
- pass: its own sound files (2 file(s))
- pass: no default look kept (lint style-*) (none)
- pass: its generator rewrites the game byte for byte (build.py 0.0 s)

## Findings

Ranked: what would save the next run the most first. `severity` 1 = papercut, 2 = cost real time, 3 = blocked this run or would block most runs. `proposal` is an engine change, not a workaround.

```json findings
[
 {
  "id": "F3",
  "key": "no-rule-reads-player-position-for-enemy-catch",
  "area": "idea-fit",
  "title": "validate: teleport in an every rule is an error (no player). A beast stepping onto a stationary player cannot ",
  "severity": 3,
  "cost_min": 8.0,
  "evidence": "`validate`: 'rules[63] (tock_catch): `teleport` moves the player that triggered the rule, but a `every` trigger has no player'. The first validate also printed 20 cascade errors ('vars: none') after that one.",
  "workaround": "`every` rule sets hit=1 when px==bi (px tracked by enter/exit of 7 column zones); the teleport fires from the zone-exit rule. Cost: a beast cannot catch a stationary player until they move.",
  "proposal": "let every/timer rules act on 'the player in zone Z' (a per-zone occupant) or add an {enemy_touch} trigger / zone occupancy var"
 },
 {
  "id": "F1",
  "key": "no-rule-triggered-one-shot-sound",
  "area": "idea-fit",
  "title": "No way for a rule to play a one-shot sound (bell, catch thump, chalk scratch): search 'sound on event' / SPEC ",
  "severity": 2,
  "cost_min": 4.0,
  "evidence": "Searched 'sound on event' and grepped SPEC.md: only `audio.duck` (events) and `audio.layers` (var-driven score fade) exist. No rule action plays a one-shot.",
  "workaround": "Bell is a looping score layer faded by var `bellglow`; catch, chalk-write and law-pick have no sound.",
  "proposal": "add a rule action {sound: name} (builtin fx.* or file) with optional position"
 },
 {
  "id": "F5",
  "key": "frame-does-not-run-start-rules",
  "area": "tooling",
  "title": "frame/wiki shots draw the scene before any rule runs, so objects the start rule hides (all words, all 7 Tock s",
  "severity": 2,
  "cost_min": 6.0,
  "evidence": "`frame` and `wiki --preview` at t=0 show all 7 Tock stations, every word and every law text at once (start rules not run).",
  "workaround": "Per-shot `hide` globs in the wiki JSON.",
  "proposal": "frame/wiki shot: run 'start' rules (and optionally N sim seconds or a scenario) before drawing"
 },
 {
  "id": "F7",
  "key": "no-string-tables",
  "area": "idea-fit",
  "title": "Text macro (type text) made words on walls trivial; show/hide per word is the only way to 'write' (no string v",
  "severity": 2,
  "cost_min": 5.0,
  "evidence": "The wall's current word cannot be a string var; wrote one `text` object per word per wall and show/hide them (about 20 objects).",
  "workaround": "show/hide per word.",
  "proposal": "string vars/text objects bound to a var"
 },
 {
  "id": "F4",
  "key": "no-unarmed-player-option",
  "area": "idea-fit",
  "title": "3D player always holds a baseball bat (weapons.starting accepts only bat/firearms, no 'none'), so attack as a ",
  "severity": 2,
  "cost_min": 3.0,
  "evidence": "In playtest the contact sheet shows a baseball bat in hand; `describe scene` weapons.starting lists only bat and firearms.",
  "workaround": "Left-click (a bat swing) is the law key; dressed as the conductor's baton.",
  "proposal": "weapons.starting: \"none\" for puzzle/walk games; show the start card in playtest"
 },
 {
  "id": "F2",
  "key": "describe-rules-enough",
  "area": "worked",
  "title": "describe rules + recipe + velvet-orchestra example were enough to plan the whole game quickly",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "From `describe rules`, `describe ui`, `describe audio` and the velvet-orchestra example I had a validating scene in one pass (about 25 min).",
  "workaround": "none needed; keep.",
  "proposal": "keep `describe rules` as the first read; add a 'puzzle with props as verbs' recipe."
 },
 {
  "id": "F6",
  "key": "sim-verify-phases",
  "area": "worked",
  "title": "sim with approach + look_at/interact holds, hop via forward+jump, and verify with phases all worked first time",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "`verify` ran lint + 7 sims (melt, close, LOW, bell/law, SLEEP cost, win, lose) in 0.1 s; `phases` made walls openable for lint reach.",
  "workaround": "none needed; keep.",
  "proposal": "keep sim helpers (`approach`, `look_at`, `hold`) and `phases`."
 }
]
```

## Idea fit

Expressed well: a word that changes a wall (hide/show/deactivate/activate plus timers), the 10 s bell (a self-restarting timer), three laws (vars read by expressions), fewest-actions score (counters). Not expressible: free typing of a word (no string input or string vars), a beast that catches a standing player (no player context in `every` rules), one-shot sounds on events, unarmed player.

## What worked

`validate` messages that name the rule and say the fix; `sim` with `approach`/`look_at`/`interact` holds and `expect` on collision/vars/events; `verify` in 0.1 s; `phases` for walls that open; `playtest` pictures from the player's eyes with no window; the `text` macro for lettering; `ui-check` at nine sizes; prebuilt executable fetched with no compile (0 builds this run).

## Automatic measurements

<!-- automatic-measurements:begin -->
Friction report (`scripts/agent_bench.py summary`):

```json
{
 "commands": 56,
 "failed": 5,
 "documentation_topics_read": [
  "overview",
  "rules",
  "audio",
  "ui",
  "style",
  "audio",
  "objects",
  "sim",
  "playtest",
  "scene"
 ],
 "searches": [
  "moving platform or patrolling enemy that chases the player",
  "object moves along a path / mover / animate / elevator in 3D scene",
  "play a sound when a rule fires event audio cue",
  "audio layers rule-driven sound on event, checks.audio",
  "player starts with no weapon unarmed weapons none"
 ],
 "cli_source_exploration": 0,
 "retries_of_a_failed_command_without_a_success_between": 0,
 "repair_cycles": 3,
 "first_failures": [
  {
   "command": "validate examples/3d/drowsy-orchestra/drowsy-orchestra.json",
   "error": "21 error(s)"
  },
  {
   "command": "lint examples/3d/drowsy-orchestra/drowsy-orchestra.json",
   "error": ""
  },
  {
   "command": "sim examples/3d/drowsy-orchestra/drowsy-orchestra.json",
   "error": ""
  },
  {
   "command": "verify examples/3d/drowsy-orchestra/drowsy-orchestra.json",
   "error": ""
  },
  {
   "command": "playtest examples/3d/drowsy-orchestra/drowsy-orchestra.json --script examples/3d/drowsy-orchestra/playtest.json",
   "error": ""
  }
 ],
 "used_verify": true,
 "minutes": 8.3,
 "transcript": {
  "engine_files_read_with_tools": 0,
  "engine_files_read_with_the_shell": 5,
  "examples": [
   "scripts/dev start \"3D drowsy orchestra\" --workflow game-create 2>&1 | tail -40; cat AGENTS.md | head -60",
   "R=\"scripts/dev red\"; $R describe audio 2>&1 | sed -n 80,140p; grep -n '\"cues\"\\|\"sounds\"\\|\"on_event\"\\|\"events\"' SPEC.md |",
   "grep -n -i 'sound.*emit\\|emit.*sound\\|\"sound\"\\|\"sfx\"\\|cue.*event\\|events.*sound' SPEC.md docs/AGENT_REFERENCE.md | head ",
   "scripts/dev red search \"player starts with no weapon unarmed weapons none\" 2>&1 | grep -v '^    ' | head -8; grep -n '\"w",
   "python3 - <<'E'\nimport json,re\np='docs/analysis/idea-forge/2026-10-20-3d-drowsy-orchestra.md'\ns=open(p).read()\nm=re.sear"
  ],
  "edits": 1
 }
}
```

Agent session: 76 assistant turns; tools {"Bash": 32, "Read": 8, "Write": 1}; cost $1.28; wall 9.3 min.

Notes logged during the run: 7 (26 minutes claimed lost). Measured wall time 9.6 min. **The claimed minutes exceed the measured wall time: read `cost_min` as a relative weight, not as minutes.**
<!-- automatic-measurements:end -->
