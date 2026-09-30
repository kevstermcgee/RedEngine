# Great Outdoors: what building a kart racer on Red taught us about the engine (2026-09-29)

Great Outdoors is a hosted fairytale-animal kart racer (eight drivers, third-person, pickups, bots, gamepad) built as the first vehicle game on Red, with the whole build measured. The numbers are in `benches/history/great-outdoors.json` (schema `red-game-build/1`); the design is ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature. This note is the friction: what the engine made easy, what it made hard, and what to fix so the next game costs less. Written after phases 1-3c (kart step, race, MatchSim, protocol v11, prediction).

## The short version

* **The core seams held.** One shared pure movement function called by the server, the client's prediction and the bots meant a new movement mode (karts) plugged in behind a single call and was identical everywhere by construction. Prediction over a real network: one correction of 0.107 m in five seconds (the light going green), and all eight drivers agree with the server bit for bit under lag, drifting, hopping and a wall.
* **Karts are nearly free to run.** 8 karts racing versus the same 8 players without a race: server tick p99 93 vs 91 us of a 16 667 us budget; +87 bytes per snapshot (+2.6 KB/s per client), exactly as calculated.
* **The cost was in plumbing, not physics.** Most of the work and nearly every mistake was in touching many places to add one concept (a scene block, a wire field, a client event). See the friction list.
* **A match without a race is untouched**, and that is tested: the wire encodes byte for byte as before, the checksum only changes when a race exists (so old traces replay: the Windows-recorded fixture still matches every checksum), and a plain scene still walks at walking pace.

## What helped

* `context`, `src show`, `src outline`, `src refs` gave the map of an unfamiliar subsystem in a few KB per question; reading whole files was rarely needed.
* The repo's own tests told me what I had missed: `describe scene` must list every scene key, `features.json` must own every file, SPEC keys are checked, derived doc facts (the protocol version) are rewritten by `preflight --fix`. Each caught a real omission with an exact message.
* Pure modules with unit tests first (`sim::kart`, `sim::race`), then the integration tests, found problems where they were cheap: every test failure in these phases was a mistake in a test's setup or expectation, not in the logic, and each was visible from the assertion message.
* The deterministic test clock (`Server::pump(now)` / `tick(now)`) and `RawClient` made a wire test quick and stable.

## Friction (each is a candidate fix)

1. **Adding a scene block touches five places by hand:** the `strict.rs` key list, `strict::check_sections`, the `Scene` struct and its parser, the second `Scene { ... }` literal in `menu.rs`, and `describe.rs` `SCENE_KEYS` plus a SPEC section. Tests catch a missing one, but only after the fact. Fix: a `Scene` builder or `Default` for the literal, and one registry that the key list, the section check and the docs are all derived from.
2. **Adding a wire field touches about 25 struct literals.** `PlayerSnap` and `Snapshot` are built by hand in 8 and 17 places (tests, benches, interpolation helpers). Fix: `Default` for both and use `..Default::default()` in tests, so a new optional field costs one line.
3. **A client event carried too little.** `NetEvent::Snapshot { own, ack_input_seq }` had to grow a `race` field, and two clients (the real session and the bot) each destructure it. Fix: hand clients the decoded `Snapshot` (or a small struct that can grow) instead of a destructured pair.
4. **Flag bits are allocated by convention.** A player's flags byte has game bits 0-3, a wire-only velocity bit at 7, and now a wire-only kart bit at 6, all as bare masks. Fix: named constants in one place with a test that they do not overlap.
5. **Two clients duplicate prediction glue.** `session.rs` and `bot.rs` each reconcile and step the predictor. I added `Predictor::reconcile_snapshot` and `apply_local_auto` so they share the kart logic; the rest of the glue is still duplicated.
6. **Rule variables are global.** Per-player state (laps, checkpoints) could not be expressed as rules, which forced a native `sim::race`. A per-player variable scope in rules-as-data would have covered laps, lives, per-player scores and per-player keys for every future game. This is the largest missing capability the game exposed.
7. **The light going green costs one small correction at the start of every race.** The client learns the phase from a snapshot, after the server has already released the kart. Extrapolating the start from the server tick and the countdown value would remove it.
8. **Surfaces have no home.** Dirt, mud and water need a per-position lookup; zones have a `kind` key but nothing reads it for movement yet. A `surface` zone kind feeding `step_kart` is the obvious next step (drivers already have the multipliers).
9. **`clippy --all-targets` finds what the default run does not.** Test-only lints (a needless `mut`, `assert_eq!(x, true)`, `&mut Vec` parameters) passed my lib-only clippy runs three times and were caught by the CI stage. Run `scripts/ci.sh clippy` before committing, not `cargo clippy --lib`.

## A mistake worth recording

A regex-driven mass edit to add a field to every struct literal also matched `-> Snapshot {` (function signatures) and `NetEvent::Snapshot { .. }` (enum patterns and a definition) and produced code that did not compile. It was caught immediately by `cargo check`, and the affected files were reverted and redone with guards. The lesson is friction item 2: when adding a field needs a script, the type is missing a `Default`.

## Not measured

Wall-clock and tokens per phase (no per-phase timing was recorded while working; the flow harness measures tool steps, not authoring), prediction over lossy or laggy links, eight real clients on the shared server, and feel on a physical gamepad or in a window. Everything above is headless.

## Phases 4-6: pickups, bots, and putting it on screen

What the second half of the build taught, most useful first. The numbers are in `benches/history/great-outdoors.json`.

### Using the tools on a real track found what the unit tests could not

