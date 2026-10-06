# Development efficiency pass (2026-10-06)

Goal: reduce **time-to-correct-change** (request → context → implementation → focused verification → done) without weakening verification, architecture or what games can be built.
Method: measure first, remove what the measurements blame, keep a guard test for every trade-off, and say what was not done.

**Machine and honesty.** Intel N97, 4 cores, 15 GB, shared with another agent for the whole pass (load average 4 to 12). Wall time on that machine mostly measures the neighbour, so every
step records **CPU seconds** and peak memory beside wall time. Fresh-agent numbers are one run per side with a small model: indicative, not statistically significant. Tools:
`benches/flow_bench.py` (flows, now with CPU, memory and step kinds), `benches/discovery_bench.py`, histories in `benches/history/`.

## Where the time actually goes (measured before changing anything)

| Representative task | Cost | Verdict |
|---|---|---|
| Change a 2D gameplay rule or an asset, prove it (`verify`) on a built CLI | **0.13 s, 2,525 tokens**, 5 commands | not a bottleneck |
| Find what to do: `describe --brief` 1.9 KB, `describe 2d` 6.7 KB, `context <file>` 11.9 KB (~3k tokens), `affected --dry-run` 1.2-2 KB | 0.15 s | cheap, but see discovery quality below |
| Engine edit, inner loop (`scripts/dev iterate`), warm | UI-kit edit **17.3 CPU-s** (34 s wall), networking edit **17.5 CPU-s** (18 s wall), 1.3 GB | fine; the header claimed "2-4 s" (corrected) |
| The same, first time in a tree | **295 CPU-s** (171 s wall) | test binaries; can't be shared safely |
| New worktree: first CLI build, no help | **257 units, 1,592 CPU-s, 1,170 s wall**, 1.7 GB | **the biggest avoidable cost** |
| New 2D game → verify → real-browser verify → publish (local) | 52 s wall, 15 CPU-s; the browser smoke runs **twice** (`web verify`, then inside `publish`, ~16 s each) | candidate, not done |
| Hosted CI, Linux job 856 s | tests 362 s, **external-client 339 s**, web3d 84 s, clippy 19 s, benches 12 s | external-client is 40% |
| Hosted CI, Windows 1,525 s | tests 815 s, **external-client 583 s** | same |
| PR #44 (this session): CI rounds to green | **4 rounds, about 70 minutes** | failures were all detectable locally: planner unit tests, `ai_tasks`, a stale example lock |
| Discovery: 16 realistic plain-language requests, is the right place in the top 3? | **recall@3 50% (8/16)**; every miss was a gameplay mechanic (door+key, timer, checkpoint, spawner, health) | the real exploration cost |
| Duplicated rules across 12 games (examples + patterns) | **25% of rule instances (27 of 108) are verbatim repeats of 8 rules**; 9 of 106 UI widgets | the next engine feature |
| `target/` of the main checkout | 59 GB → 70 GB during one session; 312 incremental dirs (9.2 GB) idle 3+ days | disk-full failures are confusing |

## What changed

