# STATUS — red_engine2

_Handoff file for whoever (human or AI) resumes this work. Keep it short and current: update it at every checkpoint with
`red_engine2 status --note "what changed" --section done|now|next|blocked|notes`. `scripts/dev preflight` warns when it is more than 20 commits behind main._

## Now (in flight)
- 2026-10-09: HARDENING PASS (the review of 2026-10-09: stale docs, a cold compile as the first step, scope drift). One pull request per item, merged on the owner's instruction after hosted CI passed:
  (1) entry docs true and consistent, (2) prebuilt binaries instead of a cold compile, (3) hermetic Idea Forge tests, (4) this file, (5) Killchain out of the engine crate, (6) browser leftovers, (7) bad input is an error, (8) CI growth and the rules-language gaps written down.
  All ten pull requests are merged (#74-#83); main was green on hosted CI after the last one. What each changed is in its PR and ADR.
- Open pull requests from earlier work, not yet reviewed: #37 `savedata` (a game's settings and progress follow its `game.json` id), #53 `hot-reload` (re2 applies a saved scene or game in place).
- Branch `multiplayer-reliability` (pushed, no PR, 5 commits): joining by the relay's six-character code carries the host's admission key (ADR 2026-10-09) and the relay's keepalive, goodbye and lease are fixed;
  the launchpad reads git as NUL-delimited data; Idea Forge agents run under a deadline; a `python-tools` CI job. The deployed `red_relay` is the old build until it is redeployed (`deploy/red-relay.service`);
  the Windows leg of the new CI job has never run.
- Multiplayer stabilization brief (`docs/feedback-2026-10-07-stabilization-brief.md`, filed against f58e15d): not implemented as a whole. Item 1 (short-code join with a key, relay lease) and the source-fingerprint half of item 2 are on `multiplayer-reliability`;
  git shows no commits for the rest of item 1 (map retry through the relay, search-and-destroy reconnect, duel capacity after replacement), item 2's executable-selection and discovery parts, item 3 (replay coverage of the objective modes) or item 5.
  Item 4 (native 2D delivery) is partly there (`re2d` and the native publish route, #55); the packaged-executable walk-through it asks for has not been done.
- Idea Forge (`docs/IDEA_FORGE.md`) runs nightly on the owner's machine, opens up to two game PRs a night and merges them when CI is green (#58-#73 so far); its backlog of engine findings is in `docs/analysis/idea-forge/`.

## Done
- 2026-10-09: Idea Forge runs Midnight Engine (#72) and Salted Choir (#73, feedback only: the game did not build).
- 2026-10-08: Idea Forge (#58-#71): the idea generator, the CLI pipeline (idea, AI builds, feedback), daily and nightly jobs, auto-merge, a settle log, test isolation; `start` now says up front that a missing executable means a cold build (#62).
- 2026-10-07: S4, browser removal, done: a native 2D player (`re2d`, `play2d`) and the browser target removed (#55), leftovers cleaned (#56, more in hardening item 6); CI on Node 24 actions (#57); the server loop blocks on the transport (1271001).
  Killchain game modes (free for all, capture the flag, search and destroy, duels; protocol v15) were pushed straight to main in f58e15d, after the simplification baseline.
- 2026-10-06: the AI launchpad `scripts/dev start|next|resume` (#51), a native MCP server (#52), core-first orientation (#49), per-player rule variables (#50), the development efficiency pass (#46), browser reliability (#47; the browser itself was removed on 10-07).
- 2026-10-06: simplification stages S1 (native 2D honest boundary, publish kind 2d), S2 (`mcp` feature), S3 (`video` feature).
- 2026-10-07: the RedEngineGames branch `native-2d-downloads` has landed there; RedEngineGames syncs from RedEngine main at every push.

## Next
- Simplification S7 (offline `re2` orchestration through MatchSim/LocalSession): not done in the hardening pass on purpose; the offline loop (`src/bin/re2/frame.rs`) also does bat swings, carried props, the rules HUD and its own prop physics that `LocalSession` lacks, so it needs a design and a parity plan first. S5 (#80) and S6 (#79) are done with parity runs recorded in their PRs and ADRs.
- Killchain downstream: `red_engine2 package` / `game play` know only `re2`; the Killchain project in RedEngineGames must build and run `killchain` (games/killchain) instead.
- The rules-language gaps from the Idea Forge notes (per-entity state and timers, a fall-out height, rules that change player movement, gravity or scale): ranked and designed in `docs/RULES_LANGUAGE_GAPS.md` (#83), not built.
- Bounding the engine's CI time: 58% of CI job-minutes since Oct 7 went to Idea Forge (about 59 job-minutes a game). A games-only path filter (#88: `games_only` in `ci.yml`'s `changes` job, `scripts/ci.sh games`) skips the Windows leg, the headless build and the image for a PR whose files are all under `examples/2d`, `examples/3d` or `docs/analysis/idea-forge`; one probe pair measured about 43 job-minutes (22 wall-clock) against about 2. Proposals 2-4 of `docs/analysis/2026-10-09-engine-ci-growth.md` (move generated games out, cap them, nightly measurement) are not built.

## Failing / blocked
- Main was green at 30bb4e5 (CI run of 2026-10-09 13:41). Not failing on main, but `scripts/test_idea_forge.py` failed on any machine with under 20 GB free (fixed in hardening item 3).
- Waiting on the owner: whether `release.yml` should publish a prebuilt on every `main` commit that changes a build input (`start`/`scripts/dev red` fetch a release only when its sources equal the checkout's, which for tag-cut releases is rare: there are no releases yet).

## Decisions & gotchas
- The front door: README is a pointer, `AGENTS.md` is the single entry point (ADR 2026-10-09-the-front-door-documents-are-a-pointer-one-entry-point, hardening item 1); the claims in the docs are checked by `red_engine2 preflight`.
- Killchain's reusable shooter (arsenal, ordnance, killcam, `sim::shooter`, the objective modes) is engine by design (`docs/features.json`, ADR 2026-09-30-killchain-loadout-shooter); the Killchain game itself (client, screens, branding) is what S6 moves out.
- `cargo` is not on the default non-interactive PATH (`scripts/dev` adds it); a fresh worktree needs `scripts/dev worktree NAME` (seeded dependencies) or `CARGO_TARGET_DIR` pointing at a built tree, or its first build takes 7 minutes on four cores.
- The user's own `RedEngine` checkout has uncommitted work: agents work in worktrees and fresh clones.
