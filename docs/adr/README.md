# Architecture Decision Records

Short records of *why* Red Engine 2 is built the way it is, so nobody re-derives intent from code.
`red_engine2 search <topic>` indexes them (kind `adr`); `red_engine2 adr list` prints the table below. Read the one-line summaries first.

The table is **generated** from the files (`red_engine2 adr index --write`, also done by `adr new` and `preflight --fix`); a test fails when it is stale. Edit an ADR's
`Summary:` line, never the table.

<!-- adr-index:begin -->
| # | Decision | Status |
|---|---|---|
| [0001](0001-json-scenes-and-self-describing-cli.md) | Maps are JSON; the engine describes itself so AIs never read Rust | accepted |
| [0002](0002-props-in-rust-prefabs-in-json.md) | Props (custom collision) are Rust; everything else is JSON prefabs | accepted |
| [0003](0003-one-physics-shared-by-game-and-tools.md) | The game and the analysis tools run the same physics functions | accepted |
| [0004](0004-reachable-ground-height-for-floors.md) | Floors work by "highest reachable surface", not floor bookkeeping | accepted |
| [0005](0005-fixed-step-physics-with-camera-interpolation.md) | Fixed 1/60 s player physics, interpolated camera | accepted |
| [0006](0006-parse-time-expansion-for-sugar.md) | `wall`/`fence`/`prefab` expand to plain objects at parse time | accepted |
| [0007](0007-docs-cannot-drift.md) | Docs are generated or test-checked; source index is on demand | accepted |
| [0008](0008-procedural-assets-only.md) | Meshes and sounds are generated in code, nothing imported | accepted |
| [0009](0009-four-maps-one-asset-library.md) | Four maps (house, school, office, store) share one library | accepted |
| [0010](0010-headless-match-server.md) | Headless multiplayer match server (historical proposal; built as 0016/0017) | superseded by 0016 and 0017 |
| [0011](0011-two-characters-and-exact-melee-hits.md) | Human + Cheddar the rat, a launch menu, exact melee hit-testing | accepted |
| [0012](0012-rapier-prop-physics-dormant-until-disturbed.md) | Loose props run on rapier and stay dormant until disturbed | accepted |
| [0013](0013-revolver-second-weapon-hitscan-infinite-ammo.md) | Silver revolver: mouse-wheel second weapon, hitscan, infinite ammo for now | superseded by 2026-09-28-remove-the-revolver |
| [0014](0014-sim-foundations-fixed-tick-scratch-change-tracking-static-props.md) | Sim foundations: 60 Hz tick + tick-based weapons, scratch buffers, change tracking, static props | accepted |
| [0015](0015-engine-first-refocus-test-lab-and-legacy-maps.md) | Engine-first refocus: the Red Test Lab is the dev map, the four maps are legacy, roadmap | accepted |
| [0016](0016-authoritative-udp-multiplayer.md) | Authoritative UDP multiplayer: server, delta snapshots, prediction, interpolation, reconnect | accepted |
| [0017](0017-headless-build-gfx-feature.md) | The dedicated server builds without graphics: the `gfx` feature, `collide`/`geometry` extraction, CI-enforced | accepted |
| [0018](0018-self-describing-engine-pattern.md) | The self-describing engine pattern (describe/search/catalog/verify): a reusable design | accepted |
| [0019](0019-strict-fields-versioning-and-json-envelope.md) | Unknown scene fields are errors, a schema version, and one `--json` envelope for every command | accepted |
| [0020](0020-game-rules-as-data-and-headless-scenarios.md) | Game rules as data (`vars`/`rules`), proven by headless scenarios (`sim`, `checks.sim`) | accepted |
| [0021](0021-deterministic-traces-replay-and-checksums.md) | Deterministic traces, replay, and split checksums: the first divergent tick | accepted |
| [0022](0022-authoritative-interactions-per-prop-acks-and-interest.md) | Authoritative pick-up/combat, per-prop acknowledgement, rooms-and-portals interest management | accepted |
| [0023](0023-walk-failures-name-their-blocker-routes-are-planned.md) | Walk failures name their blocker (id, gap, passage width); routes are planned (`walk --auto`), not guessed | accepted |
| [0024](0024-the-framework-layer-blueprints-and-game-projects.md) | The framework layer: blueprints compile to complete self-checking maps; games are projects that pin the engine, not forks | accepted |
| [0025](0025-handoff-status-and-derived-doc-facts.md) | Handoff (`status`, STATUS.md) and doc facts derived from the repo, checked by a test | accepted |
| [0026](0026-a-headless-audited-ui-kit.md) | A headless, audited UI kit: one layout drives painting, clicks and the audit; `ui-shot`, `ui-check` | accepted |
| [0027](0027-runs-anywhere-doctor-env-config-container-lf.md) | Runs anywhere: `doctor`, env-var server config, container image, software GPU fallback, LF | accepted |
| [0028](0028-authenticated-datagrams-and-join-keys.md) | Authenticated datagrams and join keys: challenge/response, HMAC tags (protocol v3) | accepted |
| [0029](0029-match-flow-lobby-rounds-rematch.md) | The match flow: lobby, ready-up, countdown, rounds, results, rematch; the online screens | accepted |
| [0030](0030-performance-is-a-contract.md) | Performance is a contract: `perf` and `checks.perf` | accepted |
| [0031](0031-home-hosting-with-upnp.md) | Home hosting with UPnP: `portmap` and `red_server --upnp` | accepted |
| [0032](0032-provable-releases.md) | Provable releases: `package` and `package --verify` | accepted |
| [0033](0033-feature-index-and-impact.md) | A feature index that cannot rot, and `impact` | accepted |
| [0034](0034-net-test-and-bad-network-resilience.md) | `net-test` and bad-network resilience (bursty loss, bounded extrapolation) | accepted |
| [0035](0035-rule-state-in-the-standard-client.md) | Shared rule state drives standard-client visibility, effects and a generic HUD | accepted |
| [0036](0036-repeated-bounded-online-rule-state.md) | Repeated bounded rule-state snapshots give online clients loss/reconnect/late-join recovery | accepted |
| [0037](0037-core-assets-are-a-growing-api.md) | Core assets are a growing API, not an exhaustive inventory | accepted |
| [0038](0038-reusable-firearm-arsenal-and-ads.md) | Reusable firearm arsenal (eleven guns when written, ten since the revolver left), procedural models and smooth aim-down-sights | accepted |
| [0039](0039-authored-player-tuning-and-jump-pads.md) | Scene-authored player tuning, starting weapon and deterministic jump pads | accepted |
| [0040](0040-shooter-presentation-and-momentum.md) | Shooter sight anchors, automatic fire, pellets and replicated momentum | accepted |
| [0041](0041-native-input-and-sandbox-workflow.md) | Native controller input, human-rig costumes and the sandbox project workflow | accepted |
| [0042](0042-proportional-verification-and-unattended-runs.md) | Verification proportional to the change (`affected`), 5-15 KB `context` packets, loopback-only unattended runs | accepted |
| [0043](0043-a-reusable-client-layer-for-custom-games.md) | A reusable client layer (`red_engine2::app`: view camera, local session, HUD state, window/GPU/input, offscreen checks, a small shell) for games that are not first-person, proven by an external top-down example crate. | accepted |
| [0044](0044-encrypted-quic-transport-and-dev-udp.md) | Production multiplayer traffic is QUIC + TLS 1.3 (quinn + rustls) behind a transport boundary, with a pinned server identity, join keys bound to the TLS exporter and no downgrade; loopback keeps development UDP, and a public UDP bind must be asked for explicitly. | accepted |
| [0050](0050-bots-weapon-ladder-and-combat-pacing.md) | Bots (AI players with a brain), the server fills empty slots, a nav graph, the weapon ladder (Gun Game), the `combat` pacing block | accepted |
| [0051](0051-shooter-feedback-on-the-wire.md) | Shooter feedback on the wire (protocol v8): shot, hit, hurt and kill counters, damage bearing, respawn countdown; the client turns them into events | accepted |
| [0052](0052-shooter-feel-in-the-client.md) | Shooter feel in the client: a headless `feel` state machine, a synthesized sound bank, a screen-effects pass and a combat HUD | accepted |
| [0053](0053-lag-compensation.md) | Lag compensation: the server rewinds the players a shot can hit by the shooter's view lag (interpolation delay plus round trip), recorded in traces | accepted |
| [0054](0054-hosting-from-the-client.md) | Hosting from inside the client: `re2 --host` serves the map on a thread of the game and joins it, so bots need no second process | accepted |
| [0055](0055-bullet-tracers-and-sparks.md) | Bullet tracers and impact sparks: a pool of glowing boxes in the scene shows where every hitscan shot went | accepted |
| [0056](0056-music-in-code.md) | Music composed in code: an eight-bar synthwave loop generated at start-up, tested for pitch, rhythm and a seamless wrap | accepted |
| [0057](0057-avatars-for-every-body.md) | The avatar pool covers every body somebody can wear, bots' included, with a stand-in for any it lacks: no enemy goes undrawn | accepted |
| [2026-09-28-generated-bookkeeping](2026-09-28-generated-bookkeeping.md) | Every registry CI bounced on is generated or derived, and `red_engine2 preflight` finds the rest in a second and prints the exact edit. | accepted |
| [2026-09-28-remove-the-revolver](2026-09-28-remove-the-revolver.md) | The silver revolver, the one weapon with its own model file, constants, sound and scene key, is gone; ten data-driven firearms remain and `weapons.ammo` replaces `weapons.revolver.ammo`. | accepted |
| [2026-09-28-seeing-what-the-player-sees](2026-09-28-seeing-what-the-player-sees.md) | The real client can be stepped without a window, photographed offscreen, played by a script and asked what it draws; every "skip it and carry on" now counts itself and `lint`/`game check` catch the classic causes. | accepted |
| [2026-09-29-authoring-ergonomics-from-the-physics-games](2026-09-29-authoring-ergonomics-from-the-physics-games.md) | Scenario hold steps take look_at, blueprint scene.zones merge by id, vars starting with _ stay off the generic HUD, info says whether an object is loose and why, checks.lint takes ignore, and describe/SPEC state zone edge order and topple direction. | accepted |
| [2026-09-29-develop-on-a-path-ship-on-a-pin](2026-09-29-develop-on-a-path-ship-on-a-pin.md) | A game follows a local engine checkout while it is developed and is pinned to an exact engine commit when a version ships; game pin and game unpin switch between the two. | accepted |
| [2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature](2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature.md) | A kart is a movement mode of the existing player: a pure kart step shared by server and prediction, per-animal specs as data, per-player race progress and three pickups in the simulation; input reuses the player input packet. | accepted |
| [2026-09-29-one-release-velocity](2026-09-29-one-release-velocity.md) | A carried prop leaves the hand with the holder's own velocity plus player.throw_speed along the look, through one function on the client and the server; the holder's body ignores it for 10 ticks and the hold pose sweeps the prop's box, not a ray. | accepted |
| [2026-09-29-open-worlds-peaceful-play-and-a-real-sea](2026-09-29-open-worlds-peaceful-play-and-a-real-sea.md) | A `world` block makes one axis loop (the seam is invisible: drawn copies, mirrored collision, short-way-round rules and interpolation) and clamps the others; `terrain` objects are walkable heightfields; `sky` and `ocean` draw a sun at infinity and water to the horizon; `hud` and `player.mode` remove the shooter's screen and weapons; F toggles fullscreen from every screen. | accepted |
| [2026-09-29-prop-aware-rules-and-scenarios](2026-09-29-prop-aware-rules-and-scenarios.md) | Rules and sim expectations see loose props: prop_enter/prop_exit/prop_below triggers, prop_y/tilt/held/mass/moved/props_in built-ins, reset/place actions, swing/prop_hit events, and {prop: ...} expectations with prop rest poses in the report. | accepted |
| [2026-09-29-replay-applies-each-shove-once](2026-09-29-replay-applies-each-shove-once.md) | A trace records inputs and external pushes only; strikes, bullets and rule impulses are outputs the replay re-derives, so it applies each shove exactly once. | accepted |
| [2026-09-29-verification-honours-the-map](2026-09-29-verification-honours-the-map.md) | lint, reach, walk, build and offline play start at the first spawn at its height, walks use the scene's player tuning and jump pads, and an unconditional start rule that disables collision is open to the tools too. | accepted |
| [2026-09-30-bounded-iteration-explicit-configuration-and-safe-green](2026-09-30-bounded-iteration-explicit-configuration-and-safe-green.md) | scripts/dev iterate is a partial tier that can never count as verification; plans state their features and toolchain and stamps key on them; the feature index is read from the checkout so planning is never stale; preflight can run tree-only; heavy jobs take a lock. | accepted |
| [2026-09-30-evidence-driven-refinements-from-the-external](2026-09-30-evidence-driven-refinements-from-the-external.md) | A grid index for ground lookups (reach spent 92% of its time there), a total order for lint findings, an in-memory virtual-clock network, sim::env with line-of-sight observations, a property test with shrinking for prop ownership; sccache rejected; the rest kept as reference or experiments. | accepted |
| [2026-09-30-force-fields-and-material-opacity](2026-09-30-force-fields-and-material-opacity.md) | Scene fields push loose props toward a target speed (currents, belts, wind); material.opacity blends see-through surfaces; underscore variables are declarable. | accepted |
| [2026-09-30-killchain-loadout-shooter](2026-09-30-killchain-loadout-shooter.md) | A scene with a shooter block runs a loadout team deathmatch: 31 weapons with per-gun ammo, two teams, pickups, grenades and rockets, a killcam, and its own client shell. | accepted |
| [2026-09-30-no-character-selector](2026-09-30-no-character-selector.md) | The engine has no character selection screen (launch menu or lobby toggle); a player is the scene's humans_play_as, else --as, else the Human. | accepted |
| [2026-09-30-publishing-to-redenginegames-never-deletes-what-it-does](2026-09-30-publishing-to-redenginegames-never-deletes-what-it-does.md) | publish_games.py export writes the files the manifest publishes and removes only files an earlier export published (per its catalog); everything else in games/, prototypes/, tests/ and demos/ is left alone, because the old delete-the-folder behaviour would have wiped hand-added games on the next engine push. | accepted |
| [2026-09-30-red-engine2-servers-manages-the-game-servers-on-this](2026-09-30-red-engine2-servers-manages-the-game-servers-on-this.md) | A thin, Linux-only layer over per-user systemd units whose ExecStart is red_server: list, status, start, stop, restart, logs, with player counts from the server's own stats line, a --yes gate when players are connected, and no secrets printed. | accepted |
| [2026-09-30-the-engine-says-what-it-did](2026-09-30-the-engine-says-what-it-did.md) | `in_zone(prop, zone)` and `deactivate`/`activate` remove the two workarounds every rules game wrote; the 14 m/s prop cap, the prop collider model and a failed pick-up are now stated by `describe`, `lint` and `sim` instead of discovered by measuring; `info` reports the mass `mass(id)` reads. | accepted |
| [2026-10-01-a-new-monster-character-hollow-and-the-avatar-pool](2026-10-01-a-new-monster-character-hollow-and-the-avatar-pool.md) | A genuinely distinct, unsettling costume (not a recolor) reusing the human rig, and the fix for a latent out-of-bounds panic in the avatar pool that any 9th Character would have hit | accepted |
| [2026-10-01-a-player-carried-flashlight](2026-10-01-a-player-carried-flashlight.md) | The player can carry a toggleable point light that follows the camera; the engine has no spotlight kind, so it is a Point, not a cone | accepted |
| [2026-10-01-a-scene-customizable-elimination-title](2026-10-01-a-scene-customizable-elimination-title.md) | The death screen's ELIMINATED title is now optional per scene (death_text); a genre-neutral engine should not hard-code one genre's words | accepted |
| [2026-10-01-game-upgrade-planning-and-staged-verification](2026-10-01-game-upgrade-planning-and-staged-verification.md) | A read-only upgrade planner matches known compatibility changes and writes a packet; staged verification builds the target engine in isolation and never writes over the real project | accepted |
| [2026-10-01-killchain-player-feedback](2026-10-01-killchain-player-feedback.md) | Killchain gains sprint controls, clear team spawns, optic reticles, a synchronized knife slash, directional strides, and restrained feedback without footsteps. | accepted |
| [2026-10-01-persisted-per-game-audio-settings-and-a-music-download](2026-10-01-persisted-per-game-audio-settings-and-a-music-download.md) | The pause menu's MUSIC/SOUND toggles are saved per game and reloaded on relaunch; a small arrow button saves the generated music loop as a WAV wherever the player chooses | accepted |
| [2026-10-01-team-aware-rules-and-generic-team-assignment](2026-10-01-team-aware-rules-and-generic-team-assignment.md) | A scene can opt into team 1/2 assignment outside a loadout shooter match, so who: team1/team2 works in rules for any two-sided game | accepted |
| [2026-10-02-a-cross-game-friction-digest-for-docs-analysis](2026-10-02-a-cross-game-friction-digest-for-docs-analysis.md) | analysis digest groups every analysis note's feedback by the engine feature it matches, ranked by how many different games raised it, regenerating docs/analysis/README.md the same way the ADR index is generated. | accepted |
| [2026-10-02-a-relay-for-players-behind-carrier-grade-nat-with-a](2026-10-02-a-relay-for-players-behind-carrier-grade-nat-with-a.md) | red_relay forwards opaque bytes between a registered host and a resolving client by a 6-character code, so hosting/joining works even when UPnP cannot, without weakening ADR 0044's end-to-end encryption | accepted |
| [2026-10-03-relay-session-identity-nat-topology-and-resource-bounds](2026-10-03-relay-session-identity-nat-topology-and-resource-bounds.md) | Short-lived claim tokens replace IP-only pending association, relay-to-host traffic is reframed to travel over the one socket a host's own registration already opened a NAT mapping for, admission is explicitly bounded, and the relay's trust model is stated rather than assumed. | accepted |
| [2026-10-03-upgrade-results-must-earn-their-recommendation](2026-10-03-upgrade-results-must-earn-their-recommendation.md) | game upgrade verify distinguishes requested-scope success, candidate verification, and application readiness, so a baseline-only or partial check can never recommend pinning an unverified target. | accepted |
<!-- adr-index:end -->
| [0043](0043-a-reusable-client-layer-for-custom-games.md) | A reusable client layer (`app`): any camera, local session over `MatchSim`, window/input/HUD pieces, an external top-down example | accepted |
| [0044](0044-encrypted-quic-transport-and-dev-udp.md) | Encrypted QUIC transport behind a transport boundary; server identity separate from join keys; development UDP explicit | accepted |

## Writing one

```bash
red_engine2 adr new "Lag compensation" --summary "The server rewinds the players a shot can hit by the shooter's view lag."
```

That creates `docs/adr/2026-09-28-lag-compensation.md` (today's date, so an ADR written on another branch at the same moment cannot take its number: the four-digit
numbers of 0001-0057 are legacy) and refreshes the index. Nothing else is registered by hand: the library embeds the folder at build time. Fill in the three sections and keep it
under ~60 lines; link code by symbol name (`red_engine2 src show <symbol>`), not line number. Refer to it in code and docs by its id (`0057`, or the whole `2026-09-28-lag-compensation`).

```
# 2026-09-28. Title
Status: accepted | proposed | superseded by <id>
Summary: one sentence; it is the row in the index above.
## Context      what forced a decision
## Decision     what we do
## Consequences what gets easier / harder; how to undo it
```
