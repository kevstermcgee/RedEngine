# Idea Forge run: Borrowed Foundry (2026-10-11)

- Idea code: `4930912320`; kind 3D; slug `borrowed-foundry`; game `examples/3d/borrowed-foundry/borrowed-foundry.json`
- Engine revision: `55de6ce99ae5`; model: `default`
- The card: YOU swap bodies with whatever you point at for as long as you hold your breath, BUT the world is drawn only by the sounds you make. Pushback: Your energy drains faster the more you succeed. Goal: Put every enemy to sleep at the same moment.

## The game

**Mechanic:** grab a glowing jar to swap into a borrowed body beside a guard wasp for as long as your breath lasts, while the dark hall is drawn only by the sounds you make (bat swings).

**Loop:** in the dark hub swing the bat to light the hall for 4 s, grab a jar (E: you are teleported to a pedestal by a wasp, breath 8 s), swing three times to hum it asleep (each swing also lights the room), step off the pedestal to come home, repeat for the next wasp. A wasp sleeps 24 s; **win** = all three asleep at the same moment. **Lose** = energy runs out; it drains 0.3/tick plus 0.35 per sleeping wasp (success makes it drain faster) plus 0.25 while swapped. If breath runs out you lose the body and must walk home in the dark.

**Adapted/cut:** body swap is a pickup + teleport (no real possession of an entity); 'point at' is the engine's aimed pickup; the jars always glow so the player can find them (everything else is hidden until a swing); collision stays when the hall is hidden, so it is blind walking. Playtest (player-eye pictures) not run: needs a second build.

## Score at the time of writing

- pass: the game file exists (examples/3d/borrowed-foundry/borrowed-foundry.json)
- pass: lint passes (PASS lint  0 error(s) (max 0), 0 warning(s) (max 0), 7 ignored (floating, interest))
- pass: verify passes (examples/3d/borrowed-foundry/borrowed-foundry.json: 7 check(s), 0 failed, 0.1s)
- pass: at least two scripted playthroughs (checks.sim) (5 playthrough(s))
- pass: the game logic is data (rules) (38 rule(s))
- pass: a declared ui (cards, objective)
- pass: a sound or music (audio)
- pass: a palette of its own (>= 6 roles not the default) (12 of 12 role(s); mean distance from the defaults 208)
- pass: prefabs of its own (>= 3) (3: wasp, cradle, comb_panel; shared catalogue 13% of 23 placed)
- pass: its own sound files (1 file(s))
- pass: no default look kept (lint style-*) (none)

## Findings

Ranked: what would save the next run the most first. `severity` 1 = papercut, 2 = cost real time, 3 = blocked this run or would block most runs. `proposal` is an engine change, not a workaround.

