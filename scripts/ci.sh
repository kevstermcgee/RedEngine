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
#
# Every requested stage runs, and both test groups run, even after a failure: one run shows every failure (a failure in one group must
# not hide that another group never ran). Each stage's output goes to out/logs/ci-<stage>.log; the run ends with one summary: PASS / FAIL
# (with a category, each failed test, where it failed and the command that runs it alone) / NOT RUN per stage, and exits 1 on any failure.
# CI_FAIL_FAST=1 stops at the first failed stage instead (the rest are listed as NOT RUN).
set -uo pipefail
cd "$(dirname "$0")/.."
# Find cargo the way scripts/dev does, so this works from a shell that was started without it on PATH.
for f in "$HOME/.local/toolchain/env.sh" "$HOME/.cargo/env"; do [ -f "$f" ] && . "$f"; done
case ":$PATH:" in *":$HOME/.cargo/bin:"*) ;; *) PATH="$HOME/.cargo/bin:$PATH" ;; esac
export PATH CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-never}"
LOGS=out/logs
mkdir -p "$LOGS"

# The serial suites, read from the index without needing a built binary.
serial_suites() {
  tr -d '\n' < docs/features.json | grep -o '"serial_suites": *\[[^]]*\]' | grep -oE '"[a-z0-9_]+"' | tr -d '"' | grep -vx serial_suites || true
}

# run_tests [cargo flags...]: the whole suite for one feature set. Both groups always run; the result is the worst of them.
run_tests() {
  local serial parallel=() serial_args=() t name rc=0
  serial="$(serial_suites)"
  for t in tests/*.rs; do
    name="$(basename "$t" .rs)"
    if [ "${RED_CI_SERIAL_ALL:-0}" != "1" ] && ! grep -qx "$name" <<<"$serial"; then parallel+=(--test "$name"); else serial_args+=(--test "$name"); fi
  done
  echo "-- parallel group: lib, bins, ${#parallel[@]} suite flag(s)"
  cargo test --locked --no-fail-fast "$@" --lib --bins ${parallel[@]+"${parallel[@]}"} || rc=1
  echo "-- doctests"
  cargo test --locked --no-fail-fast "$@" --doc || rc=1
  if [ ${#serial_args[@]} -gt 0 ]; then
    echo "-- serial group (one test at a time, real-time networking)"
    RUST_TEST_THREADS=1 cargo test --locked --no-fail-fast "$@" "${serial_args[@]}" || rc=1
  fi
  return $rc
}

stage_fmt() { echo "== rustfmt (rustfmt.toml is the style) =="; cargo fmt --check; }
stage_clippy() { echo "== clippy (warnings are errors) =="; cargo clippy --locked --all-targets -- -D warnings; }
stage_tests() { echo "== tests (lib, integration, doctests) =="; run_tests; }
stage_benches() { echo "== benches compile =="; cargo bench --locked --no-run; }
stage_headless_tree() {
  echo "== headless server: no graphics/audio crates in the dependency tree =="
  if cargo tree --locked --no-default-features -e normal --prefix none | grep -E '^(wgpu|winit|rodio|cpal|alsa|pollster|ffmpeg-sidecar|naga|ash) '; then
    echo "error: HEADLESS-001 a graphics/audio crate leaked into the headless build (red_engine2 context HEADLESS-001)"; return 1
  fi
}
stage_headless_build() {
  echo "== headless server builds without the gfx feature =="
  cargo build --locked --release --no-default-features --bin red_server --bin red_bot
}
stage_headless_clippy() { echo "== headless clippy =="; cargo clippy --locked --no-default-features --all-targets -- -D warnings; }
stage_headless_tests() { echo "== headless tests (incl. real-UDP server tests) =="; run_tests --no-default-features; }

# category <stage> <log>: what kind of failure a stage log shows.
category() {
  if grep -q '^error: cannot run\|command not found' "$2"; then echo tool-missing
  elif grep -qE '^error(\[E[0-9]+\])?: |could not compile' "$2" && ! grep -q '\.\.\. FAILED$' "$2"; then echo compile
  elif grep -q '^Diff in ' "$2"; then echo format
  elif [[ "$1" == *clippy* ]]; then echo lint
  elif grep -qE '\.\.\. FAILED$|panicked at' "$2"; then echo test
  else echo other; fi
}

stages=("$@")
[ ${#stages[@]} -gt 0 ] || stages=(fmt clippy tests benches headless-tree headless-build headless-clippy headless-tests)
declare -a summary=()
failed=0
stop=0
for s in "${stages[@]}"; do
  fn="stage_${s//-/_}"
  if ! declare -F "$fn" >/dev/null; then echo "ci.sh: unknown stage '$s'" >&2; exit 2; fi
  if [ "$stop" = 1 ]; then summary+=("NOT RUN  $s (CI_FAIL_FAST after a failure)"); continue; fi
  log="$LOGS/ci-$s.log"
  t0=$SECONDS
  "$fn" 2>&1 | tee "$log"
  rc=${PIPESTATUS[0]}
  secs=$((SECONDS - t0))
  if [ "$rc" = 0 ]; then
    summary+=("PASS     $s (${secs}s)")
  else
    failed=$((failed + 1))
    flags=""; [[ "$s" == headless-* ]] && flags=" --no-default-features"
    summary+=("FAIL     $s [$(category "$s" "$log")] (${secs}s, exit $rc)  log: $log")
    while IFS= read -r l; do summary+=("$l"); done < <(awk -v flags="$flags" -v max=30 -f scripts/ci_summary.awk "$log")
    [ "${CI_FAIL_FAST:-0}" = 1 ] && stop=1
  fi
done
echo
echo "== summary =="
printf '%s\n' "${summary[@]}"
if [ "$failed" -gt 0 ]; then
  echo "CI FAILED: $failed of ${#stages[@]} stage(s). Re-run one test with its repro line; one stage with scripts/ci.sh <stage>."
  exit 1
fi
echo "CI OK"
