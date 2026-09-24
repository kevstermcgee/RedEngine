#!/usr/bin/env bash
# The exact steps CI runs (.github/workflows/ci.yml). Run before pushing; green here = green there
# (on this platform). Usage: scripts/ci.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== clippy (warnings are errors) =="
cargo clippy --all-targets -- -D warnings

echo "== tests (lib, integration, doctests) =="
cargo test

echo "== benches compile =="
cargo bench --no-run

echo "== sim tree is renderer-free (also enforced by a unit test) =="
if grep -rnE "wgpu|winit|rodio" src/sim --include=*.rs | grep -v "^src/sim/mod.rs" | grep -vE ":\s*//" ; then
  echo "src/sim must not mention wgpu/winit/rodio"; exit 1
fi

echo "CI OK"