```json findings
[
 {
  "id": "F1",
  "key": "cold-worktree-build-and-preflight-rebuild",
  "area": "tooling",
  "title": "Cold build in a fresh worktree cost ~25 min and the default RED_TIMEOUT (900 s) killed it",
  "severity": 3,
  "cost_min": 28.0,
  "evidence": "`scripts/dev red describe --brief` -> 'dev: build red_engine2 TIMED OUT after 900s (exit 124)'; the first `scripts/dev start` also said 'executable MISSING', 'BLOCKER unrouted' with no --workflow, and seeding then broke the build (F2).",
  "workaround": "rm -rf target, RED_TIMEOUT=3000 build in background, waited.",
  "proposal": "Publish a prebuilt release (start already looks for one), share one CARGO_TARGET_DIR across worktrees, and have `start` accept a free-text task as game-create and start the build itself in the background."
 },
 {
  "id": "F2",
  "key": "seed-produces-e0460-mixed-artifacts",
  "area": "tooling",
  "title": "`scripts/dev seed --from <sibling>` reported success but the build then failed",
  "severity": 3,
  "cost_min": 8.0,
  "evidence": "seed_target copied 1399 entries in 5.8 s, then `error[E0460]: found possibly newer version of crate serde_core`, after manual deletion `error: crate thiserror required to be available in rlib format`.",
  "workaround": "Deleted the whole target and cold-built (F3).",
  "proposal": "Only seed from a sibling with identical rustc and Cargo.lock, run `cargo check` after, and wipe the copy if it fails."
 },
 {
  "id": "F3",
  "key": "no-rule-readable-held-input-or-breath",
  "area": "idea-fit",
  "title": "Rules cannot read held inputs, an aimed-at object, or the player's noise; carrying a prop blocks the bat swing",
  "severity": 3,
  "cost_min": 6.0,
  "evidence": "First design: hold a jar = hold your breath, swing the bat while holding. sim: held jar, no `swing` events at all (events seen: dark, pickup, swapped).",
  "workaround": "Swap on `pickup` + teleport + `reset` of the jar (frees the hands); breath is a counter var; 'exhale' = stepping out of a zone.",
  "proposal": "Expose input actions (crouch/sprint/hold) and `looking_at` to rules; allow swinging while carrying, or a `held_input(name)` expression."
 },
 {
  "id": "F4",
  "key": "no-body-swap-or-camera-attach",
  "area": "idea-fit",
  "title": "No body swap: nothing to attach the camera/controls to another entity",
  "severity": 3,
  "cost_min": 0.0,
  "evidence": "`search 'swap places with object I look at'` found only 2D mechanics; no action moves control into an object.",
  "workaround": "Faithful subset: pickup a glowing jar -> teleport to a pedestal beside a wasp; the 'body' only grants the right to hum there.",
  "proposal": "A `possess`/`camera_to` action for a named object or NPC, plus NPC entities rules can address."
 },
 {
  "id": "F5",
  "key": "no-per-entity-timers",
  "area": "idea-fit",
  "title": "enemies cannot be real NPCs: wasps are static decor, 'asleep' is two swapped prefab sets driven by hide/show a",
  "severity": 3,
  "cost_min": 5.0,
  "evidence": "3 wasps x (awake prefab, napping prefab, nap timer var, tick rule, 4 show/hide rules) = 24 of the 38 rules just to fake 'asleep'.",
  "workaround": "Global timer vars per wasp and hide/show of two prefab copies.",
  "proposal": "Per-entity timers/ttl and rule-addressable NPCs with a state (known key)."
 },
 {
  "id": "F6",
  "key": "teleport-needs-triggering-player",
  "area": "runtime",
  "title": "teleport inside an every-rule (breath ran out while swapped) silently did nothing: gasp event fired but player",
  "severity": 2,
  "cost_min": 4.0,
  "evidence": "sim 'holding your breath too long': event `gasp` fired, player stayed at (-5,4); `teleport` in an `every` rule moves nobody and validate said nothing.",
  "workaround": "Breath-out only clears the swap; the player walks home (a pushback); voluntary exhale uses a zone `exit` rule, which has a player.",
  "proposal": "Validate should error on teleport in start/every/after rules, or add `who`/player targets to timer rules."
 },
 {
  "id": "F7",
  "key": "no-sound-events-for-rules",
  "area": "idea-fit",
  "title": "swing event is the only 'sound the player makes' in rules (no footsteps/jump/voice event); echo mechanic rides",
  "severity": 2,
  "cost_min": 0.0,
  "evidence": "Only `swing` (and shot/pickup) reach rules; no event for jump, footsteps, sprint or voice.",
  "workaround": "The bat swing is the 'sound you make'.",
  "proposal": "Emit noise events (jump/land/sprint/crouch-walk) with a loudness."
 },
 {
  "id": "F8",
  "key": "frame-ignores-rule-state",
  "area": "verify",
  "title": "tour/frame draw hidden objects (shell hidden by start rule, wasp and nap both shown), so no engine picture sho",
  "severity": 2,
  "cost_min": 3.0,
  "evidence": "`tour out/tour.png` showed the hall fully lit although a `start` rule hides `shell`; the hidden state is only checkable via sim `expect hidden/shown`.",
  "workaround": "Asserted hidden/shown in sim; no picture of the dark state exists.",
  "proposal": "`frame`/`tour --t N` should apply rules (and accept var overrides like `--set echo=4`)."
 },
 {
  "id": "F9",
  "key": "lint-vs-verify-ignore-mismatch",
  "area": "diagnostics",
  "title": "lint --ignore in checks.lint did not apply to the 'lint' command (6 floating warnings) but did to verify; also",
  "severity": 1,
  "cost_min": 4.0,
  "evidence": "`lint` printed 6 [floating] WARNs despite checks.lint.ignore; `verify` honoured it ('7 ignored'). A zone with kind 'pad' did not avoid `[interest]` ERROR for 4 spawn zones; needed 3 portals plus an `interest` ignore for a one-player game.",
  "workaround": "Added portals and ignore list.",
  "proposal": "lint honours checks.lint.ignore; the interest check should skip scenes with no multiplayer/bots."
 },
 {
  "id": "F10",
  "key": "playtest-needs-separate-client-build",
  "area": "tooling",
  "title": "playtest failed: 'cannot find the re2 client next to this program'; the describe-brief build builds only red_e",
  "severity": 1,
  "cost_min": 1.0,
  "evidence": "`scripts/dev red playtest ... ` -> 'cannot find the re2 client next to this program: build both with scripts/dev build'.",
  "workaround": "Not run: a second cold build did not fit the time box. No player-eye pictures.",
  "proposal": "Build re2 with the CLI the first time, or have playtest say the build time and start it."
 },
 {
  "id": "F11",
  "key": "sim-interact-assumes-carry",
  "area": "verify",
  "title": "sim interact step fails ('did not pick it up') when a rule takes the prop away at once (reset) - had to hand-w",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "sim step `interact: host_2` -> 'pressed interact ... but did not pick it up' because my rule reset the jar the same tick.",
  "workaround": "`approach` + `hold {look_at, interact}`.",
  "proposal": "An interact option `carried: false`, or expect `pickup` instead."
 },
 {
  "id": "F12",
  "key": "search-noisy-for-3d-input-questions",
  "area": "discovery",
  "title": "search for 'swap places with object I look at', 'player input held button in rules', 'enemies npc sleep' retur",
  "severity": 1,
  "cost_min": 3.0,
  "evidence": "`search 'player input held button in rules'`, `'swap places with object I look at'`, `'enemies npc sleep'` returned 2D mechanics, ADR text and Windows timer docs.",
  "workaround": "Read `describe rules`/`describe sim` and the drowsy-marathon example.",
  "proposal": "Rank describe topics first; answer 'no such capability' plainly."
 },
 {
  "id": "F13",
  "key": "array-id-contradictory-errors",
  "area": "diagnostics",
  "title": "array macro inside a prefab: template with id gave 'children[0].id: missing' AND 'template.id: remove it' at o",
  "severity": 1,
  "cost_min": 2.0,
  "evidence": "Prefab with an `array` whose template had an id: 'children[0].id: missing' and 'template.id: remove it' together.",
  "workaround": "Id on the array, none on the template.",
  "proposal": "One message: put the id on the array."
 },
 {
  "id": "F14",
  "key": "sim-report-fast-iteration",
  "area": "worked",
  "title": "describe rules + drowsy-marathon example were enough to model a whole game as data; sim reports with events/va",
  "severity": 1,
  "cost_min": 0.0,
  "evidence": "Three fix rounds of `sim`, each under a second, with events, vars, end position and prop state in the report.",
  "workaround": "",
  "proposal": "n/a"
 }
]
```

## Idea fit

Expressed: sound-lights-the-world (swing event + hide/show), energy that drains faster with success (expression over an `asleep` var), win-at-the-same-moment (conjunction of three timers). Not expressible: a true body swap (possession/camera attach), a breath held by the player (no rule-readable input), real sleeping enemies (no NPC entities, no per-entity timers), noise other than the bat swing.

## What worked

`describe rules`, `describe sim` and the drowsy-marathon example were enough to build the whole game as data; `sim` reports (events, vars, ends, prop state) made each fix round under a second; `verify` honoured lint ignores; `catalog --local` and `style-report` showed own prefabs; the validator's did-you-mean/stable errors (score `b2`, array ids) were fast to fix.

## Automatic measurements

<!-- automatic-measurements:begin -->
(the CLI fills this in after the run: command trace, failures, repair cycles, documentation read, engine source read)
<!-- automatic-measurements:end -->
