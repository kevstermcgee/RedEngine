# Build-time baseline on the dev/server box (2026-09-29)

Measured, not estimated: how long Red takes to build and test-compile on the Ubuntu machine that is both the dev/test box and the online game server (Intel N97, 4 low-power cores, 15 GB). The raw numbers, the commands and how to repeat them are in `benches/history/build-times.json` (schema `red-build-times/1`; append a new run, never edit an old one). Engine commit `3a516f0`, clean tree, cargo 1.98.1.

## The short version

* **The dev-profile edit loop is already fast:** a real one-line edit rebuilds all binaries in **4.5 s** (`src/sim/rules.rs`) to **7.6 s** (`src/schema.rs`); every test target in 14 s. Do not spend effort here first.
* **The `fast` profile edit loop was not (fixed, see Phase 1 below):** the same edit cost **57 s** (13x) because `[profile.fast]` had `incremental = false` and 4 codegen units. It is now 4-5 s.
* **Cold builds are dependency-bound, not engine-bound:** headless 199 s, with graphics 234 s (dev) / 270 s (fast). `red_engine2` itself is 44-50 s of that and cannot start until its dependencies finish (142-208 s in). Heaviest: the graphics stack (naga 62 s, wgpu-core 60 s, x11rb-protocol 51 s), then syn, rustls, parry3d, moxcms, image, serde_derive, clap_derive, rapier3d, quinn-proto. Cold builds happen on a fresh clone, a new target dir, a toolchain bump, and every `docker build` (the Dockerfile has no layer cache).
* **Compiling tests from scratch is expensive:** 135 s for the lib tests plus 88 s for the other 35 test binaries.
* **Release (LTO) is the slowest edit loop:** a one-line edit costs **219 s** for all binaries (49x the dev edit) and **87 s** for the headless server alone (the binary that is deployed). Cold: headless 154 s; all binaries 329 s on top of the headless build.
* **The server has ample headroom, and building on the same box eats some of it.** Idle, 4 walking players: server tick p99 **48 us** against a 16 667 us budget at 60 Hz. 8 players: p99 579 us. With a cold cargo build running: 4 players p99 76 us, 8 players p99 **3.2 ms** (worst tick 3.9 ms, 24% of budget). Still inside the budget, but a build and a full server compete for four cores. 8 players costs 12x the p99 of 4 (worse than linear; not investigated).
* **Not yet measured:** `scripts/ci.sh`, `affected`, Docker builds, test run time, real-network behaviour, and any end-to-end token cost. They are listed under `not_measured_yet` in the JSON.

## Caveats a later session must not trip over

1. **`touch` is not an edit.** Touching a file gave 3-4 s because rustc's incremental cache saw identical content. Use the probe edit in the JSON.
2. **`cc` used to shadow the C compiler (fixed).** The user's Claude tmux launcher was `~/.local/bin/cc`, ahead of `/usr/bin` on PATH, and broke every link (`open terminal failed: not a terminal`). The user renamed it to `ccl` on 2026-09-29. Runs up to the `fast` profile used `PATH=/usr/bin:$HOME/.cargo/bin:/usr/local/bin:/bin`; the release runs used the default PATH; same compiler either way. If that error returns, run `which cc` first, and never run the launcher from a non-interactive shell.
   `sccache` 0.7.7 (apt) was installed afterwards and is deliberately **not** enabled in any measurement here (`RUSTC_WRAPPER` unset): these are the no-cache numbers.
