# STATUS — red_engine2

_Handoff file for whoever (human or AI) resumes this work. Keep it short and current: update it at every checkpoint with
`red_engine2 status --note "what changed" --section done|now|next|blocked|notes`. `scripts/dev preflight` warns when it is more than 20 commits behind main._

_Rewritten from current facts on 2026-10-17 at main c4860ce (git, `gh pr list`, `gh release list`, the CI runs on main). The per-PR history it replaced is in git
(`git log -p STATUS.md`) and in each PR._

## Now (in flight)
- 2026-10-17: Phase 2 of ADR 2026-10-13-the-next-phase starts, in this order, one PR each: (0) a client-read rule var past the 16 the server sends is a validate
  error (Quiet Compass F1); keep the rule generators Idea Forge agents write; (1) per-entity rules: `groups` + `for_each` as parse-time expansion, then per-entity
  vars and restartable timers. Done = Gentle Carousel (252 rules, 30 vars today) in well under 100 rules with its board intact. Out of scope until (1) merges:
  game systems/Workshop, Phase 3, new games or genres.
- Open pull requests waiting on the owner: #170 interned ids for game systems (review follow-up item 3), #173 Honeybound v2, #167 narrative foundation
  (`.story` + story system + Tidewatch), #93 Redline feedback (18 backlog findings, open since 2026-10-10). RedEngineGames: #7 (Killchain plays with its own
  client; until it merges Frameless and Killchain ship the old way), #8 (installer icons).
