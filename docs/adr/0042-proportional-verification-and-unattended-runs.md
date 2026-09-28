# 0042. Proportional verification, small context, and runs nobody has to babysit
Status: accepted
Summary: Verification proportional to the change (`affected`), 5-15 KB `context` packets, loopback-only unattended runs

## Context
Projects built with Red Engine were slow and token-hungry for three separate reasons, all of them *fixed costs paid by every change*:

1. **Context.** `CLAUDE.md` imported the whole 43 KB `AGENTS.md`, so every session started with about 12k tokens of tool reference. Orienting on a Rust change meant
   `src map` (17 KB), `describe commands` (11 KB) and file reads.
2. **Validation.** The only choices were `scripts/dev fast` (lib unit tests) and `scripts/dev test` / `scripts/ci.sh` (everything: 27 test binaries, twice, once more headless, all
   forced to one test at a time because a few suites use real-time UDP). ADR 0033 already knew which tests a change affects but nothing ran them.
3. **Interruptions.** `red_server` listened on `0.0.0.0` by default and `NetClient` bound `0.0.0.0:0` even to a loopback server. On Windows that raises an "allow this app through
   the firewall?" dialog for every new executable path (every rebuilt test binary, every game project), which blocks an unattended run until someone clicks it.

## Decision
**Change -> affected subsystems -> minimal high-confidence verification -> full CI only at integration boundaries.**

- `red_engine2 affected` (`tools::affected`) turns `impact` into a plan and runs it: `cargo fmt --check`, clippy on only the changed targets, unit tests filtered to the changed
  modules (derived from the path *and* the owning feature), and the integration suites of the owning features, plus their dependents unless `--quick`. Suites listed under
  `serial_suites` in `docs/features.json` run one test at a time; everything else runs on all cores. Boundary changes (`Cargo.toml`/`Cargo.lock`, `src/lib.rs`, `rustfmt.toml`, `.cargo/`,
  CI files), a diff over 60 files, or an affected set that is at least 75% of the suites escalate to `scripts/ci.sh`: a subset cannot vouch for those. A changed test file runs itself.
  Nothing is silently skipped: `--quick` lists the dependents' suites it did not run. Correctness is not weakened, it is *scheduled*: the edit loop is `--quick`, "done" is the default
  scope, and the push boundary is `--full` (exactly what CI runs). A pass is stamped by a hash of the changed files' contents, scope and feature index, so re-asking is free.
- `red_engine2 context` builds a 5-15 KB work packet (feature summary and neighbours, owned files with purpose, public API with doc lines, tests, verification command, ADR *pointers*)
  from the same index and scanner; `src map <prefix>` is the cheap form of the module map. `CLAUDE.md` no longer imports `AGENTS.md`; `AGENTS.md` is a front door (about 11 KB) and the
  long reference is `docs/AGENT_REFERENCE.md`, indexed by `search` so its sections arrive as fragments.
- `scripts/ci.sh` is a set of stages (`fmt clippy tests benches headless-*`) that the GitHub workflow calls, so local and CI cannot drift. CI runs on pull requests and pushes to `main`
  only (no double runs), cancels superseded pull-request runs, and skips the Windows leg and the Docker image when nothing they build changed.
- `game check` remembers passing maps (`out/cache/`), keyed by the map's bytes, all other project JSON and the engine binary; failures are never cached and `RED_NO_CACHE=1` bypasses it.
- The dev profile keeps `line-tables-only` debug info for the crate and none for dependencies: about half the disk and a faster build and link, panics still carry file:line.
- **Loopback by default.** `red_server` listens on `127.0.0.1` unless `--bind`, `--public`, `RED_BIND` or `--upnp` says otherwise; a client of a loopback server binds loopback;
  `doctor` probes the port on loopback. The container image and the systemd unit set `RED_BIND=0.0.0.0` explicitly. No automated path (tests, bots, `net-test`, `perf`, `play-local`,
  `scripts/red serve`) needs a firewall exception, so none can wait for one. Hosting for other machines is a deliberate flag.

## Consequences
- A typical change verifies in seconds to a couple of minutes instead of the whole suite; the full run still happens before anything reaches `main`.
- `docs/features.json` is now load-bearing for speed as well as for `impact`: a wrong or missing test entry under-verifies a change in the default scope. `features --check` and a test keep
  it true, an unlisted source file adds the `features_index` suite to the plan, and the path-derived unit filter is a floor that does not depend on the index being complete.
- Test-module clippy lints are only checked at `--full`; the quick scopes lint the non-test code of the changed targets.
- Anyone who relied on `red_server` being reachable from other machines by default must now say `--public`; running a server for friends on Windows will (correctly) prompt once.
- Undo: remove the stages' callers from the workflow, or run `scripts/ci.sh` and `scripts/dev test` as before; nothing else depends on the new commands.
