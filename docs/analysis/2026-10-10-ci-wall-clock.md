# CI wall-clock per pull request: measured before and after (2026-10-10)

Goal: the time a pull request waits for CI (its slowest job), not job-minutes (standard runners are free on this public repository).
Each change was measured on hosted runners on PR #97 before the next went in; local numbers are from the 4-core development box.

## The result

| | slowest job | wall-clock |
|---|---|---|
| before: 43 engine PRs with the full matrix (median per job) | `full build (windows-latest)` | **19.0 min** (Linux 9.3, headless 8.1, container 3.8) |
| before: run 38076883035 (same sources as main + A1, which is not on the critical path) | `full build (windows-latest)` | **1386 s (23.1 min)** |
| after: runs 38086958666 attempts 1 and 2 | `tests (linux, shard 3/5)` | **275 s / 271 s (4.6 min)** |

Every job of the final layout, two passes (warm caches): Linux test shards 1-5: 255/249/275/212/262 s and 246/208/271/268/263 s; headless shards 1-3:
224/241/258 s and 234/237/240 s; headless build + clippy 68/74 s; clippy + benches 54/59 s; games on the public API 157/174 s; Windows build + Direct3D
219/218 s; container image 247/241 s; Python tools 63/61 s (Linux), 163/131 s (Windows). All green, all four passes of the final test layout.

## Step by step

| step | change | what it bought (hosted, warm unless noted) |
|---|---|---|
| A1 | headless server built in the dev profile, not release + LTO | `headless-build` 99 -> 58 s; the headless job 498 -> 451 s |
| A2 | Windows on a PR = build + Direct3D; full Windows suite on main | Windows job 1386 -> **219 s** (711 s on the first, cold run of the renamed job); PR wall-clock 23.1 -> 10.9 min |
| A3 | one `cargo nextest run`; the serial suites a test group that runs alone | Linux tests stage 434 -> 310 s, job 651 -> 477 s; headless tests 350 -> 287 s |
| A4 | tests in shards (Linux 5, headless 3), clippy/benches, the API games and the headless build their own jobs | slowest job 477 -> **275 s** |
| A5 | container image caches its dependency layer (cargo-chef + BuildKit GHA cache) | **no gain: reverted** (below) |
| A6 | fewer test binaries | **not done** (below) |

**A1.** `headless-build` compiled the engine with release + LTO only to prove the server links without `gfx`; the dev artifacts answer that, and the release
build is still made by `release.yml` and the container image.

**A2.** The Windows leg was 348 s of test compile, 278 s of parallel tests (two software-rendering suites: `split_render` 143 s, `procgen_render` 46 s), 180 s
of serial network suites and 141 s of the external client (run 37962855231). On a PR it now runs `scripts/ci.sh direct3d`: `shader_validation` plus
`shadow_render`, which creates a real Direct3D device on WARP and compiles the renderer's pipelines through Microsoft's compiler, and builds and links every
binary on the way (engine compile 2 m 38 s warm). Finding: `shader_validation` alone proves nothing on Windows that Linux does not (naga is the same
everywhere); the Direct3D authority comes from a test that creates a device.

**A3.** Local, warm build, 4 cores: the old three `cargo test` runs took 426 and 479 s (about 370 s of it running tests). Letting the serial suites run
alongside the rest took 220-222 s, but **two `net_e2e` tests failed under that load**: the 15% loss / 40 ms latency test on hosted CI (the watcher's last
view of a still-rolling barrel was 0.1 m from the server's) and the reconnect-by-timeout test locally, even with two of the four slots reserved. The serial
suites now take every slot (`threads-required = 'num-test-threads'`): 263-268 s locally, the same isolation they always had. The gain left is nextest
running tests of different binaries at once (`cargo test` ran one binary at a time) and a process per test. The loss/latency test now listens until the
barrel has stopped before comparing.

**A4.** Each shard compiles every test binary itself (about 150-180 s from cached dependencies), so sharding divides only the run. Shard count and
partitioning were chosen from a simulation over per-test JUnit times of a local run (1569 tests, 476 s of test time, 177 s of it serial): the slowest
shard's run under `hash:3` about 102 s, `hash:4` about 105 s (shard 3 drew 78.5 s of exclusive serial suites: the slow shard of both 4-shard runs,
317/325 s), `hash:5` about 57 s; `count:` partitioning was worse at every size. With 5 Linux shards and 3 headless shards the slowest job is 271-275 s,
and the floor is now each shard's compile.

**A5, container cache: negative, reverted.** With a source change the cargo-chef dependency layer was reused (a 16 s download), but the workspace's own
release + LTO compile is 128 s, and with buildx setup, cache import and export the job took about 213 s against a 228 s median before: inside runner noise.
It only wins when nothing changed at all (19 s), and the container job is not on the critical path. Not merged.

**A6, fewer test binaries: not done.** After touching `src/lib.rs`, a warm `cargo test --no-run` took 105 s locally (`--timings`): the library 35 s on the
critical path (15.8 s normal + 19.2 s test profile), more than 70 test crates at about 1 s each, and four at 22-24 s (`interactions`, `gate_meadow`,
`net_join_flow`, `sim_replay`, 200-470 lines, no unusual macros) that start together at 75 s and are the last 24 s of the build. Grouping 77 binaries into
ten saves at most the per-binary link, about 70 CPU-seconds, about 17 s on 4 cores, and breaks `scripts/dev affected`'s mapping of suites to
`tests/<name>.rs` and the `serial_suites` binary names. ADR 2026-10-03 measured the same for rebuilds.

## Not verified here

- The Windows **full** suite (pushes to main) now runs under nextest too; a pull request cannot exercise that path, so the first push to main is its test.
- The first pull request after merge runs the renamed and new jobs with caches saved by main's first run; until then they run cold (the measurements
  above used caches saved on this PR, see Method).

## Next

The floor is the per-shard compile of the test binaries (150-180 s of a 270 s job). Building them once and handing them to the shards
(`cargo nextest archive`) would remove it if the archive transfers faster than it compiles; measure the archive's size first (debug info makes test
binaries large). The four 23 s test crates are the other lead.

## Method (to repeat)

- Per-job wall-clock: `gh run list --workflow CI` + `gh api repos/<owner>/<repo>/actions/runs/<id>/jobs` (`attempts/<n>/jobs` for an earlier attempt):
  `completed_at - started_at` per job; per step the same; per stage the `-- stage times` table each `scripts/ci.sh` call prints; compile vs run inside a
  stage from the log timestamps of `Finished \`test\` profile` and the group markers.
- Cold vs warm: `Swatinem/rust-cache` keys on the job (or `shared-key`) and saves on main only, so a renamed or new job runs cold on its PR. To measure the
  steady state before merging, commits marked "TEMPORARY, measurement only" let those jobs save on this PR, and the run was re-run (`gh run rerun`); the
  final commit puts every `save-if` back to main only.
- Local: alternating runs (old, nextest, nextest, old) of `scripts/ci.sh tests` on a warm build, `RED_CI_NO_NEXTEST=1` for the old path, load average
  checked first. Shard balance: a nextest JUnit report (`[profile.default.junit]`) for per-test times, `cargo nextest list --partition <p> --message-format
  json` for each shard's tests, serial time summed plus parallel time over four cores.
- Limits: hosted runner speed varies 20-30% between runs on identical sources; most steps have one or two hosted samples.
