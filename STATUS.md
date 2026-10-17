# STATUS — red_engine2

_Handoff file for whoever (human or AI) resumes this work. Keep it short and current: update it at every checkpoint with
`red_engine2 status --note "what changed" --section done|now|next|blocked|notes`. `scripts/dev preflight` warns when it is more than 20 commits behind main._

## Now (in flight)
- 2026-10-09: HARDENING PASS (the review of 2026-10-09: stale docs, a cold compile as the first step, scope drift). One pull request per item, merged on the owner's instruction after hosted CI passed:
  (1) entry docs true and consistent, (2) prebuilt binaries instead of a cold compile, (3) hermetic Idea Forge tests, (4) this file, (5) Killchain out of the engine crate, (6) browser leftovers, (7) bad input is an error, (8) CI growth and the rules-language gaps written down.
  All ten pull requests are merged (#74-#83); main was green on hosted CI after the last one. What each changed is in its PR and ADR.
- 2026-10-09 cleanup pass: #90 reverted #89 (a CI-measurement probe base, author `probe@example.invalid`, merged into main by mistake; main's tree returned to 11f1bcb); #85 and #88 merged after hosted CI, on the owner's yes.
- Reviewed, not merged (review comments on the PRs): #37 `savedata` (worth merging after three textual conflicts; the id `new-game` derives from the name can collide), #53 `hot-reload` (the browser half has no target since #55; the native `re2` half still builds and its tests pass on main: re-cut it as a smaller PR or close).
- Merged #85: joining by the relay's six-character code carries the host's admission key (ADR 2026-10-09) and the relay's keepalive, goodbye and lease are fixed; the launchpad reads git as NUL-delimited data;
  Idea Forge agents run under a deadline; a `python-tools` CI job runs the Python tools on Linux and Windows. **The deployed `red_relay` is still the old build** until it is redeployed (the live unit is the user service `red-relay`, binary `~/.local/share/red-relay/bin/red_relay`; `deploy/red-relay.service` is the system-wide template).
- Multiplayer stabilization brief (`docs/feedback-2026-10-07-stabilization-brief.md`, filed against f58e15d): not implemented as a whole. Item 1 (short-code join with a key, relay lease) and the source-fingerprint half of item 2 are merged (#85);
  git shows no commits for the rest of item 1 (map retry through the relay, search-and-destroy reconnect, duel capacity after replacement), item 2's executable-selection and discovery parts, item 3 (replay coverage of the objective modes) or item 5.
  Item 4 (native 2D delivery) is partly there (`re2d` and the native publish route, #55); the packaged-executable walk-through it asks for has not been done.
- Idea Forge (`docs/IDEA_FORGE.md`) runs nightly on the owner's machine, opens up to two game PRs a night and merges them when CI is green (#58-#73 so far); its backlog of engine findings is in `docs/analysis/idea-forge/`.
- 2026-10-11: Agent loop trace phase 1: three tasks traced (docs/analysis/agent-loop-trace.md); top sinks: launchpad routing, affected false green (describe guard tests unowned), 3D search domain; awaiting review before phase 2
- 2026-10-13: Phase 0 of the next phase: the outside review of b5b458f verified at 4c45d86 (docs/analysis/2026-10-13-next-phase-review-verified.md) and ADR 2026-10-13-the-next-phase sequencing phases 1-3 with measured done criteria; accepted by the owner; Phase 1 item 1 next

## Done
- 2026-10-09: Idea Forge runs Midnight Engine (#72) and Salted Choir (#73, feedback only: the game did not build).
- 2026-10-08: Idea Forge (#58-#71): the idea generator, the CLI pipeline (idea, AI builds, feedback), daily and nightly jobs, auto-merge, a settle log, test isolation; `start` now says up front that a missing executable means a cold build (#62).
- 2026-10-07: S4, browser removal, done: a native 2D player (`re2d`, `play2d`) and the browser target removed (#55), leftovers cleaned (#56, more in hardening item 6); CI on Node 24 actions (#57); the server loop blocks on the transport (1271001).
  Killchain game modes (free for all, capture the flag, search and destroy, duels; protocol v15) were pushed straight to main in f58e15d, after the simplification baseline.
- 2026-10-06: the AI launchpad `scripts/dev start|next|resume` (#51), a native MCP server (#52), core-first orientation (#49), per-player rule variables (#50), the development efficiency pass (#46), browser reliability (#47; the browser itself was removed on 10-07).
- 2026-10-06: simplification stages S1 (native 2D honest boundary, publish kind 2d), S2 (`mcp` feature), S3 (`video` feature).
- 2026-10-07: the RedEngineGames branch `native-2d-downloads` has landed there; RedEngineGames syncs from RedEngine main at every push.
- 2026-10-10: 2026-10-10: CI wall-clock 23.1 -> 4.6 min per PR (#97: Windows build+Direct3D on PRs, nextest, 5+3 shards); Idea Forge publishes merged games with screenshots to RedEngineGames (#96); style block, style lint, style-report, voxels (#98); story primitives next (#99)
- 2026-10-11: Agent-loop phases 2+3: fixes #103 (affected includes), #104 (start routing), #105 (search domain); uniqueness: no own asset unprompted, own prefab ~20x a library prop; proposal = lint style-own + recipe own_prefab (not built)
- 2026-10-11: Agent-loop fix 3 (search domain): lead lane holds 2D mechanics and 3D recipes by the domain a query names; ADR 2026-10-11-search-leads-with-the-domain-a-query-names
- 2026-10-11: Agent-loop fix 2 (start routing): scene route, new-2D-game cue, owners by file/command, directory names; ADR 2026-10-11-the-launchpad-routes-by-what-a-task-names
- 2026-10-11: Agent-loop fix 1 (affected false green): compile-time includes are planned as changes; ADR 2026-10-11-compile-time-includes-are-changes
- 2026-10-11: Wormhole (examples/wormhole): sky.shader, audio.journey, player.still added; feedback in docs/analysis/2026-10-11-wormhole-feedback.md
- 2026-10-11: Chroma (examples/chroma): audio.playlist (picker, drag and drop, symphonia), sky hears music, frame --song/--seed; feedback docs/analysis/2026-10-11-chroma-feedback.md
- 2026-10-13: Aimed fields + the rule's subject prop (ADR 2026-10-13-aimed-fields-and-the-rule-s-subject-prop): fields[].toward/speed/props, aim action, self + prop lists on prop triggers and pickup/drop/prop_hit; field pushes computed in RulesEngine so offline re2 now applies fields too (it never did); Velvet Orchestra 94 generated rules -> 33 + 1 field with the card's real mechanic; recipe whistle_herd reuses it. Report: docs/analysis/2026-10-13-aimed-fields-and-subject-props.md
- 2026-10-13: Phase 1 item 1 (docs at runtime): tools::own_docs reads SPEC/AGENTS/REFERENCE/PLAY_2D/GLOSSARY/ADRs/analysis/features.json from the checkout; build.rs embeds only for release-type builds; a doc edit 86-108 s -> 0.3 s, no compile (docs/analysis/2026-10-13-docs-read-at-runtime.md)
- 2026-10-13: Phase 1 item 2 (prebuilts): release.yml publishes prebuilt-<fingerprint> pre-releases on every main push whose fingerprint has none (skips docs/games-only commits), keeps 10; prebuilt.py no longer counts the engine's own docs as inputs
- 2026-10-13: Phase 1 items 1-2 merged (#118 docs at runtime, #119 prebuilts). First prebuilt-a7c3e49c9bcf published from main in 773 s; a fresh clone reaches describe --brief in 3.9 s + 0.09 s with no compile (docs/analysis/2026-10-13-prebuilt-on-main-measured.md). Next: item 3, red_core
- 2026-10-13: Phase 1 item 3, step 1: crates/red_core holds the ten files red2d shared by #[path] (synth, dsp, audio_fx, audio_analysis, voice_spec, score, fields, suggest, font_glyphs, sim::rules_expr); engine re-exports them at their old paths; red2d 127 -> 69 tests (58 no longer compiled twice); ADR 2026-10-13-red-core-is-one-crate-the-engine-and-its-games-share
- 2026-10-13: Phase 1 item 3, step 2: red_core takes color, easing, crypto, sim::{clock,scratch,components,change,entities,snapshot,statics,spawns,approach}, net::{limits,netsim,upnp,join_code,auth,relay} (+DEFAULT_PORT); [workspace.dependencies] gives every crate one spec per shared dependency; core sim/net deny unwrap/panic like the engine's
- 2026-10-13: item 3 step 2: PR #122 open (sim foundations, crypto, net pieces into red_core; [workspace.dependencies]); local CI OK except split_render timing under load (passes alone); waiting on hosted CI
- 2026-10-13: item 3: #123 merged (src deps --cycles/--core/--reach); cut plan: server reaches 102 modules, 7 crossings, 70 modules to move; cut 1 portmap PR open
- 2026-10-13: item 3: cuts 1-3 merged (#124 portmap, #125 font, #126 shader-check); scene-block seam built, parity 10/10 identical; full CI next
- 2026-10-14: item 3: move 2 merged (#129): ring of 55 + ground/checksum into red_core (96 modules, 47k lines), parity identical, tests 1659->1660 (+1 new); next: edge-reader fix, move 3 (network layer + quic feature), then red_server from core
- 2026-10-14: item 3: moves 1-3 merged (#128,#129,#131), red_core = server tree but the binary; iterate core-aware: sim edit 9-14 s warm (was ~50); next: red_server crate
- 2026-10-14: item 3 closed: ADR 2026-10-14-the-server-binary-stays-in-the-engine-crate-while-red (measures met but the headless-build clause, deferred to item 5's game servers); next: item 4 CI
- 2026-10-14: item 4: #134 (compile once: Rust PR 57.0 -> 32.8 job-min), #135 (docs-only PRs from main's archive), #136 (split_render flake) merged; this docs-only PR measures the docs path
- 2026-10-14: item 5 started (owner: go ahead): ADR a-game-ships-as-its-own-binary; steps 5.1 client to library, 5.2 GameMode, 5.3 WorldLayer, 5.4 game binaries, 5.5 Frameless to games/frameless, 5.6 audit
- 2026-10-14: item 5.2: client::GameMode seam + MuseumMode (branch game-mode); parity: frameless_visit + the same dump and saves as main for the same script (door-event print order varies in main too); affected green
- 2026-10-14: item 5.3: world_layer::WorldLayer (PR #147); renderers hold layers, museum types out of render.rs/viewer.rs; parity scripts/picture_parity.py: offline 9/9 byte-identical, live within old run-to-run spread; 5.2 merged (#146)
- 2026-10-14: item 5.4: game.json "binary" (PR game-binary): play-local/publish/package/scripts/red know a game's own client; checked on Killchain's project (KC_SCRIPT run via scripts/red play-local); RedEngineGames PR follows for the owner; 5.3 merged (#147)
- 2026-10-14: item 5.5a: BlockDef art_file/endless/describe (branch frameless-seams); core no longer names museum in code; 5.4 merged (#148), RedEngineGames PR #7 for the owner
- 2026-10-14: item 5.5a merged (#149: BlockDef art_file/endless/describe); 5.5b-pre: validate leaves games/<crate> content to the crate (PR)
- 2026-10-14: item 5.5: Frameless moved to games/frameless (own crate, binary, tool, tests, content; PR). Parity: offline pictures identical, visit dump/save identical, old re2 save loads in frameless. Ships from its crate once RedEngineGames PR #7 merges
- 2026-10-14: item 5.5 merged (#151: src/museum gone; Frameless ships from its crate once RedEngineGames PR #7 merges). 5.6: audit + ADR a-system-stays-in-the-engine-when-two-games-use-it-or (PR): loadout shooter moves to Killchain, wire structs first
- 2026-10-14: item 5.6 merged (#152 audit + ADR); protocol v16 GameMsg + net::arena_wire (PR): the loadout shooter's first step out
- 2026-10-14: shooter move step 1: loadout parity harness + ADR a-game-plugs-into-the-server-s-match-simulation-the (PR); 6 steps planned
- 2026-10-14: shooter move step 2: sim::game::MatchGame + ShooterGame (PR); parity fingerprints unchanged
- 2026-10-14: shooter move step 3: bot minds (MatchGame::bot_mind, ShooterBot) (PR); step 2 merged (#155); parity unchanged
- 2026-10-14: shooter move step 4a: server flow + snapshots via MatchGame hooks (PR); step 3 merged (#156); parity unchanged
- 2026-10-14: shooter move step 4b: shooter block + GameDef registry (PR); 4a merged (#157)
- 2026-10-14: shooter move step 5: server_app library + HostOptions::scene_text (PR); 4b merged (#158)
- 2026-10-14: shooter move step 6a: client game part + decoder, snapshot History, killcam cuts from it (PR); step 5 merged (#159)
- 2026-10-14: shooter move step 6b: public match door + ShooterSim + uniforms hook (PR); step 6a merged (#160)
- 2026-10-14: CLI is a library (ADR a-game-runs-the-engine-s-commands, step 1) (PR); step 6b merged (#161)
- 2026-10-14: game systems milestone (ADR game-systems-compose-under-one-match-authority): sim::defs/ledger/systems, Workshop reference game (games/workshop), ability reuse fixture, sim game expectations + --inspect, describe systems; fresh-agent trial passed (69.5K tokens, 19 calls, 325 s, 1 tool-caused error)
- 2026-10-14: game systems milestone: evidence ledger, update-path audit and dependency-ordered roadmap in docs/analysis/2026-10-14-game-systems-milestone.md (PR); #165 merged
- 2026-10-15: Review follow-up item 1 (branch dismantle-atomic): a refused ws:dismantle with room for the input but not the output changed the hash (tick 2160); fixed with Ledger::remove_stores (one transaction) and Systems::command restoring ledger, system state, rng and queue on Err; probe_dismantle.rs + partial-room case + host unit test
- 2026-10-15: Review follow-up item 2 (branch ledger-props): ledger property tests, 60k seeded steps vs a model, 0.47 s, two mutations caught; docs/analysis/2026-10-14-ledger-properties.md
- 2026-10-15: Review follow-up item 3 (branch interned-ids): Id interned (Copy, ordered by text), ledger slots + in-place tx with undo, Workshop resolves once; N=1000 p95 1.58 ms -> 18 us, 19,539 -> 0 allocations/tick, golden hashes + describe systems + inspect output identical; docs/analysis/2026-10-15-interned-ids-measured.md. Friction: cli validate content/workshop_plus.json from games/workshop cannot find defs/expansion.json (works from content/); scripts/dev start routed this task to no workflow

## Next
- 2026-10-13: **The next phase** (ADR 2026-10-13-the-next-phase, accepted by the owner 2026-10-13): the review of b5b458f verified in `docs/analysis/2026-10-13-next-phase-review-verified.md`.
  Phase 1, structural, in order: (1) [done, #118] docs read at runtime in dev builds (done: a doc edit rebuilds nothing; `search` sees a new ADR); (2) [done, #119; measured] a prebuilt for every build-input
  change on main, enabled by the owner 2026-10-13 (done: a fresh clone reaches `describe --brief` with no compile; a doc-only PR compiles nothing in CI); (3) `red_core` (done: headless build/clippy/tests compile only it, no
  `#[path]`, a sim edit through `iterate` under 20 s warm); (4) CI archive fan-out, headless PR shard, cache and Docker (done: measured against #115's 59 job-minutes);
  (5) shippable game crates + the generated-world and `GameMode` seams, Frameless out of the engine crate, the 30K-line audit (done: `src/museum` gone, parity kept).
  Alongside: AGENTS.md budget and split, file-length preflight, ADR bar, stale claims. Phase 2: 16-var lint, per-entity vars, restartable timers, fall-out height,
  movement/scale/body-swap rules, 3D strings, 3D spawn/destroy if needed (done: Gentle Carousel well under 100 rules; Prop Hunt with no engine Rust). Phase 3: shape ops,
  procedural materials, palette roles, `catalog --template`, own_prefab, style-own, scaffold text, the import ADR (done: own prefab at most 2x a library prop; two Idea
  Forge games in a row unlike each other). The bullets below are folded into it.
- Simplification S7 (offline `re2` orchestration through MatchSim/LocalSession): not done in the hardening pass on purpose; the offline loop (`src/bin/re2/frame.rs`) also does bat swings, carried props, the rules HUD and its own prop physics that `LocalSession` lacks, so it needs a design and a parity plan first. S5 (#80) and S6 (#79) are done with parity runs recorded in their PRs and ADRs.
- Killchain downstream: `red_engine2 package` / `game play` know only `re2`; the Killchain project in RedEngineGames must build and run `killchain` (games/killchain) instead.
- The rules-language gaps from the Idea Forge notes (per-entity state and timers, a fall-out height, rules that change player movement, gravity or scale): ranked and designed in `docs/RULES_LANGUAGE_GAPS.md` (#83); the prop-subject piece of gap 1 (`self`, `prop` lists) and aimed fields were built on 2026-10-13, the rest is not.
- Bounding the engine's CI time: 58% of CI job-minutes since Oct 7 went to Idea Forge (about 59 job-minutes a game). A games-only path filter (#88, merged: `games_only` in `ci.yml`'s `changes` job, `scripts/ci.sh games`) skips the Windows leg, the headless build and the image for a PR whose files are all under `examples/2d`, `examples/3d` or `docs/analysis/idea-forge`; one probe pair measured about 43 job-minutes (22 wall-clock) against about 2. Proposals 2-4 of `docs/analysis/2026-10-09-engine-ci-growth.md` (move generated games out, cap them, nightly measurement) are not built.
- 2026-10-13: Rules gaps after aimed fields: per-entity vars and restartable timers (RULES_LANGUAGE_GAPS rank 1) are the next milestone; the museum boundary proposal (procedural-model layer, game-neutral surface hook, a release route for a game crate) is in docs/analysis/2026-10-13-aimed-fields-and-subject-props.md section 5
- 2026-10-14: friction: `scripts/dev start` routes a task that says "migration" to the upgrade workflow (an engine-work task about save migration got `game upgrade plan`)
- 2026-10-14: friction (fresh agent): `validate` with no paths, run in a game folder outside a checkout, checks the engine repo's 90 files instead of the folder
- 2026-10-14: friction (fresh agent): `validate defs/x.json` reads a game's definitions file as a scene and gives misleading errors: a block that reads files has no way to declare their kind to `validate`
- 2026-10-14: friction: `sim --json --inspect` returns the whole scenario result (14 KB) to answer one question; an inspect-only output would be about 2 KB
- 2026-10-14: friction (fresh agent): `sim --only` matches a substring, so 'lever' also picked 'without the lever'
- 2026-10-14: next (measured bottleneck): compile game content into indexed runtime data: string-keyed lookups and per-tick allocation are about half of Workshop's tick at 1000 machines (perf)
- 2026-10-14: next (measured bottleneck): replication relevance/deltas for a game's part: a whole-state part is 8 bytes per machine and is left out of snapshots above about 170 machines

## Failing / blocked
- Main was green at 30bb4e5 (CI run of 2026-10-09 13:41). Not failing on main, but `scripts/test_idea_forge.py` failed on any machine with under 20 GB free (fixed in hardening item 3).
- Prebuilts: the owner chose (2026-10-10) to publish a prebuilt for every `main` commit that changes a build input, and on 2026-10-13 told the agent to enable it
  (Phase 1 item 2). Still no release exists, so `start`/`scripts/dev red` always build from source. Phase 1 item 2 of ADR 2026-10-13-the-next-phase.

## Decisions & gotchas
- The front door: README is a pointer, `AGENTS.md` is the single entry point (ADR 2026-10-09-the-front-door-documents-are-a-pointer-one-entry-point, hardening item 1); the claims in the docs are checked by `red_engine2 preflight`.
- Killchain's reusable shooter (arsenal, ordnance, killcam, `sim::shooter`, the objective modes) is engine by design (`docs/features.json`, ADR 2026-09-30-killchain-loadout-shooter); the Killchain game itself (client, screens, branding) is what S6 moves out.
- `cargo` is not on the default non-interactive PATH (`scripts/dev` adds it); a fresh worktree needs `scripts/dev worktree NAME` (seeded dependencies) or `CARGO_TARGET_DIR` pointing at a built tree, or its first build takes 7 minutes on four cores.
- The user's own `RedEngine` checkout has uncommitted work: agents work in worktrees and fresh clones.
- CI measurements: a probe PR targets a throwaway branch, never `main`, and the throwaway base branch is deleted right after; a pushed branch off `main` invites a "Compare & pull request" click (that is how #89 reached main). Close probes and delete their branches when done.
- `tests/net_sim.rs` `a_full_match_of_eight_holds_up_on_a_mobile_link` failed once on the Windows runner (#88, bandwidth 25351 B/s against a 24000 limit) and passed on re-run; not seen in the 60 runs before. Watch it; if it recurs the limit is too tight for Windows.
- 2026-10-15: Friction (item 3): scripts/dev affected's clippy did not lint tests/systems_abilities.rs (an engine-crate test target using sim::defs); hosted CI caught 9 clone-on-Copy lints
