# 2026-10-09. CI runs the Python tools on Windows when they change
Status: accepted
Summary: A path-selected python-tools job runs every scripts/test_*.py on Linux and Windows, with no Rust build, whenever a Python, PowerShell or script file changes

## Context
The hosted workflow decides what to run from the changed paths. The Windows leg of the `test` job starts only when `native` is true: `src/`, `crates/`, `tests/`, `benches/`, `examples/`,
`assets/`, `recipes/`, `Cargo.*`, `rustfmt.toml`, `scripts/ci.sh`, `.cargo/` or `.github/` changed. A change to `scripts/launchpad.py`, `red_resolve.py`, `idea_forge.py` or a PowerShell
script touches none of them, so it reached Windows never, or only by riding on an unrelated Rust change, although these are the files where Windows differs: path separators, quoting,
subprocess and process-tree behaviour, git's output. (The launchpad's `rc/lib.rs` corruption and the supervisor's Windows tree-kill both live there.) The Rust-side wrappers
(`tests/launchpad.rs`, `tests/idea_forge.rs`) do run the Python suites on Windows, but only after a full engine build, and only when something under `tests/` changed.
Separately, the changed-file list came from `git diff --name-only`, which quotes a name with a non-ASCII or special character, so every anchored pattern missed such a file; and
`echo "$files" | grep -q` under the runner's `pipefail` can report "no match" when grep exits before echo has finished a long list.

## Decision
- `scripts/ci.sh pytools` is the one cheap stage that runs every `scripts/test_*.py` plus `publish_games.py check` on the machine's own interpreter, with no Rust build (about a minute). It is part of
  a full local run, so what CI runs and what `scripts/ci.sh` runs still cannot drift.
- The `changes` job has a third output, `pytools`, true for any `*.py` or `*.ps1`, anything under `scripts/`, `tools/` or `bench/`, the publish manifest, the MCP adapter, the bench scripts and
  `.github/`; Rust-only and documentation-only changes leave it false. No usable base commit still runs everything.
- The `python-tools` job runs that stage on `ubuntu-24.04` and `windows-latest` (a 25-minute limit, because a hung process is exactly what these tests are about) when `pytools` is true. Linux is
  there for the fast signal (the `test` job's Linux leg reaches the same files only after the engine builds).
- The file list is read with `-z` and matched through a here-string, so quoted names and long lists are handled.
- `scripts/test_ci_selection.py` (wrapped by `tests/ci_selection.rs`, and run by `pytools` itself) extracts the workflow's own step and runs it in throwaway repositories, so the selection is
  tested as written rather than re-implemented.

## Consequences
- A Python-only pull request now gets a Windows verdict in minutes and does not pay for a Windows engine build; a Rust-only or documentation-only one pays nothing new.
- The Linux `test` job still runs the same Python suites through cargo (unchanged); a Python-only change therefore runs them twice on Linux, the second time in a minute.
- Only the interpreter on the runner is exercised; a Windows machine without `bash` cannot run the `ci.sh` stage, but the job uses Git Bash like the existing `test` job does.
