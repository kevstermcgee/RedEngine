#!/usr/bin/env bash
# The exact steps CI runs (.github/workflows/ci.yml). Run before pushing; green here = green there
# (on this platform). Usage: scripts/ci.sh [stage ...]      (no stage = all of them, in this order)
#
#   fmt  clippy  tests  benches  headless-tree  headless-build  headless-clippy  headless-tests  external-client  video
#
# Tests run in two groups, because only one kind needs to be slow:
#   * suites listed under "serial_suites" in docs/features.json (real-time UDP, spawned servers): one test at a time, as before;
#   * everything else (unit tests, doctests, every other integration suite): on all cores.
# RED_CI_SERIAL_ALL=1 restores "everything one test at a time".
set -euo pipefail
cd "$(dirname "$0")/.."
# Find cargo the way scripts/dev does, so this works from a shell that was started without it on PATH.
for f in "$HOME/.local/toolchain/env.sh" "$HOME/.cargo/env"; do [ -f "$f" ] && . "$f"; done
case ":$PATH:" in *":$HOME/.cargo/bin:"*) ;; *) PATH="$HOME/.cargo/bin:$PATH" ;; esac
export PATH CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-never}"

# The serial suites, read from the index without needing a built binary.
serial_suites() {
  tr -d '\n' < docs/features.json | grep -o '"serial_suites": *\[[^]]*\]' | grep -oE '"[a-z0-9_]+"' | tr -d '"' | grep -vx serial_suites || true
}

# run_tests [cargo flags...]: the whole suite for one feature set.
run_tests() {
  local serial parallel=() serial_args=() t name
  serial="$(serial_suites)"
  for t in tests/*.rs; do
    name="$(basename "$t" .rs)"
    if [ "${RED_CI_SERIAL_ALL:-0}" != "1" ] && ! grep -qx "$name" <<<"$serial"; then parallel+=(--test "$name"); else serial_args+=(--test "$name"); fi
  done
  echo "-- parallel group: lib, bins, ${#parallel[@]} suite flag(s)"
  cargo test --locked --no-fail-fast "$@" --lib --bins ${parallel[@]+"${parallel[@]}"}
  echo "-- doctests"
  cargo test --locked --no-fail-fast "$@" --doc
  if [ ${#serial_args[@]} -gt 0 ]; then
    echo "-- serial group (one test at a time, real-time networking)"
    RUST_TEST_THREADS=1 cargo test --locked --no-fail-fast "$@" "${serial_args[@]}"
  fi
}

stage_fmt() { echo "== rustfmt (rustfmt.toml is the style) =="; cargo fmt --check; }
stage_clippy() { echo "== clippy (warnings are errors) =="; cargo clippy --locked --all-targets -- -D warnings; }
stage_tests() { echo "== tests (lib, integration, doctests) =="; run_tests; }
# "The benches compile" is a type-check question: `check` answers it in seconds, where `bench --no-run` builds the whole engine optimised (measured: 475 s of
# a 1167 s local run, all of it for binaries nobody runs). RED_CI_BUILD_BENCHES=1 restores the full build.
stage_benches() {
  echo "== benches compile =="
  if [ "${RED_CI_BUILD_BENCHES:-0}" = "1" ]; then cargo bench --locked --no-run; else cargo check --locked --benches; fi
}
stage_headless_tree() {
  echo "== headless server: no graphics, audio or MCP crates in the dependency tree =="
  # The tree is captured first, then searched: piped straight into grep, a *failing* `cargo tree` (a stale Cargo.lock under
  # --locked, a broken registry) matched nothing and the stage passed with nothing checked.
  local tree
  if ! tree="$(cargo tree --locked --no-default-features -e normal --prefix none)"; then
    echo "cargo tree failed: the headless dependency tree could not be checked (a stale Cargo.lock? run cargo update -p <crate> or regenerate it)"; exit 1
  fi
  if [ -z "$tree" ]; then echo "cargo tree printed nothing: the dependency check ran on an empty tree"; exit 1; fi
  if grep -E '^(wgpu|winit|softbuffer|rodio|cpal|alsa|pollster|ffmpeg-sidecar|naga|ash|rmcp|schemars) ' <<<"$tree"; then
    echo "a graphics, audio or MCP-adapter crate leaked into the headless build"; exit 1
  fi
  echo "$(grep -c . <<<"$tree") crates checked, no graphics, audio or MCP crate among them"
}
stage_headless_build() {
  echo "== headless server builds without the gfx feature =="
  # The shipped build is the LTO release one, and the hosted job (CI=true) builds exactly that. A local run only needs to know the binaries build and
  # link without gfx, which the dev profile answers from the artifacts `headless-tests` builds anyway; RED_CI_RELEASE=1 asks for the release build.
  if [ "${CI:-}" = "true" ] || [ "${RED_CI_RELEASE:-0}" = "1" ]; then
    cargo build --locked --release --no-default-features --bin red_server --bin red_bot
  else
    cargo build --locked --no-default-features --bin red_server --bin red_bot
  fi
}
stage_headless_clippy() { echo "== headless clippy =="; cargo clippy --locked --no-default-features --all-targets -- -D warnings; }
stage_headless_tests() { echo "== headless tests (incl. real-UDP server tests) =="; run_tests --no-default-features; }
# A game outside the engine crate that uses only the public client layer (`red_engine2::app`, ADR 0043): if the API breaks it, CI says so.
# Its presentation test renders offscreen; a runner with no GPU adapter at all sets RED_OFFSCREEN_OPTIONAL=1 to skip just that check.
# Every example crate that is its own workspace has its own Cargo.lock; it must follow the engine's dependencies or `--locked` refuses it with a message that does not say so.
# Seconds, and the commonest reason a dependency change fails CI: so it is its own stage and runs FIRST (measured on PR #44: found by the last stage of a 14-minute job).
check_example_locks() {
  local manifest
  for manifest in examples/external/*/Cargo.toml; do
    # Only crates that use the engine and keep a lock of their own (a standalone experiment that does not use the engine has neither to fall behind).
    [ -f "$manifest" ] && [ -f "$(dirname "$manifest")/Cargo.lock" ] && grep -q '^red_engine2' "$manifest" || continue
    if ! cargo metadata --locked --manifest-path "$manifest" --format-version 1 >/dev/null 2>&1; then
      echo "$(dirname "$manifest")/Cargo.lock is behind the engine's dependencies: run"
      echo "  cargo metadata --manifest-path $manifest --format-version 1 >/dev/null"
      echo "and commit the updated lock."; exit 1
    fi
  done
}
stage_lockfiles() { echo "== example crates' lock files follow the engine's dependencies =="; check_example_locks; echo "ok"; }
stage_external_client() {
  echo "== external custom client (examples/external/topdown_switch) =="
  check_example_locks
  cargo clippy --locked --manifest-path examples/external/topdown_switch/Cargo.toml --all-targets -- -D warnings
  cargo test --locked --manifest-path examples/external/topdown_switch/Cargo.toml
}

