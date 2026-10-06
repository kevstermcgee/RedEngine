#!/usr/bin/env bash
# The browser half of CI for 2D games: build the WebAssembly player, then build and run every examples/2d game in a real headless Chromium (`red_engine2 web verify`).
# Usage: scripts/web_check.sh [game.game2d.json ...]     (no arguments = every example)
#
# It needs two things this repository does not vendor: the `wasm32-unknown-unknown` Rust target and a Playwright Chromium (`red_engine2 web setup-browser`).
# Missing either is reported as SKIPPED, loudly, and exits 0 - unless RED_CI_REQUIRE_BROWSER=1 (hosted CI sets it), where it is a failure. A skip is never a pass:
# nothing here claims browser support unless the browser ran.
set -euo pipefail
cd "$(dirname "$0")/.."
for f in "$HOME/.local/toolchain/env.sh" "$HOME/.cargo/env"; do [ -f "$f" ] && . "$f"; done
case ":$PATH:" in *":$HOME/.cargo/bin:"*) ;; *) PATH="$HOME/.cargo/bin:$PATH" ;; esac
export PATH

skip() {
  echo "SKIPPED: $1"
  if [ "${RED_CI_REQUIRE_BROWSER:-0}" = "1" ]; then echo "RED_CI_REQUIRE_BROWSER=1: a skipped browser check is a failure"; exit 1; fi
  exit 0
}

if ! rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then
  if [ "${CI:-}" = "true" ]; then rustup target add wasm32-unknown-unknown; else skip "the wasm32-unknown-unknown target is not installed (fix: rustup target add wasm32-unknown-unknown)"; fi
fi

# The 2D path needs no window, GPU or audio device, so the CLI is built without the `gfx` feature (no system graphics/audio libraries; also proves the path is headless).
R=(cargo run --locked -q --no-default-features --bin red_engine2 --)
if ! "${R[@]}" web verify --help >/dev/null; then echo "the red_engine2 binary could not be built (cargo's message is above)"; exit 1; fi

# Is a browser available? (`web verify` reports the fix itself; this only decides skip vs run.)
probe() {
  [ -n "${RED2D_BROWSER_PYTHON:-}" ] && "$RED2D_BROWSER_PYTHON" -c 'import playwright' 2>/dev/null && return 0
  local home="${RED2D_BROWSER_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/red_engine2/browser}"
  [ -x "$home/bin/python" ] && "$home/bin/python" -c 'import playwright' 2>/dev/null && return 0
  python3 -c 'import playwright' 2>/dev/null && return 0
  return 1
}
probe || skip "no headless browser is set up (fix: red_engine2 web setup-browser, or set RED2D_BROWSER_PYTHON to a Python with Playwright)"

games=("$@")
[ ${#games[@]} -gt 0 ] || games=(examples/2d/*.game2d.json)
status=0
for g in "${games[@]}"; do
  echo "== web verify $g"
  "${R[@]}" web verify "$g" || status=1
done
# The whole pipeline, for every game, to the local backend: it must end with a catalog, and without a URL (nothing here serves the site to anyone else).
rm -rf out/site-ci out/publish-ci
for g in "${games[@]}"; do
  echo "== publish $g (local backend)"
  "${R[@]}" publish "$g" --site out/site-ci --out "out/publish-ci/$(basename "$g" .game2d.json)" || status=1
done
[ -f out/site-ci/catalog.json ] || { echo "publish wrote no catalog.json"; status=1; }
# The library page those games are listed on: hearts, filters, in a real browser.
echo "== library page"
py="${RED2D_BROWSER_PYTHON:-}"; [ -n "$py" ] || { home="${RED2D_BROWSER_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/red_engine2/browser}"; [ -x "$home/bin/python" ] && py="$home/bin/python" || py=python3; }
"$py" crates/red2d/web/library_check.py out/site-ci || status=1
exit $status