3. `cargo` is at `~/.cargo/bin` and not on the non-interactive PATH; `scripts/dev` and `scripts/ci.sh` add it themselves.
4. One restore-rebuild of the test targets took 41 s against 14 s for the edit before it; single runs are noisy on this CPU. Repeat before drawing fine conclusions from a difference under about 20%.
5. Load-sensitive tests (for example `net_sim`'s cruel-link case, noted in the physics-games analysis) and any timing on a busy box, such as while it serves a game, will differ from these numbers.

## Phase 1 results: what worked and what did not (2026-09-29, run `2026-09-29-phase1-build-speed-experiments`)

* **Worked: `[profile.fast]` edit loop 57 s -> 4-5 s.** Incremental compilation reuses whole codegen units, and the profile had `codegen-units = 4`, so an edit recompiled about a quarter of the engine crate. Now `incremental = true` and `[profile.fast.package.red_engine2] codegen-units = 256`; dependencies keep 4 (the `wide` crash workaround). Opt-level is unchanged and server tick times did not move; the Windows-recorded fixture trace replays with every checksum matched under the debug, release and fast binaries. Applied in `Cargo.toml`.
* **Worked: the server does not need LTO.** For up to 8 players the tick is about 4% of budget on every profile (p99 503-694 us of 16 667 us), because the hot path is in the physics dependencies, which are optimized even in the dev profile. `docs/HOSTING.md` now builds the Ubuntu server with `--profile fast` (headless edit rebuild 87 s -> 45 s, or 27 s with incremental). `red_engine2 package` still uses release: it must stay reproducible.
* **Did not work: the `mold` linker.** Same edit times with and without it. rustc already uses `lld` and linking is not the bottleneck.
* **Did not work (here): `sccache` 0.7.7.** Cold builds into different target dirs took 298 s, 300 s and 292 s: it did not carry over between them (89 hits vs 245 new misses on the second build). Not enabled; its cache was deleted. Path-remapping flags might fix it; only worth trying if cold builds start to matter.
* **Not a lever: engine-crate opt-level** (25-31 s at level 1-2 with 4 units, vs 4-5 s once units are split).
* **Trap:** `CARGO_PROFILE_<name>_PACKAGE_<crate>_OPT_LEVEL` is silently ignored by cargo. Use `--config profile.<name>.package.<crate>.<key>=<value>` and confirm with `cargo build -v`.
* **Per-game engine builds.** RedEngineGames pins the engine two ways. `reddm` uses a path, which shares this checkout's `target/`. `gravity-gauntlet` pins a git commit, which `scripts/red` clones into the project's `.red/engine` and builds cold (about 5 minutes; it is an older engine than `main`). Sharing one `CARGO_TARGET_DIR` across projects on *different* commits is unsafe as the wrapper stands: the final binaries overwrite each other and the staleness check would trust the wrong one. A safe version needs the wrapper to copy binaries into a per-project folder. Not built; path-pinning games to this checkout avoids the problem on this box.
* **The shared server and pinned commits.** Clients come from RedEngineGames CI at the cataloged RedEngine commit, so a game's server must be built from that commit (same wire protocol; `main` is at protocol v10). One `red_server` built from `main` will not serve a game pinned to an older, incompatible commit.

## Phase 2 results: the edit-and-verify loop (2026-09-30, run `2026-09-30-dev-workflow-iterate`; decision in ADR 2026-09-30-bounded-iteration-explicit-configuration-and-safe-green)

What the phase-1 numbers left unexplained was not compile time (the dev loop is 2-4 s to type-check) but what the loop *asked for*. Measured, steady state, wall seconds, one command at a time:

| scenario | before | after |
|---|---|---|
| `preflight` after a one-line edit | 5.9 (rebuilt the graphics CLI) | 1.3 (tree-only) |
| edit loop, localized edit (`rules.rs`) | `affected --quick --base HEAD` 23.7 | `iterate` 7.8 (8.9 CPU s), 1.6 with `--check-only` |
| edit loop, central edit (`schema.rs`) | 36.7 | `iterate` 9.2 |
| edit loop, docs-only edit | 20.8 | `iterate` 0.3 (nothing to compile or run) |
| `affected --quick` on this 17-file branch (default base) | 140 wall, 303 CPU s, 1.2 GB tree PSS | unchanged: use `iterate` for the edit loop |

* **`--release` in the loop was the largest avoidable cost** (release binary 3 min 18 s for a one-line edit, release suite about 15 min); the docs now say so and `CLAUDE.md` no longer contradicts itself.
* **A correctness bug, not just speed:** the planner used the compiled-in feature index and `scripts/dev` never rebuilt it after `docs/features.json` changed, so `context`/`affected` planned with a stale index. The index is now read from the checkout.
* **Cargo `-j2/-j3/-j4` is not a lever** for rebuilding the 42 test binaries after a central edit: means 48.5 / 43.5 / 49.5 s with 36-61 s spread inside each setting. The step writes about 1.3 GB (system-wide disk write counters) and stalls on I/O for 25-44 s of it, at 57-64 CPU-seconds and about 1.7 GB peak tree memory whatever `-j` is. The first attempt at this comparison was wrong (dirty pages from the previous build's writes leaked into the next run; a `sync` barrier fixed it), which is why the JSON says what is system-wide.
* **Headless type-check** saves 0.3-0.4 s warm (1.8-2.1 s vs 2.4 s): selectable with `iterate --headless`, not automatic.
* No stuck or abandoned processes were seen. Not measured: Windows, `iterate --headless` timing, test-thread concurrency, sccache (still off).

## Hypotheses still untested

* A workspace split (sim/net core, tools, gfx) would shrink the headless server build. With the edit loop at 4-5 s the payoff is now mostly cold and release builds; measure the release edit (87-219 s) before deciding.
* Trimming dependencies (`image`/`moxcms`, `clap_derive`, duplicated `syn`) would cut cold builds only.
* `Dockerfile`: no layer cache, so a cold release build on every image rebuild. Deploying the binary with the systemd unit on this box avoids Docker entirely.
* Fewer, larger integration-test binaries (35 link separately).

## Server headroom (measured with `red_engine2 perf examples/test_lab.json`, release build)

| condition | players | server tick p50 / p95 / p99 / max (us) | bytes/client/s |
|---|---|---|---|
| idle box | 4 | 22 / 44 / 48 / 86 | 7 182 |
| idle box | 8 | 51 / 478 / 579 / 666 | 13 750 |
| cold cargo build running | 4 | 32 / 75 / 76 / 137 | 7 182 |
| cold cargo build running | 8 | 96 / 846 / 3 172 / 3 929 | 13 750 |

The tick budget is 16 667 us (60 Hz). Best-of-3 windows filters noise, so the loaded rows understate the worst case. This is in-process loopback on a 25-prop map: no NIC, router, real QUIC link or prop-heavy map. One earlier attempt to measure under load silently ran on an idle box because the background build failed to start (`cargo` was not on that shell's PATH); always check the load average and `pgrep rustc` before trusting a "loaded" number. Mitigations to test if the server is hosted from this box while it builds: `nice`/`ionice` on cargo or `-j2`, or `CPUWeight`/`AllowedCPUs` in `deploy/red-server.service`.

## How to keep this useful

Re-run the scenarios in `build-times.json` after any change meant to speed builds, append a run with the new `id`, and add a line here saying what changed and what it bought. A change that does not move a measured number here should not be described as a speed-up.
