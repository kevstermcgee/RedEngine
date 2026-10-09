# Engine CI growth from generated games: measured, with a proposal (2026-10-09)

Asked: `tests/games2d.rs` and `tests/games3d.rs` verify every generated game and Idea Forge adds up to two games a night; how much has CI grown, and how could it stay bounded? Measured, not implemented. Everything below is
from the repository's own history and hosted CI (`gh run view`, `gh api .../actions/jobs/<id>/logs`) plus one local pass; the method is at the end so it can be repeated.

## The finding in one paragraph

**The games themselves are not what grows CI; the number of CI runs they cause is.** Verifying every generated game costs 2.5 s of a 350 s Linux test stage on hosted CI (under 1%), and the Linux job's time has not gone up
since the games began (7.4-9.5 minutes now, against 8.8-16.3 minutes on October 5-7, before any generated game; the browser removal and the cache account for the drop). But each game is a pull request and a merge, and
each of those runs the whole CI, including the 18-minute Windows leg, for a change that only adds a JSON file and a note: **about 59 CI job-minutes a game** (its PR 33, its merge 26). Since October 7, the seven game and feedback
PRs and their five merges cost 359 of the 1,544 CI job-minutes (23%) in two and a half days, and the work on Idea Forge's own scripts (13 PRs, 11 merges) another 530 (34%): **58% of all CI went to Idea Forge**, games and tool together.

## What the generated games cost per run

| | now | note |
|---|---|---|
| generated games in the repository | 2D: 11 files in `examples/2d/` (4 are Idea Forge's: idea-forge, electric-tide, patient-loom, midnight-engine; 7 are hand-made), 3D: 2 (`drowsy-marathon`, `gentle-carousel`) | a third 3D game (`salted-choir`) was filed as feedback only: it did not build |
| `tests/games2d.rs` on hosted CI | 24 tests, 2.4 s | of a 320 s `tests` stage (350 s with lints) |
| `tests/games3d.rs` on hosted CI | 2 tests, 0.1 s | |
| `tests/idea_forge.rs` | 1 test (the Python suite), 8.6 s | the biggest single contribution of the Idea Forge feature, and it does not grow with games |
| `verify` of one game, local, CPU seconds | 2D: 0.03-2.2 (mean 0.68, 11 games = 7.4 s); 3D: 0.05 and 0.15 | debug test profile, one process; `tiny-station` (2.2 s) and `idea-forge` (1.7 s) are the slow ones |
| bytes per game in git | 8-102 KB (`gentle-carousel` 102,527: 252 rules; `idea-forge` 72,164; most 8-34 KB) plus a 6-12 KB analysis note | at 2 games a night, 0.1-0.3 MB a night |

Extrapolated linearly (0.7 CPU-s a 2D game, 0.1 a 3D game, one of each a night): +0.8 CPU-seconds of test time per night, about 5 minutes of test CPU a year added to a 6-minute stage. That is not a reason to act.

## What the games cost in runs

| trigger (since 2026-10-07, 54 CI runs) | runs | CI job-minutes | per run |
|---|---|---|---|
| Idea Forge game or feedback pull requests (`idea-2d-*`, `idea-3d-*`, `idea-electric-tide`) | 7 | 229 | 32.8 |
| their merges to `main` | 5 | 130 | 26.0 |
| Idea Forge tool pull requests (`idea-forge-*`, `engine-fix-*`) | 13 | 299 | 23.0 |
| their merges to `main` | 11 | 231 | 21.0 |
| everything else (other PRs, other merges) | 18 | 655 | 36.4 |

A job-minute is one runner busy for a minute (the sum of the jobs' durations, so parallel jobs add up). The cost of one run is dominated by compiling and by two legs the change does not need: the Windows build (14-22 minutes, 38% of all job-minutes in the period) and the headless server build (7-9). The `what changed` job already skips Docker for a docs-only PR, but its `native` filter lists `examples/` and `tests/`, so a PR that adds
`examples/2d/<game>.game2d.json` and a note runs everything. A game PR only needs: the game still verifies (Linux, seconds once the binary exists) and the Idea Forge scripts' own tests if `scripts/` changed.

Steady state at the planned rate (two games a day, about 59 job-minutes each): about 120 job-minutes a day, 3,500 a month, of which the games' own tests are 0.1%.

## Proposals (not implemented; in the order I would do them)

1. **A narrow path filter for generated-game PRs.** In `ci.yml`'s `changes` job, add a third output (`games_only`): true when every changed file is under `examples/2d/`, `examples/3d/` or `docs/analysis/idea-forge/`. Then the `test` job runs Linux only and, instead of the full
   `ci.sh` suite, runs `cargo test --test games2d --test games3d` plus `docs_fresh`; `headless-linux` and `docker` are skipped. Measured saving per game PR: the Windows leg (13 job-minutes on average for these PRs) and the headless job (8), leaving about 8-10: **from 33 to
   about 9 job-minutes a PR, and from 26 to about 9 for its merge**, which can use the same filter (a `main` push already caches). Risk: a game that depends on an engine change in the same PR is not a games-only PR, so the filter does not apply. Cost: about 15 lines of shell.
2. **Move generated games to RedEngineGames with their own CI** (the idea in the brief). The engine would keep a bounded set (hand-made examples and the games that prove a mechanic the engine documents, now 5-6 files), Idea Forge would open its game PR against `RedEngineGames/projects/idea-forge/`
   (RedEngine content already syncs there at every push), and that repository's CI would run `red_engine2 verify` for each game with a prebuilt engine fetched by `scripts/prebuilt.py`'s mechanism (no compile at all). Engine CI would then not grow with the games, at the price of a second repository to keep green and
   a rule for the games that are evidence for an Idea Forge feedback note (`docs/analysis/idea-forge/` keeps the note; the game moves). This removes the whole 56%.
3. **Cap and rotate** whichever games stay: keep the newest N (say 12) generated games under `examples/`, and let `idea_forge.py ship` move the oldest to RedEngineGames when it opens a PR for a new one. Bounds repository size and `games2d` at a number, whatever the rate.
4. **Measure it every night so growth cannot go unseen**: `benches/history/` already holds histories; one line per merged game (job-minutes of its PR run, from `gh run view`) appended by the nightly job, and `games2d`'s own time printed by its test, would make the next version of this note a table read, not a forensic exercise.

Recommendation: 1 now (small, local, reversible), 2 if the rate of games stays near two a day for a month. 3 only if 2 is refused.

## Method (to repeat)

- Runs and jobs: `gh run list --workflow CI --limit 150 --json ...` then `gh run view <id> --json jobs`; durations from `startedAt`/`completedAt` of successful or failed jobs. Triggers classified by branch name (`idea-*`) and the merge titles on `main`.
- Per-stage and per-binary times of one run: `gh api repos/<owner>/<repo>/actions/jobs/<job id>/logs` (the plain `gh run view --log` returned nothing here), the `-- stage times` table that `scripts/ci.sh` prints, and each `Running tests/<name>.rs` / `test result: ... finished in Ns` pair (ANSI colour codes stripped).
- Local: `red_engine2 verify <game>` per game, user plus system CPU seconds from `getrusage`, on a four-core machine with other work running, so CPU seconds rather than wall time.
- Limits: 54 runs in 2.5 days; hosted runner speed varies by 20-30% from run to run (the Linux job ranged 7.4-10.7 minutes on identical sources), so differences under a minute are noise. The Windows leg was not broken down by test.