- 2026-10-17: STATUS.md rewritten from current facts at c4860ce (PR #176); #92 closed as superseded
- 2026-10-17: Phase 2 item 0 (HUD var cap, PR #179): the server sends the variables the HUD reads first (no protocol change), lint net-hud for a HUD reading >16 or a name >32 bytes; Redline (RedEngineGames, offline) reads 42 and gets the lint error; cap raise proposed as ADR 2026-10-17-rule-variables-travel-by-index (not built)
- 2026-10-17: Item 3 (keep generators, PR #180): brief + ship paths + scripts/game_generators.py check (score, feedback --check, CI content job); 13 of 17 runs had lost a /tmp generator; a games-only PR with a generator no longer starts python-tools
- 2026-10-17: Phase 2 item 1 step 1 (PR #181): groups + for_each expanded at load (no protocol change); Gentle Carousel 252 rules / 102,527 B -> 13 entries / 34,413 B, expansion = old RuleSet minus 3 dead rules, sim output and checksums identical; verify 0.11 s before and after. Next: per-entity vars + restartable timers

## Done
- Phase 1 of the next phase is complete (ADR 2026-10-13-the-next-phase, #116-#153): docs read at runtime (#118), a prebuilt per build-input change on main (#119),
  `red_core` holds the headless server's tree (#121-#133), CI compiles the tests once (#134-#136), game crates ship their own binary and Frameless lives in
  `games/frameless` (#143-#151), the system audit (#152), protocol v16 `GameMsg` (#153); alongside: AGENTS.md split with byte budgets, the file-length check,
  the ADR bar, stale claims (#138-#142). Measurements are in `docs/analysis/2026-10-13-*` and `2026-10-14-*`.
- Shooter move to Killchain (ADR 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the): steps 1-6b and the CLI library merged (#154-#164).
- Game systems (ADR 2026-10-14-game-systems-compose-under-one-match-authority): `sim::defs`/`ledger`/`systems` and the Workshop reference game (#165, #166);
  review follow-ups 1 (a refused command changes nothing, #168) and 2 (ledger property tests, #169).
- Idea Forge: 17 runs, two a night from 06:00 on the owner's machine, merged and published by the tool when CI passes; latest Quiet Compass (#171), Final
  Staircase (#172), Sunken Marathon (#174), Electric Carousel (#175). Findings: `docs/analysis/idea-forge/`, backlog `fixes.json`.
- Carried over from #92 (closed unmerged 2026-10-17, superseded by this file): #37 `savedata` merged 2026-10-10 (a game's settings and progress are filed under
  the `id` in its `game.json`); #53 `hot-reload` closed 2026-10-10 (branch kept; its native `re2` half could be re-cut); the live relay was redeployed with #85's
  code on 2026-10-10 (old binary `red_relay.prev-20261010` beside it).
- Earlier milestones (hardening #74-#84, CI wall-clock #97, content formats #100-#106, agent loop #102-#105, aimed fields #115, the games) are in git and
  `docs/analysis/`.

## Next
- Phase 2 after the items in Now: (2) a fall-out height, (3) rules that change movement, gravity and scale, and a body swap, (4) 3D string tables, (5) 3D
  spawn/destroy only if needed. Phase 2 is done when Gentle Carousel is well under 100 rules and a Prop Hunt PR touches no file under `src/` or `crates/`.
- Phase 3 (uniqueness as the cheap path: shape ops, palette roles, `catalog --template`, `own_prefab`, `style-own`) after Phase 2.
- Review follow-ups 4-7 of 2026-10-14 (headless `workshop_server` without red2d, feeding-order ADR, fresh-agent trial of the Rust path, the TUF/save-backup
  page): not started. Shooter move step 6c (the move itself): paused.
- Measured bottleneck: a game's snapshot part is whole-state (8 bytes per machine) and is left out of snapshots above about 170 machines: replication
  relevance or deltas. (The other one, string-keyed lookups and per-tick allocation, is what #170 addresses.)
- Killchain downstream: the Killchain project in RedEngineGames runs `killchain` once RedEngineGames #7 merges.
- Friction still reproducing on 2026-10-17: `scripts/dev start` routes a task that says "migration" to the upgrade workflow; `sim --only` matches a substring
  ("lever" picks "without the lever"). Not re-checked: `validate` with no paths outside a checkout, `validate defs/x.json` reading definitions as a scene,
  `sim --json --inspect` returning the whole 14 KB result.
- Simplification S7 (offline `re2` through MatchSim/LocalSession): parked; it needs a design and a parity plan.
- CI growth from generated games: proposals 2-4 of `docs/analysis/2026-10-09-engine-ci-growth.md` are not built.

## Failing / blocked
- Nothing failing on main: CI, Release (Linux binaries) and Publish games passed at c4860ce (2026-10-16).
- Prebuilts exist: the newest is `prebuilt-e4f91efe9590` (main f23b763, 2026-10-15; the commits since are games and docs, so no new one). On this checkout
  `scripts/dev start` fetched it, verified its checksum and needed no compile (4.5 s, 2026-10-17).
- `red_server --key auto` prints a 32-hex key (`net::auth::random_key`, `src/server_app.rs`), not the 12-character readable key `docs/HOSTING.md` promises
  (`net::relay::generate_join_key`), so its relay code is long; not fixed.
- The game servers are asleep at the owner's request since 2026-10-17: `red-relay`, `red-game@killchain`, the health and portmap timers are stopped and
  disabled. Do not restart them as a side effect of other work.

## Decisions & gotchas
- The front door: README is a pointer, `AGENTS.md` is the single entry point (ADR 2026-10-09-the-front-door-documents-are-a-pointer-one-entry-point, hardening item 1); the claims in the docs are checked by `red_engine2 preflight`.
- The loadout shooter (arsenal, ordnance, killcam, `sim::shooter`, the objective modes) moves to Killchain: no second game uses it (ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or, `docs/analysis/2026-10-14-system-audit.md`).
- Merging: `main` has no branch protection or required checks, so `gh pr merge --auto` merges at once; wait for hosted CI and merge by hand. The owner merges engine PRs (Idea Forge's own PRs merge themselves when CI passes).
- `cargo` is not on the default non-interactive PATH (`scripts/dev` adds it); a fresh worktree needs `scripts/dev worktree NAME` (seeded dependencies) or `CARGO_TARGET_DIR` pointing at a built tree, or its first build takes 7 minutes on four cores.
- The user's own `RedEngine` checkout has uncommitted work: agents work in worktrees and fresh clones.
- CI measurements: a probe PR targets a throwaway branch, never `main`, and the throwaway base branch is deleted right after; a pushed branch off `main` invites a "Compare & pull request" click (that is how #89 reached main). Close probes and delete their branches when done.
- `tests/net_sim.rs` `a_full_match_of_eight_holds_up_on_a_mobile_link` failed once on the Windows runner (#88, bandwidth 25351 B/s against a 24000 limit) and passed on re-run; not seen in the 60 runs before. Watch it; if it recurs the limit is too tight for Windows.
