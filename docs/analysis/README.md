# Analysis notes

What builders (people and AIs) reported about using Red, and what was done about it (`red_engine2 analysis new|list`). The table below is generated: it groups every note's feedback by the engine feature it matches, ranked by how many different games raised something about it. Regenerate it with `red_engine2 analysis digest --write`.

<!-- analysis-index:begin -->
## Open friction (ranked by how many different games still hit it)

### bots — 6 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): Found: parked (disconnected) players held their ids and could lock newcomers out — Soft reservations (`Server::free_slot`)
- **[open]** 2026-09-24 (2026-09-24-cheddar-feedback.md): QUIC datagrams with TLS 1.3 and a bundled certificate — Feta — not yet: see "encryption" below; the decision is open
- 2026-09-27 (2026-09-27-transport-threat-model.md): Status with a full roster, Welcome, Input, Hello — < 400 B — yes — yes
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Tuning fun needs a human proxy (bots vs bots only) — **started, not merged** — `agent/bot-match` has a `red_engine2 bot-match --json` command with health thresholds and a human-proxy profile; not yet reviewed or merged.
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): More data-driven blocks so games need no engine edits (hud, sfx/music palettes, pickups, teams, kill feed) — **not done** — The engine work behind Trigger Happy (bots, ladder, pacing, feedback, feel, lag compensation, client hosting, tracers, music, avatars) is already on `game/work`. The newer blocks need protocol and UI design (a kill feed needs the killer's id on the wire; teams change scoring and spawns) that a second game should drive.
- 2026-09-29 (2026-09-29-great-outdoors-engine-findings.md): Great Outdoors: what building a kart racer on Red taught us about the engine — Great Outdoors is a hosted fairytale-animal kart racer (eight drivers, third-person, pickups, bots, gamepad) built as the first vehicle game on Red, with the whole build measured.
- 2026-09-30 (2026-09-30-agent-feedback-building-great-outdoors.md): Hours — A raceable map (loop, barriers with no gaps, gates, boxes, grid, bot line, animals, match block, checks) was ~400 lines of hand-written Python, iterated with 8 render/lint/race-test rounds. Every kart game needs the same thing. — **`red_engine2 race-track OUT.json`** generates it from a few numbers, lint-clean, bots finish it (tested at two sizes). **`new-game --kind race`** scaffolds a whole race project around it. The eight animals are now the core **`karts`** pack (`catalog karts`), so no game re-draws them.
- 2026-09-30 (2026-09-30-agent-feedback-building-great-outdoors.md): ~1 hour — A shortcut trapped bots: a missed line point behind a wall pinned them for the rest of the race, and the bot doc *promised* recovery that the code did not do. The tool reported only "did not finish". — Bots back up to the previous line point when wedged twice at one; **`race-test` says where a bot that did not finish stopped**.

### playtest — 2 note(s)

- **[open]** 2026-09-24 (2026-09-24-cheddar-feedback.md): Slow feedback loops (25 s verify, 52 s map test) — **partly done** — `verify` no longer computes the grid when only walks are selected and shares it across `reach` checks; `--only walk[1]` or any name text; every check is timed; `scripts/dev test` prints a summary and keeps the log. **Not done:** the reachability flood itself (about 3.4 s on a 885-object map) and a changed-files-only test runner
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Couldn't see the game the way a player does; nine one-off PowerShell capture scripts; concurrent runs photographed each other — **done** (ADR 2026-09-28-seeing-what-the-player-sees) — `re2 --shot-at 5,10 --shot-dir out/`, F12, and a script's `shot` step render the frame a second time into an offscreen target and read it back — no focus, no visible desktop, nothing to photograph by accident. `red_engine2 playtest MAP --secs 60 --shots 12` hosts a match, plays a scripted player (spin, walk, aim, fire, overview and follow cameras) and writes the pictures, a labelled contact sheet and a JSON report. A server-side spectator seat was not built; the overview/follow cameras are client-side.
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Pre-allocated pools are the only way to add dynamic things; hidden objects still cost draws — **done** — `ScenePool` owns sizing, claiming and hiding of pooled scene objects (avatars, tracers, sparks) in one tested place; `LiveRenderer::set_hidden_objects` skips hidden objects instead of drawing them at scale 0.0005. General mesh add/remove after renderer creation was not built.
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Windows shipping (a stale console handle blocked `println!` forever; packaging written from scratch; difficulty `.cmd` launchers because there's no options screen) — **partly done** (ADR 2026-09-28-windows-shipping) — `re2` now sets up its output for how it was started (terminal, double-click, redirected), logs to `re2.log`, and shows a crash box on a panic; `tools::pe` can flip a built exe to the GUI subsystem so a shipped copy never opens a console. `red_engine2 package`/a prebuilt kit and a map-declared `options` screen were started (`agent/game-package`, `agent/options`), not yet merged. A Windows CI test that closes the window and expects a clean exit was not built.

### blueprints_and_games — 4 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): `verify` printed "3 error(s)" for lint but not the errors — **already fixed in the current engine** — the fork predated it; `verify` prints the first findings inline
- 2026-09-24 (2026-09-24-cheddar-feedback.md): Generated map JSON compiled in and possibly stale; loose props at hand-typed coordinates block routes — **done** — `build --check` / `game check` fail when a map is not what its blueprint builds; blueprint `fill` keeps door pads, aisles between doors and spawn pads clear (tested over 12 seeds of a crowded room); a game's own prefab libraries are merged in with `prefab_files`
- 2026-09-24 (2026-09-24-cheddar-feedback.md): Map/game content fingerprint in the handshake — BlueEngine protocol 3 — already there (`WrongMap` reject; the hash ignores line endings)
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Silent failures cost a debugging round each (skipped avatars, players vanishing with no portals, a slab shoving a jumper sideways) — **done** — Every failed lookup counts (`RemoteStats`/`SessionCounters`: undrawn, stand-in, unposed, hidden by interest, pool use) in `RE2_STATS`, the F3 overlay and the dump, plus a warning the first time a player can't be drawn. New lint rules: `interest` (a map's occupied rooms with no portal path between them — see the caveat below) and `jump-clearance` (computed from the map's own `player` tuning). `game check` verifies every roster body has an avatar. Caveat found rolling this out: the lint compares *all* of a map's spawn points, not per spawn `group`, so a shared reference/testbed map with several unrelated spawn groups (like the engine's own Test Lab) can get a warning that isn't actionable; grouping the check by spawn `group` is the cleaner fix and is not done yet.
- 2026-09-30 (2026-09-30-agent-feedback-building-great-outdoors.md): ~1 hour — Hosting and publishing were manual, undocumented-in-one-place steps: build the headless server, make an identity, invent a join key, write a systemd unit, copy the project into RedEngineGames without scratch files or keys, hand-edit `.release-games.json` in its own style. — **`deploy/install.sh`** in every scaffold (reads `game.json`; `--info`, `--uninstall`); **`game publish ../RedEngineGames`** (never commits; refuses keys and env files); both documented in HOSTING.md / GAMES_PUBLISHING.md.
- 2026-09-30 (2026-09-30-physics-mini-games-feedback.md): Engine feedback from building three physics mini games — Three games were built with data and rules only (no Rust), each with a Python map generator and `checks.sim` scenarios: **Skyline Stacker** (stack crates, a barrel wrecks the tower), **Castle Crash** (throw ammo at a crate pyramid), **Plate Chamber** (three pressure-plate puzzle rooms).

### build_and_ci — 4 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): `check_headless.py` — Already covered by `tests/headless_boundary.rs` and CI; now also enforced on the packaged binaries — 
- 2026-09-24 (2026-09-24-cheddar-feedback.md): Toolchain setup friction (`. ~/.local/toolchain/env.sh;` before every command) — **done** — `scripts/dev` and `dev.ps1` (any OS, any directory, finds the toolchain, sets timeouts); scaffolded projects get `scripts/red`
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Build cost (4-minute LTO builds, a stale `red_engine2` after `cargo build --bin re2`, a `rustc` crash at codegen-units=16, C: filling up) — **partly done** — `[profile.fast]` is checked in and documented. A prebuilt engine kit and honouring `CARGO_TARGET_DIR` everywhere were started on the `agent/game-package` branch, not yet merged (see below).
- 2026-09-29 (2026-09-29-build-time-baseline.md): cold cargo build running — 4 — 32 / 75 / 76 / 137 — 7 182
- 2026-09-29 (2026-09-29-build-time-baseline.md): cold cargo build running — 8 — 96 / 846 / 3 172 / 3 929 — 13 750

### offline_renderer — 4 note(s)

- 2026-09-27 (2026-09-27-transport-threat-model.md): Eavesdropper learns a short join key — Possible: record one handshake, guess offline against the HMAC proof
- 2026-09-27 (2026-09-27-transport-threat-model.md): Offline guessing of the join key — Not possible from a recording: the proof is bound to TLS exporter material — As before
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Audio is blind for an AI — **started, not merged** — `agent/sound-lab` has a `red_engine2 sound-lab` command rendering cues to WAV plus waveform/spectrogram images and stats; not yet reviewed or merged.
- 2026-09-30 (2026-09-30-agent-feedback-building-great-outdoors.md): 30 min each — Object rotation composes `Rx*Ry*Rz` about world axes, so `[90, yaw, 0]` lay a cylinder across the road; `frame --eye` from 300 m rendered pure sky because `camera.far` was 200. — `describe objects` states the composition and the nested-group recipe; **`frame` warns** when the eye is farther than `camera.far`.
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **01** — **Laser Vault** — `PASS` (752 ticks, 0 alarms) — `PASS` (Checksum: `0x84608df09bab46af`) — Laser tripwire AABBs, multi-console sequential hacking, alarm counters, vault door deactivation.
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **02** — **Floor is Lava** — `PASS` (125 ticks, y=1.5m) — `PASS` (Checksum: `0x5aa0df4e1752dfad`) — Staged rising hazard timers, vertical platform step heights (<=0.35m), checkpoint perches, evac beacon.
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **04** — **Relic Relay** — `PASS` (360 ticks, delivered=1) — `PASS` (Checksum: `0x552151cc7b168dce`) — Fragile courier dash, periodic decay countdown, mid-point stabilization recharge pedestal.
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **05** — **Prism Matrix** — `PASS` (337 ticks, energized=1) — `PASS` (Checksum: `0xe5b89bcb234ce136`) — Optical beam redirection, multi-station angular alignment, order-independent accumulator gating.
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **06** — **Phantom Maze** — `PASS` (399 ticks, exit=1) — `PASS` (Checksum: `0x2cab1dfdcc7bc2af`) — Cloaked labyrinth navigation, sonar ping sensor pads, dynamic obstacle visibility reveal.
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **07** — **Bomb Defusal** — `PASS` (382 ticks, defused=1) — `PASS` (Checksum: `0xa956c78bb17fafc7`) — Ticking ordnance crisis, decreasing countdown loop, strict 3-station chronological wire cutting.
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **10** — **Target Gallery** — `PASS` (333 ticks, hits=3) — `PASS` (Checksum: `0x3abb1cbff54f98f7`) — Pop-up silhouette targets, reaction marksmanship, dual visibility & eligibility toggling.

### docs_and_adrs — 3 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): README "Known limits" said multiplayer did not exist; also "no 2-D UI" and a link to a sibling repo (`../forge3d`) that only exists on one machine — Rewritten; `tests/repo_hygiene.rs` bans those claims in README/SPEC/AGENTS/CLAUDE and checks every relative markdown link resolves inside the repo
- 2026-09-24 (2026-09-24-blue-comparison.md): No LICENSE — MIT `LICENSE` (same licence as Blue) and `license` in `Cargo.toml`; the hygiene test requires both. The holder is "Red Engine contributors": change it if you want a name
- 2026-09-24 (2026-09-24-blue-comparison.md): `FEATURES.json` (hand-written feature -> files -> checks) — `docs/features.json` + `features --check` (a test) + `impact` (changed files -> features -> dependents -> exact commands) (ADR 0033) — Dependencies are declared, not inferred
- **[open]** 2026-09-24 (2026-09-24-cheddar-feedback.md): Stale project docs ("not built", "95+ tests") — **done** (ADR 0025) — `CLAUDE.md` carries a facts block derived from the repo; `tests/docs_fresh.rs` fails on a stale block, a hand-written test count, a "not built" claim, a doc path that does not exist, an undocumented blueprint key or a CLI command missing from `AGENTS.md`
- 2026-09-24 (2026-09-24-cheddar-feedback.md): `tools/FEATURES.json` feature to file to check map — BlueEngine — no: hand-kept indexes drift; Red derives (`src map`, `status`)
- 2026-09-28 (2026-09-28-trigger-happy-feedback.md): CI bookkeeping fails after the work is done (an ADR in four places, file ownership, a byte budget, protocol bumps by hand, ADR number collisions across branches) — **done** (ADR 2026-09-28-generated-bookkeeping) — `red_engine2 preflight [--fix]` (about a second, compiles nothing) checks and repairs it; ADR files are the registry (the README index is generated); dated ADR ids (`adr new "title"`) avoid cross-branch collisions; file ownership and the protocol version in the docs are derived facts.

### net_transport — 3 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): `blue_portmap.py`: hard-coded control URL, gateway guessed as `.1`, SSDP port used as the HTTP port — `net::upnp`: SSDP discovery, description parsing, service-version handling, ownership rules, permanent-lease refusal, lease renewal, CGNAT warning, `red_server --upnp` (ADR 0031) — Tested against an in-process fake router and SSDP responder, **not a real router**. No NAT-PMP/PCP/IPv6
- 2026-09-27 (2026-09-27-transport-threat-model.md): Floods — Connection cap (16), handshake timeout 5 s, inbound queue 1024 (excess dropped, counted), stream budget 64 x 8 KB — As before
- **[open]** 2026-09-28 (2026-09-28-trigger-happy-feedback.md): small — Net tests use real-time UDP; no in-memory transport with a virtual clock — **not done** — It is a refactor of every socket use in the server and client.

### net_server — 2 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): No handoff file, so a closed terminal costs a full re-derivation — **done** — `red_engine2 status` (one screen: facts, git, recent commits, uncommitted files, `STATUS.md`), `status --note "..." --section next`
- **[open]** 2026-09-24 (2026-09-24-cheddar-feedback.md): small — `assert_send::<Server>()`, `Server::run` blocking shape, bot as practice opponent, `hint_for` / `lobby_hint` parity — not done — Cheddar-specific or minor; the blocking `Server::run(&stop)` shape was kept
- 2026-09-27 (2026-09-27-transport-threat-model.md): On-path attacker on an *open* server — Session key derivable from the visible handshake: tags stop only blind attackers
- 2026-09-27 (2026-09-27-transport-threat-model.md): Client reaches an impostor server — Undetectable: no server identity
- 2026-09-27 (2026-09-27-transport-threat-model.md): Impostor server — Refused: certificate pinned by SHA-256 or verified to a CA; `ServerIdentity` rejection, no retry, no fallback — Undetectable (why it is loopback-only by default)

### karts — 1 note(s)

- **[open]** 2026-09-29 (2026-09-29-what-building-three-physics-games-taught-us-about-red.md): `describe sim` hides `hold` keys; aiming needs hand-computed angles; `scene.zones` clobbers generated zones; HUD shows every var raw; `info`/`catalog` silent about loose/mass; `lint_ignore` gaps; zone edge order and topple direction undocumented; steering note (all three) — **done in part** (ADR 2026-09-29-authoring-ergonomics-from-the-physics-games) — `hold: {look_at: [x,y,z]}`; `describe sim` lists every key and a test keeps it complete; `scene.zones` merge by id (spawns/camera replace, documented); `_` vars stay off the HUD; `info <id>` prints `loose: yes/no (why)` with mass and carryability; `checks.lint.ignore`; the measured edge order, no-`enter`-when-starting-inside, topple direction and steering are in `describe` and SPEC; the JSON `describe rules` lists prop triggers, built-ins and events. Not done: a failed pickup saying why, `interact: {target}`, `sim --sweep`, a `chain` command, `weapons.bat.impulse`, persistent vars and text labels, lint's `drop` on small props.

### match_flow — 1 note(s)

- **[open]** 2026-09-24 (2026-09-24-blue-comparison.md): Multiplayer template: menu, connect, lobby, ready, HUD with ping, rematch. Its screens are drawn but not wired (Connect flips a screen; Ready jumps to Playing) — The whole loop is real: connect form with masked key, lobby with roster / character / ready, countdown, timed rounds, results, rematch, late joiners, HUD with ping, timer, scoreboard (ADR 0029). Pure `sim::flow` state machine, state-based lobby messages that survive packet loss, screens audited at 9 window sizes, proven over real UDP (`tests/net_flow.rs`) and with a real window (`scripts/lobby_demo.ps1`) — Not built: spectator camera, teams, kick/ban, map voting

### verification — 1 note(s)

- **[open]** 2026-09-29 (2026-09-29-what-building-three-physics-games-taught-us-about-red.md): Spawn height ignored by lint/reach and the offline client (KA); `build` lints from the wrong spawn (KA); `walk`/`reach`/`verify` use the default walker, no pads (FD); lint cannot see rule-driven collision (FD); `path` walks ignore their first point (all) — **done** (ADR 2026-09-29-verification-honours-the-map, commit 0a36d04) — `MapWorld.spawn` is `spawns[0]` at its height (`spawn_y`); reach, lint, `checks.walk`/`checks.reach` (`from_y` on both) and the `walk` command start there; `re2` starts offline at `spawns[0]` (x, y, z, yaw); a blueprint with no spawns takes the first `scene.spawns` entry for its camera, self-check and generated checks; the walker is `sim::player::step_player_tuned` with the scene's tuning and jump pads; unconditional `start` rules' `collision: [id, false]` are open to every tool; a failing `path` walk without `from` explains that it starts at the spawn. `tests/verify_map.rs` through the real CLI and client. Not done: the lint flood-fill still does not launch off pads (a `checks.walk` proves a pad route instead), no `jump: true` leg, conditional rules stay solid.

## No open friction (resolved, or no row says either way)

### replay — 5 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): Join key sent inside a cleartext JSON `Hello`; "no encryption or cryptographic authentication" — Challenge/response (the key is never sent), stateless address cookies (no amplification), HMAC tag on every datagram, replay and forgery tests (ADR 0028) — Authentication only: **traffic is not encrypted**, and a short key can be guessed offline. Blue's template does have QUIC/TLS transport; Red does not
- 2026-09-24 (2026-09-24-cheddar-feedback.md): UI work is blind without a screenshot path; magic-number layout — **done for the engine's screens** (ADR 0026) — `ui-shot pause out.png --size 1280x720 --message "..."` renders with no window/GPU; `ui-check` audits every screen at 9 sizes and found three real overflow bugs in the launch/pause menus (fixed). Cheddar's own `ui.rs` lives in its repo and is not ported
- 2026-09-24 (2026-09-24-cheddar-feedback.md): `check` with persistent logs, not full output in the context — `be2.py check` — yes, in `scripts/dev` (`out/logs/`, summary only)
- 2026-09-27 (2026-09-27-transport-threat-model.md): Forgery / injection / replay — QUIC packet protection; replayed packets rejected by QUIC — HMAC tags, as before
- 2026-09-29 (2026-09-29-what-building-three-physics-games-taught-us-about-red.md): `replay` diverges at the strike tick of any bat swing on a prop (DH) and the tick after a rule `impulse` (KA); walk-and-pickup traces and `replay --against` agree — **done** (ADR 2026-09-29-replay-applies-each-shove-once, commit 67731e3) — The hypothesis was right: `MatchSim::apply_impulse` recorded an `Impulse` entry *and* the replay re-derived the strike from the recorded input, so the prop was shoved twice. `apply_impulse` is now only for external pushes (the server's demo kick); the bat, hitscan and rule `impulse` go through the non-recording `MatchSim::shove`. `tests/sim_replay.rs` records a swing at a dormant crate, a rule impulse and an external push on `tests/fixtures/strike_replay.json`, checks the prop really moved, and replays each with every checksum matched; the first two failed before the fix.
- 2026-09-30 (2026-09-30-agent-feedback-building-great-outdoors.md): ~1 hour — Hosted play had no lobby and no second race: without a `match` block a map is open play, and nothing ended a race round. — A finished race ends the round, the winner is first place, the lobby character byte is the animal (picker with keys, d-pad, bumpers). The race scaffold ships the `match` block.

### player_physics — 3 note(s)

- 2026-09-27 (2026-09-27-transport-threat-model.md): Resource bounds — Packet ceiling 1400, bounded decoders, per-session token buckets, parked-player cap
- 2026-09-28 (2026-09-28-trigger-happy-feedback.md): Orientation cost, undiscoverable debug switches, `player.character` reads as global — **done** — `docs/LIFE_OF_A_SHOT.md` and `docs/LIFE_OF_A_REMOTE_PLAYER.md` walk one trigger pull and one remote player through the code (function, tuning key, how to see it, its classic silent failure). `re2 --debug-help` lists every `RE2_*` switch and hotkey (a test keeps it complete). `player.humans_play_as` is the clear name (`player.character` still works).
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **09** — **Weight & Balance** — `PASS` (359 ticks, balanced=1) — `PASS` (Checksum: `0x2cab1dfdcc7bc2af`) — Physics scale equilibrium, ballast plate placement, counterweight hydraulic gate release.

### client_app — 2 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): Follow-camera boom that retracts and eases out — Feta `FETA_CAMERA_FIX.md` — not taken: game-side third-person camera work; worth reading before building one
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **03** — **Echo Chambers** — `PASS` (568 ticks, step=4) — `PASS` (Checksum: `0xe8a2aa410ff34f45`) — Simon-says memory sequence (N->S->E->W), custom audio/event emission, state machine locking.

### combat_weapons — 2 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): `ray` line-of-sight query — `be2-tools ray` — yes, as `ray` (uses the exact shapes the weapons use)
- 2026-09-30 (2026-09-30-ten-minigames-feedback.md): **08** — **Turret Trench** — `PASS` (343 ticks, 0 hits) — `PASS` (Checksum: `0x5aa0df4e1752dfad`) — Cyclic turret sweep / cooldown phases (modulo math), cover-to-cover infiltration, grid override.

### game_rules — 2 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): Bounded `GameDocument` (counters, rules, interactables) — BlueEngine `game.json` — no: Red's `vars`/`rules` (ADR 0020) is richer and already proven headless
- 2026-09-27 (2026-09-27-transport-threat-model.md): Eavesdropper reads traffic (positions, names, lobby, rule state) — Readable: nothing encrypted
- 2026-09-27 (2026-09-27-transport-threat-model.md): Full rule state (16 vars, 256 hidden, 64 collision) — <= 1392 B (test-enforced) — yes — no: sent on a unidirectional stream (reliable; it is repeated state)

### map_analysis — 2 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): A failing `walk` says where the player stopped, not what stopped them — **done** (ADR 0023) — `BLOCKED BY 'floor_crate_x' [prop:crate] gap 0.00 m, occupies x -9.80..-7.00 z 5.10..7.90`, passage width vs the 0.7 m body, and an image with the blocker boxed and labelled (`out/verify/<scene>_walk<N>_explain.png`)
- 2026-09-24 (2026-09-24-cheddar-feedback.md): `reach` passes but `walk` fails and nothing explains the gap — **done** — the diagnosis also says whether a flood fill from the stop point can reach the target ("the straight leg is what is obstructed: try `walk --auto`")
- 2026-09-24 (2026-09-24-cheddar-feedback.md): Authoring walk routes by guessing coordinates — **done** — `walk --auto --from X,Z --to X,Z[,Y]` plans a route (grid A* on the reach model, string-pulled, validated with the real per-tick physics) and prints waypoints plus a paste-ready check; a `checks.walk` entry can be `{"from","to","auto":true}` and plans every run
- 2026-09-28 (2026-09-28-trigger-happy-feedback.md): small — `docs/analysis/` had no CLI — **done** — `red_engine2 analysis new "Title" [--from file]`, `analysis list`, `search --kind analysis`. This very file was written by hand, matching that format.

### map_editing — 2 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): Checked patch transactions (`apply`) — `be2-tools apply` — yes, as `patch` (atomic, one validation, names the failing op)
- 2026-09-29 (2026-09-29-build-time-baseline.md): `preflight` after a one-line edit — 5.9 (rebuilt the graphics CLI) — 1.3 (tree-only)
- 2026-09-29 (2026-09-29-build-time-baseline.md): edit loop, localized edit (`rules.rs`) — `affected --quick --base HEAD` 23.7 — `iterate` 7.8 (8.9 CPU s), 1.6 with `--check-only`
- 2026-09-29 (2026-09-29-build-time-baseline.md): edit loop, central edit (`schema.rs`) — 36.7 — `iterate` 9.2
- 2026-09-29 (2026-09-29-build-time-baseline.md): edit loop, docs-only edit — 20.8 — `iterate` 0.3 (nothing to compile or run)
- 2026-09-29 (2026-09-29-build-time-baseline.md): `affected --quick` on this 17-file branch (default base) — 140 wall, 303 CPU s, 1.2 GB tree PSS — unchanged: use `iterate` for the edit loop

### net_client — 2 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): Found while building the replacements: remote players froze during a burst of lost snapshots, then snapped forward — Bounded, speed-aware extrapolation and a catch-up limit in `net::interp` (ADR 0034), guarded by `net-test` and unit tests
- 2026-09-27 (2026-09-27-transport-threat-model.md): Off-path attacker forges or injects datagrams — Stopped: per-session 8-byte HMAC tag (after the handshake)
- 2026-09-27 (2026-09-27-transport-threat-model.md): Relay of a join proof to another server — Useless: exporter differs per connection — Not prevented

### game_upgrade — 1 note(s)

- 2026-09-30 (2026-09-30-agent-feedback-building-great-outdoors.md): 10 min x3 — Test-only clippy lints passed my lib-only runs and were caught by the CI stage. — Written into the findings note: run the full `--all-targets` stage before committing. (The gate already does; the trap is local shortcuts.)

### graphical_client — 1 note(s)

- 2026-09-28 (2026-09-28-trigger-happy-feedback.md): The real client can't be tested headlessly; the glue that decides who gets a body had no test — **done** — `re2 --headless --script play.json --dump state.json` steps the same `App` the window steps, with a null renderer (a GPU is only needed if the script takes a picture). The dump (`re2-dump/1`) lists remote players and their avatars, the HUD text, cues, the crosshair and every counter; script `expect` steps assert on it by JSON pointer. `tests/client_headless.rs` runs the real binaries end to end.

### home_hosting — 1 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): systemd unit, hosting notes, router mapping — Feta `deploy/`, `docs/HOSTING.md`, `feta_portmap.py` — unit + `docs/HOSTING.md` yes; automatic UPnP mapping **not** (a good later `red_server --upnp`)

### net_auth — 1 note(s)

- 2026-09-27 (2026-09-27-transport-threat-model.md): Spoofed-source join flood / amplification — Stopped: stateless address cookie, Challenge smaller than Hello, Hello budget
- 2026-09-27 (2026-09-27-transport-threat-model.md): Cryptographic primitives — Hand-written SHA-256/HMAC (vector-tested); secrets from `RandomState` hashing (not a CSPRNG)
- 2026-09-27 (2026-09-27-transport-threat-model.md): Spoofed joins / amplification — QUIC Retry (stateless) before any connection state, then Red's cookie — Red's cookie, as before
- 2026-09-27 (2026-09-27-transport-threat-model.md): Primitives / randomness — rustls (ring); `sha2`/`hmac`/`subtle`; `getrandom` for cookie secret, tokens, nonces, `--key auto` — Same maintained primitives

### net_test — 1 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): `net-test`: 120 ticks of a prediction buffer through a latency model in one process — `net-test`: real server + real clients behind a seeded bursty-loss proxy, five profiles, judged on what a player would notice (ADR 0034) — It found and fixed a real defect (above). Not simulated: bandwidth caps, NAT rebinding, hitscan lag compensation

### performance_contract — 1 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): `inspect-performance` / `validate-budget`: one hard-coded 2-player lab, a mean — `perf` / `checks.perf`: any scene, real walking players, p50/p95/p99/worst, best-of windows, bandwidth, datagram size, promoted props, advice, runs inside `verify` (ADR 0030) — Wall-clock budgets are machine-dependent; no GPU or memory measurement

### prop_physics — 1 note(s)

- 2026-09-29 (2026-09-29-what-building-three-physics-games-taught-us-about-red.md): Rules and `sim expect` only see players; no prop reset; no swing event (all three games) — **done** (ADR 2026-09-29-prop-aware-rules-and-scenarios, commit 9bf951e) — Triggers `{prop_enter: VOLUME}` / `{prop_exit: VOLUME}` (any prop or `prop: id`) and `{prop_below: [id, y]}`; built-ins `prop_y(id)`, `tilt(id)`, `held(id)`, `mass(id)`, `moved(id)`, `props_in(zone)`; actions `reset: id \ — [ids] \ — {zone}` and `place: [id, [x,y,z]]`; events `swing` and `prop_hit`; `sim` expectations `{prop: id, in_zone \ — not_in_zone \ — below_y \ — y_lt \ — y_gt \ — tilt_gt \ — tilt_lt \ — moved \ — near \ — held_by}` with every prop's rest pose in the report; the client dump gains `/rules` and `/props`. Prop occupancy is part of the rule checksum; nothing allocates per tick unless a rule looks at props. `tests/prop_rules.rs` on `tests/fixtures/prop_rules.json`: a crate launched off a 2 m deck scores through `prop_enter` and fails without the impulse; one swing topples a three-domino line (`tilt(d3) > 60`) and fails without the swing; `reset` stands the line up for a second swing that topples it again; `place` and `held` work; the busiest scenario replays clean; the real client runs the same rules. KA's `demo_knock` and DH's room 1 proofs pass as plain `sim --scenario` files against their unchanged maps.
- 2026-09-29 (2026-09-29-what-building-three-physics-games-taught-us-about-red.md): The authoritative drop gives 1 m/s along the flat look and none of the player's motion; offline inherits planar velocity; distance depends on whether you stop; no upward throw; the held crate can sit inside a low wall (FD, KA) — **done** (ADR 2026-09-29-one-release-velocity, commit 5b51298) — `sim::player::release_velocity(state, look, throw_speed)` = the holder's horizontal and vertical velocity plus `player.throw_speed` (new key, default 1) along the pitched look, called by `MatchSim::interact` and the offline client. A released prop ignores its former holder's body for `physics::RELEASE_GRACE_TICKS` (10) via rapier collision groups. `PropWorld::hold_pose` sweeps the prop's own box at the held height. `tests/throw.rs` on `tests/fixtures/throw.json`: standing 0.2 m, walk-and-stop 1.7 m, sprint-and-stop 4 m of travel; looking up 45 degrees at throw speed 6 lands on a 1.2 m ledge 2 m away; the real client and the sim release a crate to the same centimetre; a crate held facing a 1.2 m wall stops at its face; the grace window holds for 9 ticks and ends. KA's caveat ("momentum or body shove?") is settled by the grace test: the arc is the velocity, and a body that keeps running into the landed crate still pushes it.

### release_packaging — 1 note(s)

- 2026-09-24 (2026-09-24-blue-comparison.md): `be2.py package`: zip + SHA-256 + commit (Python, cannot verify, wall-clock timestamps) — `package` / `package --verify`: reproducible bytes, dirty-tree refusal, symlink refusal, feature-isolated binaries, graphics-free check re-run at verify time (ADR 0032) — Not signed; the manifest records what was built, not that tests passed

### scene_format — 1 note(s)

- 2026-09-30 (2026-09-30-agent-feedback-building-great-outdoors.md): Many small edits — Adding one concept touched many files (scene key list, `check_sections`, `Scene` literal in two places, `describe`, SPEC, protocol, server, session, client, HUD, features.json), and a regex mass edit corrupted signatures. — `PlayerSnap` and `Snapshot` derive **`Default`** (a test builds them with `..Default::default()`); the rest is still open (below).

### self_description — 1 note(s)

- 2026-09-24 (2026-09-24-cheddar-feedback.md): `doctor` (read-only environment report) — BlueEngine `tools/be2.py doctor` — yes, as `red_engine2 doctor`, probing GPU (hardware then software), audio, ffmpeg, UDP, output dir, git

### sim_core — 1 note(s)

- 2026-09-27 (2026-09-27-transport-threat-model.md): Full snapshot, 8 players + 29 props — 1205 B (+8 tag on UDP) — yes — no: `protocol::snapshot_prop_budget` caps props (8 players leave room for 25 at 1150 B); the rest go in the next snapshot

## Unassigned (no feature matched)

### unassigned (no feature matched) — 2 note(s)

- 2026-09-27 (2026-09-27-transport-threat-model.md): Eavesdropping — Encrypted (TLS 1.3 AEAD) — Readable, as before
- 2026-09-27 (2026-09-27-transport-threat-model.md): lan — 5-6 ms, 0%, 3958 — 5-6 ms, 0%, 3957 / 3956 — 5 ms, 0%, 3635 / 6372
- 2026-09-27 (2026-09-27-transport-threat-model.md): wifi — 16-18 ms, 1.7-2.6%, 3977 — 15-17 ms, 1.7-2.6%, 3977 / 3976 — 17 ms, 0.8-1.7%, 3654 / 6392
- 2026-09-27 (2026-09-27-transport-threat-model.md): 4g — 51-55 ms, 3.4-5.1%, 3988 — 53-56 ms, 3.4-5.1%, 3987 / 3985 — 48-50 ms, 2.1-2.8%, 3724 / 6784
- 2026-09-27 (2026-09-27-transport-threat-model.md): bad — 107-108 ms, 6.8-7.5%, 4022 — 100-103 ms, 7.5-13.9%, 4033 / 4031 — 100-103 ms, 6.2-8.8%, 3728 / 7224
- 2026-09-27 (2026-09-27-transport-threat-model.md): awful — 183-204 ms, 30.9-32.8%, 4141 — 173-210 ms, 30.6-32.4%, 4127 / 4126 — 170-183 ms, 28.6-33.0%, 4003 / 7981
- 2026-09-29 (2026-09-29-build-time-baseline.md): idle box — 4 — 22 / 44 / 48 / 86 — 7 182
- 2026-09-29 (2026-09-29-build-time-baseline.md): idle box — 8 — 51 / 478 / 579 / 666 — 13 750
<!-- analysis-index:end -->