* **`playtest` and `frame` let a build see itself.** With no display and no hardware GPU, the client still ran end to end (software rendering), took screenshots, and
  reported "7 of 7 other players drawn". Every visual bug below was found by *looking* at a rendered frame, not by a test: a log lying across the road, a
  washed-out palette, a camera far plane that showed only sky.
* **`lint` found a real hole.** The barrier ring had wedge-shaped gaps on the outside of every bend (a piece as long as the centre-line chord is too short on the
  outer edge), so a kart could leave the track. `leak` reported it as "the player can walk off the map"; the plan view showed exactly where. The fix is to build
  barriers along the mitred offset curve. The same lint also caught 235 z-fighting road pieces (planes overlapping at one height).
* **`race-test` (new) made "can this track be raced?" one command:** all eight bots finish three laps of the 651 m circuit in 0.2 s of wall clock for ~105 s of race.

### Tuning came from data, not taste

* **A pack of perfect bots is not a pack.** With top speeds from 21 to 27 m/s the winner finishes about 5 s (120 m) ahead of second place, so nobody is ever near
  anybody: 24 pickups, **0 hits**. Catch-up pacing (leaders ease off 5%, the tail pushes 5%, not at top skill) brought hits to 1; a homing Acorn (a mild turn onto a kart
  inside a 28 degree cone) brought a no-catch-up race to 11 hits. Bubbles were 14 of 23 pickups because the leaders reach the boxes first: the roll weights need to
  be re-checked against real races.
* **The game's own rule can hide a track problem, and a tool can wrongly report one.** The 30 s finish-grace ended a race before the slowest kart (Beaver, top speed 21
  against the Deer's 27) finished, which `race-test` first reported as "did not finish". It is a balance finding (the speed spread is wide enough for the grace rule to
  DNF the slowest driver), and the tool now ignores the rule so it judges the track.
* **Lap times per animal on the real track** (bots at 0.8): Deer 79.7 s, Coyote 84.2, Wolf 86.6, Hawk 87.0, Duck 88.9, Bear 98.1, Bunny 98.2, Beaver 102.8. Top speed still
  dominates: Bunny (best handling) and Bear are level. Beaver's Build never fires in a bot race, because it is slowest and nobody is behind him.

### Engine friction (new)

10. **Object rotation composes in an order that is easy to get wrong.** A horizontal cylinder (`rotation [90, yaw, 0]`) lay across the road on some pieces. Nested
    groups (turn the group, lay the child on its side) are unambiguous; `describe objects` should say how the axes combine.
11. **Free-camera renders need `camera.far`.** `frame --eye` from 300 m rendered pure sky because the scene's far plane was shorter than the view; the tool could
    warn when the eye is farther from every object than `far`.
12. **`game check`'s avatar audit is first-person shaped.** It reports "every body in play is drawn (Human x8, Cheddar the rat x8 ...)" for a kart race, where
    people are never drawn. It should audit the kart fleet in a race scene instead.
13. **The first-person lint (`leak`, `sunk`) is the only containment check.** It worked here (it found the gap), but "the player can walk off the map" is about a
    walker; a `race-test` that reports when a kart leaves the track polygon would say what a racer cares about.
14. **Parked models need `lint_ignore`.** The eight kart models sit at y = -50 until the client places them, which lint reads as "sunk 50 m". A scene-level way to
    mark objects as pool templates would avoid per-object ignores.
15. **Adding one concept still touched many files** (protocol, server, session, client, HUD, docs facts, feature index): the plumbing friction from the first half
    is unchanged, but the repo's own tests named each omission, which is the reason it cost minutes and not hours.

## Phases 7-8: terrain, a shortcut, the lobby, hosting and publishing

* **A balance lever needs a route, not a number.** With mud and water on the track the Coyote (fine on dirt) fell to 5th: the penalties cost more than the dirt gained. A stat tweak would have made him strong everywhere; a real dirt shortcut across the SE bend put him 1st at bot level 1.0 (85.2 s) and left the others where they were.
* **A shortcut is a trap for bots.** A bot that overshot the entrance pushed at the wall for the rest of the race because its next line point was "behind" the wall. The bot doc promised stuck recovery would drop a missed point but the code only reversed. Fix in the engine (general): wedged twice at the same point -> back up to the previous point and ignore the "passed" test until it is reached. Widening the mouth (12 m -> 19 m) cut the wedges further. `race-test` now prints where a bot that did not finish stopped, which is how this was found in minutes.
* **Scenery hid the shortcut.** Infield trees stood in the corridor (scenery does not collide, but you cannot see the way). A keep-out around the corridor in the generator fixed it; a render from above found it, no test could.
* **A race needs the match flow to be hosted.** With no `match` block the map was open play: no lobby, no second race. Adding one gives lobby -> race -> results -> lobby with a fresh world; the missing pieces were a round end when the race finishes and a winner from the standings. The sim only ticks in the flow's Playing phase, so the flow's own countdown is dead time before the race's: keep it 1 s.
* **The lobby's character byte is enough for a driver choice** (0..7 in a race), with the server clamping it by what the scene means. Duplicate choices resolve at round start (lower slot keeps it). No protocol change.
* **Green-light extrapolation was not done**: its cost is one 0.1 m correction per race and a fix means per-step race-phase knowledge in the predictor's replay.
* **Friction**: `sfx` is graphics-build only, so a pure cue module that returns clips had to be graphics-gated too (a headless build cannot see it); split cue logic from clips if it ever needs testing headless. Round-end conditions are hard-coded in the server (`rules_outcome`, score); a scene-level "round ends when" would have made races configuration.
