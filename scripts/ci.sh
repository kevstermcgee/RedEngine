#!/usr/bin/env bash
# The exact steps CI runs (.github/workflows/ci.yml). Run before pushing; green here = green there
# (on this platform). Usage: scripts/ci.sh [stage ...]      (no stage = all of them, in this order)
#
#   fmt  clippy  tests  benches  headless-tree  headless-build  headless-clippy  headless-tests  external-client  web
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
  echo "== headless server: no graphics/audio crates in the dependency tree =="
  # The tree is captured first, then searched: piped straight into grep, a *failing* `cargo tree` (a stale Cargo.lock under
  # --locked, a broken registry) matched nothing and the stage passed with nothing checked.
  local tree
  if ! tree="$(cargo tree --locked --no-default-features -e normal --prefix none)"; then
    echo "cargo tree failed: the headless dependency tree could not be checked (a stale Cargo.lock? run cargo update -p <crate> or regenerate it)"; exit 1
  fi
  if [ -z "$tree" ]; then echo "cargo tree printed nothing: the dependency check ran on an empty tree"; exit 1; fi
  if grep -E '^(wgpu|winit|rodio|cpal|alsa|pollster|ffmpeg-sidecar|naga|ash) ' <<<"$tree"; then
    echo "a graphics/audio crate leaked into the headless build"; exit 1
  fi
  echo "$(grep -c . <<<"$tree") crates checked, no graphics/audio crate among them"
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
stage_external_client() {
  echo "== external custom client (examples/external/topdown_switch) =="
  # The example has its own Cargo.lock (it is a separate crate that uses the engine by path): whenever the engine's dependencies
  # change it must be refreshed and committed, or --locked refuses it below with a message that does not say so.
  if ! cargo metadata --locked --manifest-path examples/external/topdown_switch/Cargo.toml --format-version 1 >/dev/null 2>&1; then
    echo "examples/external/topdown_switch/Cargo.lock is behind the engine's dependencies: run"
    echo "  cargo metadata --manifest-path examples/external/topdown_switch/Cargo.toml --format-version 1 >/dev/null"
    echo "and commit the updated lock."; exit 1
  fi
  cargo clippy --locked --manifest-path examples/external/topdown_switch/Cargo.toml --all-targets -- -D warnings
  cargo test --locked --manifest-path examples/external/topdown_switch/Cargo.toml
}

# The 2D browser games: build the WebAssembly player and run every examples/2d game in a real headless browser. Reports SKIPPED (not passed) when the wasm target or a browser
# is missing, and fails instead when RED_CI_REQUIRE_BROWSER=1 (hosted CI). See docs/WEB_PLATFORM.md.
stage_web() { echo "== 2D games in a real browser (WebAssembly player + headless Chromium) =="; bash scripts/web_check.sh; }

# The 3D player for browsers (docs/analysis/2026-10-05-3d-in-the-browser.md): the engine's renderer and simulation must keep compiling for wasm32 under the `web` feature, and lint clean.
# Compile-only: that it draws is `crates/web3d/verify.py`, which needs a browser.
stage_web3d() {
  echo "== 3D browser build (wasm32, feature web) =="
  cargo clippy --locked -p red_engine2 --lib --target wasm32-unknown-unknown --no-default-features --features web -- -D warnings
  cargo clippy --locked -p web3d --target wasm32-unknown-unknown -- -D warnings
}

stages=("$@")
[ ${#stages[@]} -gt 0 ] || stages=(fmt clippy tests benches headless-tree headless-build headless-clippy headless-tests external-client web web3d)
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
