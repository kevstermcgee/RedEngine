#!/usr/bin/env bash
# The exact steps CI runs (.github/workflows/ci.yml). Run before pushing; green here = green there
# (on this platform). Usage: scripts/ci.sh [stage ...]      (no stage = all of them, in this order)
#
#   fmt  clippy  tests  benches  headless-tree  headless-build  headless-clippy  headless-tests
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
stage_benches() { echo "== benches compile =="; cargo bench --locked --no-run; }
stage_headless_tree() {
  echo "== headless server: no graphics/audio crates in the dependency tree =="
  if cargo tree --locked --no-default-features -e normal --prefix none | grep -E '^(wgpu|winit|rodio|cpal|alsa|pollster|ffmpeg-sidecar|naga|ash) '; then
    echo "a graphics/audio crate leaked into the headless build"; exit 1
  fi
}
stage_headless_build() {
  echo "== headless server builds without the gfx feature =="
  cargo build --locked --release --no-default-features --bin red_server --bin red_bot
}
stage_headless_clippy() { echo "== headless clippy =="; cargo clippy --locked --no-default-features --all-targets -- -D warnings; }
stage_headless_tests() { echo "== headless tests (incl. real-UDP server tests) =="; run_tests --no-default-features; }

stages=("$@")
[ ${#stages[@]} -gt 0 ] || stages=(fmt clippy tests benches headless-tree headless-build headless-clippy headless-tests)
for s in "${stages[@]}"; do
  fn="stage_${s//-/_}"
  if ! declare -F "$fn" >/dev/null; then echo "ci.sh: unknown stage '$s'" >&2; exit 2; fi
  "$fn"
done
echo "CI OK"