1. **Warm-start new checkouts** (`scripts/dev worktree NAME`, `seed`, `scripts/seed_target.py`). Copies only *third-party* artifacts from a donor checkout.
   First CLI build in the new tree: 8 units, **378 CPU-s, 219 s wall** (+123 s of I/O-bound copying, 2.8 CPU-s) against 257 units, 1,592 CPU-s, 1,170 s: **CPU −76%, wall −71% including the copy**.
   *Two things learned the hard way, both recorded in the script:* (a) a build directory shared between two checkouts is **unsafe** (measured: two worktrees of one crate printed `Finished in 0.18s` and ran
   the other's binary), so workspace crates are never copied; (b) my first version skipped crates *named* `lib…` (`libc`, `libm`) by stripping a `lib` prefix, which cascaded into recompiling 82 units: cargo's own
   `stale: missing …/dep-lib-libloading` message found it.
2. **Verified 2D mechanics behind the existing front door** (`recipe`, `search`, `describe 2d`, new-game hints). Seven tiny complete games, each with the scenarios that prove it: key-door, timer-lose,
   collect-then-exit, health-damage, checkpoint-respawn, spawner-waves, survive-then-escape (the "collect artifacts, power a generator, survive, escape" concept as data only: no custom code, 43 s of deterministic play).
   Discovery recall@3 **50% → 100%** (16/16), answers +3% in bytes; all seven mechanic requests now return the right mechanic *first*.
   *The trade-off that nearly shipped:* the dense `health-damage` entry outranked the multiplayer docs for a 3D query and broke an existing test. Mechanics are now a **separate lane** (up to two, only when at
   least two *distinctive* query words match: words found in at most three of the seven), excluded from the corpus statistics, and a guard test asserts the ordinary results are **identical** with and without them.
3. **The plan says what it does not need and what to run before pushing** (`affected`: `not needed yet: network (…), browser (…), packaging (…)`, `before pushing: …`; JSON `needs`, `before_merge`). Derived from
   the plan itself, so it cannot disagree with it.
4. **Ownership fixes that make focused verification honest.** `ai_tasks` asserts on the planner's output but was owned by three unrelated features, so editing the planner never scheduled it (the cause of
   CI rounds 2-3 on #44); it is now owned by the feature that owns those tools, with a test. The mechanics, the new scripts and four release scripts had no owner, so editing them scheduled nothing: owned now.
5. **Fail fast, cheapest first.** A stale example `Cargo.lock` was found by the *last* CI stage (~14 min in); it is now the second stage (`ci.sh lockfiles`, **0.8 s**, fails with the exact fix in 2.5 s; proved by breaking
   a lock on purpose) and the local default order is fmt → lockfiles → headless-tree → clippy → the long stages.
6. **CI caches the example crate's dependencies** (`rust-cache workspaces`). **Unproven until a `main` run saves the cache**; a miss costs what it costs today.
7. `scripts/prune_target.py` (`scripts/dev prune`): reports idle incremental dirs, deletes only with `--apply`. Not applied to anyone's tree.
8. `flow_bench.py`: CPU seconds, peak memory, step kinds (discovery/edit/build/verify/ship), real-edit steps that are always undone; four new flows (gameplay rule, new 2D game to publish, engine edit loop, networking edit loop).

## Fresh-agent benchmark (small model, one run per side, identical task)

Task: a 2D "lighthouse" game composing four mechanics (3 oil cans open a door, spikes cost lives with a cooldown, a checkpoint with pit respawn, a 60 s countdown), HUD, and three scenarios (door blocks, bot wins, standing still loses).

| | before (no catalogue) | after | change |
|---|---|---|---|
| wall time | 313 s | 147 s | **−53%** |
| tokens | 78,157 | 57,717 | **−26%** |
| tool calls | 39 | 32 | −18% |
| game file | 21.4 KB | 4.1 KB | −81% |
| verify | passes (20 checks, it added sounds/music) | passes (10 checks) | |
| requirements | all, plus music/sfx/restart (not asked) | **all but one sub-requirement** | **not equal** |

The "after" game does not send the player back to the start when a pit is hit *before* the flag, and merges spikes and pits into one hazard; its report said "nothing fails". Grading caught it. The catalogue's checkpoint
pattern had not proven that case itself; it does now (a scenario added after grading, **not re-measured**). Speed rose; correctness in this run did not keep up, and one run cannot say whether that is the catalogue or the model.

## Capability and verification regressions checked

Additive only: no verification was removed or weakened, no format changed. Guard tests: ordinary search results identical with and without the mechanics; every pattern file listed and verifying; planner ownership;
lock check; `docs_fresh`, `cli_envelope`, `game_upgrade`, `headless_boundary`, `repo_hygiene`, `web2d_games`, `features_index`, `ai_tasks` all green after the last code change.

## Not done, and why

* **Crate splits / fewer rebuilds (phase 3):** the data does not ask for one: a warm inner loop is 17 CPU-s for UI and networking edits alike. No split was made.
* **New `locate`/`explain` commands (phase 9):** extended `search`, `recipe` and `affected` instead of adding commands (the overview has a byte budget).
* **Cache the duplicate browser run in `publish` (phase 12):** ~16 s per ship; left because the other agent is changing the publish pipeline.
* **Parallelising independent verification (phase 13):** not attempted on a shared 4-core box; the heavy-job lock already exists.
* **Reusable modules for the repeated 25% of rules (phases 6-7)** and **declarative scenario helpers (phase 8):** the catalogue covers copy-and-adapt; making restart/countdown/lives/banners/best-score *modules* in the format is the next change.
* Full-suite local CI was not run for this branch (the planner correctly says CI changes need it); hosted CI is the gate.

## Remaining bottlenecks and the next highest-leverage change

1. **First `iterate` in a fresh tree: 295 CPU-s** (test binaries), unavoidable without sharing workspace artifacts, which is unsafe.
2. **external-client: 339 s hosted** (40% of the Linux job): confirm whether the cache change works; otherwise run it only when the engine's public client API or dependencies changed.
3. **Full local CI is ~34 minutes** on a shared box, so agents skip it; the cheaper-first order and `lockfiles` reduce that cost, they don't remove it.
4. **Next change:** 2D `modules` (e.g. `"modules": ["countdown:60", "lives:3", "result-banners", "restart-on-action", "best-score"]`) expanding at parse time, removing about a quarter of the rules an agent writes today, with the existing scenarios as the proof.
