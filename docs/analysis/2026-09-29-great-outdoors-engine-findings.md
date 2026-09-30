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