# Killchain is a game on the engine's public API (games/killchain, ADR 2026-10-09-killchain-is-a-game-crate-on-the-public-engine-api): a workspace member that is not a default member, so
# the engine's own stages never compile it and the headless build never sees its graphics crates. If an engine API change breaks the game, this stage says so.
stage_killchain() {
  echo "== Killchain (games/killchain) builds, lints and tests against the engine's public API =="
  cargo clippy --locked -p killchain --all-targets -- -D warnings
  cargo test --locked -p killchain
}

# The optional export capability stays buildable and lint clean: MP4 export (`video`, ffmpeg-sidecar) is not in the default build (ADR 2026-10-07-tooling-dependencies-are-optional-features).
stage_video() { echo "== optional MP4 export builds (feature video) =="; cargo clippy --locked --bins --lib --features video -- -D warnings; }

stages=("$@")
# Cheapest, most-likely-to-fail first: formatting, the lock files and the headless dependency tree take seconds; the long stages come after them.
[ ${#stages[@]} -gt 0 ] || stages=(fmt lockfiles headless-tree clippy tests benches headless-build headless-clippy headless-tests external-client killchain video)
# Every stage is timed, and the table at the end says where the minutes went (the first thing to read when CI feels slow).
timings=()
t_all=$SECONDS
for s in "${stages[@]}"; do
  fn="stage_${s//-/_}"
  if ! declare -F "$fn" >/dev/null; then echo "ci.sh: unknown stage '$s'" >&2; exit 2; fi
  t0=$SECONDS
  "$fn"
  timings+=("$(printf '%5ds  %s' $((SECONDS - t0)) "$s")")
done
echo "-- stage times"
printf '   %s\n' "${timings[@]}"
echo "CI OK ($((SECONDS - t_all)) s)"
