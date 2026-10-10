# What building Redline taught us about Red (2026-10-10)

An AI session (Claude) built **Redline** ([RedEngineGames/projects/redline](https://github.com/kevstermcgee/RedEngineGames/tree/main/projects/redline)), a first-person momentum roguelite, on RedEngine `main` at `9c62ea7` with no engine changes: one
generated scene (2,525 objects, 657 rules, 227 vars, 44 persisted, 19 chambers 520 m apart, a hub, synthesized music), 1,784 lines of
Python generator and test tooling, and 25 `checks.sim` scenarios that prove every chamber can be cleared at base speed and that the
run loop works (dive, clear a chamber, go deeper, burn, burn out). This is observation from that build, not instruction: each item
names its evidence so it can be checked, and the suggestions are suggestions.

The game is a hub plus chambers you teleport between; a run is one "match" that ends with an `end` action and an end card, and the
card's button restarts the scene with the persisted variables. That loop (persist + start/end cards + restart) carried a whole
roguelite meta-progression in pure data, which is the strongest thing in this report.

## The short version

1. **Data games cannot make a sound or a flash of their own.** Nothing a rule does is heard; a pickup, a checkpoint, a "under par!"
   is silent text. This is the ceiling on how good a data-only game can feel. (item 1)
2. **Two physics/tooling bugs with one-line repros**: being pushed out of a ledge turns into an 11x horizontal speed burst (item 2);
   a scenario `until_event` step ends at once if the event happened before the step began (item 3).
3. **`shot` photographs the frame after the steps that follow it**, so `shot` then `look`/`press` captures the wrong picture (item 4).
4. **`lint`/`reach` abort with an allocation failure** on a map spread over a large area (item 5).
5. **Proving a platformer is expensive**: making 19 chambers provable needed a 320-line route planner (item 6). A few scenario
   primitives would remove most of it.
6. **A minimal HUD cannot be built**: the banner and the stats panel have fixed widths, and the panel jumps down whenever a banner
   shows (item 11).

## Findings, most valuable first

### 1. Rules have no way to play a cue or flash the screen

- **Problem.** The rules' actions are `set add emit hide show collision deactivate activate teleport end impulse reset place`. None makes
  a sound. The scene `audio` block can duck the music on an event and fade a score layer by a variable, but cannot play a one-shot.
  So everything the game itself means (spark collected, checkpoint, vent, under par, burned, upgrade bought) is silent; only the
  engine's own cues (jump, landing, pad launch, footsteps) are heard. There is also no screen flash/tint a rule can trigger; the
  only feedback channel is HUD text.
- **Evidence.** Redline faked feedback with an objective line that a rule switches on for 1.8 s (`_msg`, `_msg_t`, `_msg_on`, 12 message
  codes plus one per chamber, 52 objective candidates in all). `describe audio` and `search "play a sound when a rule event fires"` find no way to do it; the
  2D side has `effects` with sounds (`recipe shared-effect`), the 3D side does not.
- **Where.** `src/sim/rules_run.rs` (`Effect`), `src/bin/re2/frame.rs` (where effects are applied), `src/sfx.rs` (the cue catalog:
  `fx.kill_ding`, `fx.level_up`, `fx.pad_launch`, ...), `src/feel.rs` (screen flashes already exist for level-up/respawn).
- **Suggested change.** One presentation-only action, `{"cue": "fx.level_up"}` (any `audio list` name or a sound JSON file next to the
  scene), and optionally `{"flash": "#ffcc33"}`, applied by the client and ignored by the server (like `hide`), with `checks.audio` able
  to assert a cue fired. An event-to-cue map in the `audio` block (`"cues": {"spark": "fx.hit_tick"}`) is the same thing without a rule.
- **How we'll know.** Redline's message machinery shrinks to one action per rule, and a headless dump's `/cues/recent` lists `spark` cues.

### 2. A push out of a ledge becomes horizontal speed (bug)

- **Problem.** In `step_player_on_tuned`, after the horizontal move, `if (actual - intended).abs() > 0.001 { velocity[axis] = actual / FIXED_DT }`.
  It is meant to remove the blocked part of the velocity, but when the move *pushes the player out* of a collider (the feet drop
  below a ledge the body circle still overlaps), the push is larger than the intended move and is stored as velocity.
- **Repro.** A 4x4x4 box, the player creeping off its edge at 0.95 m/s (crouch with `crouch_multiplier: 0.1`): at tick 35, as the feet
  drop below the step height, horizontal speed goes **0.95 -> 11.03 m/s** in one tick and the player is flung about 5 m from the box.
  In Redline it shows as a sideways shove when you come down just short of a platform (traces had 0.5 -> 11.4 and 0.4 -> 16.8 m/s).
- **Where.** `src/sim/player.rs`, the "Remove blocked components" loop in `step_player_on_tuned`.
- **Suggested change.** Only ever reduce a component: if the actual move along an axis is smaller than intended (or opposite), set that
  velocity component to `actual / dt` clamped toward zero, never above the old value in magnitude; a depenetration push should move the
  player without adding speed. A unit test: creeping off a ledge never raises horizontal speed above the walk speed.
- **How we'll know.** The repro's largest one-tick speed gain is under 1 m/s.

### 3. A scenario `until_event` step can end before it starts waiting (bug)

- **Problem.** The step's doc says it ends "once that event has happened since it began". The code counts events from `cur.mark`, which
  is only reset when a step ends *through* `until_event`; a step that ends normally (a `wait`, `hold` or `walk` that runs out) leaves the
  mark where it was. So `{"wait": 4, "until_event": "a_on"}` returns immediately if `a_on` fired at any time since the last
  until_event step (or since the scenario began). The mark is also an index into a history that drops its oldest entries at 4,096, so a
  long scenario counts from the wrong place.
- **Evidence.** Phase platforms in Redline emit `a_on` every 3.2 s. A runner that walked 1.2 s and then waited for `a_on` jumped at once and
  fell when the platform vanished. Workaround in `tools/routes.py`: wait for `a_off` first, then `a_on`.
- **Where.** `src/sim/scenario.rs` around the `until_event` check (`sim.rules().history().iter().skip(cur.mark)`).
- **Suggested change.** Set the mark whenever a step begins (the `seen_step != step` branch), and count with a monotonic event counter
  rather than an index into the capped history.
- **How we'll know.** A test: emit an event at t=0.05, `wait 1`, then `wait 4 until_event` that event: the step lasts until the next emission.

### 4. `shot` photographs the frame after the following steps (playtest bug)

- **Problem.** A `shot` is rendered at the next presented frame, and zero-duration steps after it (`look`, `press`, `snapshot`) run first.
- **Repro.** Four coloured towers at N/E/S/W; script `shot, look 90, wait, shot, look 180, wait, shot, ...`: every picture shows the
  direction of the *next* `look` (the first shows east although the player faces north), while the dump's `yaw_deg` is right. In
  Redline `shot "end_card"` followed by `press restart` photographed the hub instead of the end card.
- **Where.** `src/bin/re2/headless.rs` / `shots.rs` (when a requested shot is drawn relative to the script queue).
- **Suggested change.** Draw the shot before the next step runs (or make `shot` occupy a frame), and say it in `describe playtest`.
- **How we'll know.** The compass repro returns N, E, S, W, N.

### 5. `lint` and `reach` abort on a map spread over a large area

- **Problem.** The reach flood allocates a dense grid over the map's solid bounds at 0.1 m (`levels: vec![Vec::new(); nx * nz]`). Redline's
  chambers span about 2,100 x 1,800 m, about 380 million cells, and `red_engine2 lint maps/main.json` dies in `handle_alloc_error`
  (`src/tools/reach.rs:165`) instead of reporting anything. `plan` and `reach` share the code.
- **Why it matters.** Teleport-linked rooms (a hub and levels, a roguelite's chambers, a puzzle game's rooms) are a natural way to build
  a data game, and the authoring tools assume one compact map. Redline's `checks` deliberately has no `lint` group so `game check` passes.
- **Suggested change.** Flood each connected cluster of colliders separately (spawns and zones say where to start), or cap the grid and
  fail with a message that names the bounds and the cell; at minimum, check the size before allocating.
- **How we'll know.** `lint` on Redline either runs or prints a one-line, actionable error.

### 6. Proving a platformer needs a route planner the engine could provide

- **Problem.** At 9.5 m/s, `walk` overshoots every waypoint (it ends within 0.25 m and keeps the momentum), a stop slides about a metre,
  and in the air momentum does not turn. A scripted runner that jumps gaps has to plan run-ups, take-off points, flight times and
  air-braking. Redline's `tools/routes.py` (320 lines) does that; getting 19 chambers to pass took most of the build's debugging time.
- **Evidence.** The failures it had to solve, in order: diagonal jumps flown straight (momentum), runners walking backwards to a waypoint
  they had overshot, landing on the next pad and being thrown again, waiting at the edge of a phase platform and sliding off, braking in
  the air into a backwards run on landing. Each was found with `sim --trace --dump-every`, which made it possible at all.
- **Suggested change.** Scenario primitives that say what is meant instead of how: `{"walk": "x,z", "stop": true}` (arrive standing
  still); `{"run_jump": {"to": [x, z], "y": top}}` (line up, take off at the edge, steer and brake in the air to land on the point, fail
  if the physics cannot); `walk --auto` that knows the scene's `player` jump and pads. A `jump_reach` line in `describe physics` that
  uses the scene's tuning would also help level design (Redline's generator computes it to refuse impossible jumps).
- **How we'll know.** A chamber of gaps, pads and phase platforms is provable in a dozen scenario steps written by hand.

### 7. The expression language needs min/max/floor/clamp and random numbers

- **Problem.** Expressions have arithmetic, comparisons and prop functions, nothing else.
- **Evidence.** Redline writes max as `best + (d > best) * (d - best)`, rounding as `x - x % 0.1`, a per-Heat record as six of those, and
  picks a random unvisited chamber from the tick the runner reached the vent on (`(tick * 7919 + depth * 104729 + runs * 613) % cnt`) plus
  a prefix-sum expression that finds the r-th unused chamber. It works and is deterministic, but it is unreadable.
- **Suggested change.** `min max floor ceil round abs clamp` and a deterministic `rand(n)` seeded from the match seed and the tick (the
  RULES_LANGUAGE_GAPS note already ranks random numbers high).

### 8. Teleport throws away the player's momentum and facing

- **Problem.** `teleport` zeroes horizontal velocity, and teleporting to a spawn uses its position but not its `yaw_deg`.
- **Why it matters.** In a movement game a portal that keeps your speed is the feature; and a teleport into a room should face the
  player into it. Redline lays every chamber out along -Z so that the player's facing survives the teleport.
- **Suggested change.** `{"teleport": "spawn", "keep_velocity": true, "face": true}` (the defaults stay as they are).

### 9. UI text: three inconsistencies

- The HUD silently draws a blank for characters the font lacks (`·`, `;`, `|`), while the `text` macro rejects the same characters with
  the list of what the font has. `validate` should check `ui` strings the same way. (Redline's start card lost its `;` and middle dots.)
- `\n` in an objective or card text is not a line break (the card text runs on), and a long objective line is shrunk to a tiny font
  rather than wrapped at a readable size. Either honour `\n` or say in `describe ui` that it is ignored and give a length budget.
- `ui-shot game-hud` ignores `hud.show_rules_vars: false` and draws nine plain variable rows the game itself hides, so the picture is
  not what a player sees.

### 10. A cold start on a headless Linux box

- No prebuilt release exists yet (`start` first could not list releases, HTTP 403, then reported none published), so the first step is a cold build: about 10 minutes on
  two cores here. It first failed because `alsa.pc` was missing (`libasound2-dev`; `libudev-dev` is also needed). CI installs them but
  `doctor` does not name them before a build fails.
- `frame` panics on a machine without a Vulkan driver: wgpu falls back to GL and the post pass fails with "`textureLoad` from depth
  textures is not supported in GLSL". Installing `mesa-vulkan-drivers` (lavapipe) fixed it and every picture tool then worked headless.
  Suggest `doctor` checks for a Vulkan ICD and the render path errors with that advice instead of panicking.

### 11. A minimal HUD is not possible from the `ui` block

- **Problem.** The objective banner is always `260 * s` px wide and the counters panel always `140 * s` px wide (`s = height / 240`),
  whatever they hold, and the panel sits `40 * s` px lower whenever a banner is showing (`src/ui/game.rs`, `hud_layout`). A scene
  controls only which rows exist and whether there is an objective; not size, scale, position or width.
- **Evidence.** Asked for a minimal HUD, Redline cut the panel to four rows, dropped its title and every standing banner, and kept
  banners for real events only. Even so, at 1280x720 "BURNED -3S" draws a 780 x 81 px gold bar, and while it shows the stats panel
  (420 px wide for "CLOCK: 16.9") jumps down 120 px. Counters cannot be conditional either, so the hub shows `PAR: 0 / 0`.
- **Suggested change.** Size the banner and the panel to their content; keep the panel where it is whether or not a banner shows (a
  fixed lane for the banner); let `ui` choose a corner and a scale (`"hud_style": {"corner": "top-left", "scale": 0.75}`); and give
  counters the objective's `if`. All presentation-only, checkable by `ui-check`.
- **How we'll know.** `ui-shot game-hud` of a four-counter HUD with a short banner shows a panel as wide as its longest row, in the same
  place with and without the banner.

### 12. Smaller things

- **No "Restart" in the offline pause menu.** A run can only end by burning out or quitting the game; a game with `ui.end` cards would
  want a pause-menu row that restarts the scene the way the end card's button does.
- **Docs drift.** `shadow_follow` (which makes a large world's sun shadows follow the camera, and gives a jumping player a shadow to
  land by) is in `describe scene` but not in SPEC's Lights section, which still says shadows are centred on `shadow_center`; `describe
  scene` says "lights <= 16" while SPEC allows 256 authored (16 active).
- **Offline play starts at the first spawn in the list**; worth one line in `describe scene` next to `spawns` (it decides which room a
  teleport-based game opens in).
- **Trace and sim JSON disagree on event shape** (`[tick, rule, name, player]` in a trace, objects in `sim --json`).
- **The `--dump` audio report describes countryside that is not playing.** Redline has no `audio.ambience`, so no bed is loaded and no
  call is played, but `re2 --headless --script --dump` reports `wind 0.42`, `bees 0.40` and a blackbird and a cuckoo in `recent`. In
  `src/bin/re2/ambient.rs` the bed levels and `recent` are recorded before the `spec.nature` gate. An AI reading the dump concludes its
  lava reactor has birdsong. Report zero (or omit `beds`/`calls`) when `nature` is off.
- **`describe scene` has drifted from the `player` parser.** Its `player` line lists `character: human|rat|wizard|cowboy|alien|robot`;
  the parser (`src/schema.rs`) prefers `humans_play_as`, also takes `boy`, and accepts `view` (first|third) and `mode` (peaceful), none
  of which the line mentions. `view` was found through `search`, not `describe`.
- **The player's look is a pick from seven bodies, never a colour.** `HumanLook::styled` says "scene authors can still override
  individual colours", but that is true of `human` objects, not of the player: `characters::character_object` fixes the shirt per
  character. Redline wanted a heat-suit orange robot and took the stock teal one. A `player.look: {shirt, pants, skin}` would do.

## What worked well (keep it)

- **`persist` + start/end cards + restart is a complete roguelite loop in data.** Verified in the real client headless with a seeded
  `RE2_SAVE_DIR`: buying an upgrade (bank 300 -> 288, level 1, the pip shown, the starting clock 25 -> 29), burning out, the end card,
  the restart, and the saved values surviving it.
- **Validation is excellent.** Every typo and range error came back with a path and a fix; a skip pad that needed a 35 m/s launch was
  rejected (`launch_speed` 2-30) before anything ran.
- **`sim` is fast and deterministic.** All 25 scenarios (about 6 minutes of play) run in 2.3 s; `game check` in 2.5 s. Because the chamber order comes from the
  tick, the "dive and clear the first chamber" integration test is reproducible.
- **`re2 --headless --script --dump`** made the HUD, cards, hidden objects and variables testable without a screen, and `frame`,
  `ui-check`, `audio report` and `audio picture` made the look and the music checkable by an AI.
- **The movement model is genuinely good**: Quake-style air control, bunny hopping by holding Space, momentum tuning per scene. It is the
  reason a movement game was the right thing to build on Red.
- **Synthesized scores** (`bpm`, patterns, chords, walks, `lufs`) gave three music layers in a few dozen lines of JSON, and
  `audio.layers` keyed to rule variables gives the run music and a low-clock alarm with no code.

## Not recommended

- Do not add a roguelite, time-trial or "run" runtime to the engine because of this game. Every gap above is a small, general primitive
  (a cue action, maths built-ins, teleport options, scenario steps, two bug fixes, a lint that copes with spread-out maps, a HUD
  that sizes to its content).
- Do not loosen validation to make generated content easier; it caught real mistakes every time.
