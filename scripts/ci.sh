#!/usr/bin/env bash
# The exact steps CI runs (.github/workflows/ci.yml). Run before pushing; green here = green there
# (on this platform). Usage: scripts/ci.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== rustfmt (rustfmt.toml is the style) =="
cargo fmt --check

echo "== clippy (warnings are errors) =="
cargo clippy --locked --all-targets -- -D warnings

echo "== tests (lib, integration, doctests) =="
cargo test --locked

echo "== benches compile =="
cargo bench --locked --no-run

echo "== headless server: no graphics/audio crates in the dependency tree =="
if cargo tree --locked --no-default-features -e normal --prefix none | grep -E '^(wgpu|winit|rodio|cpal|alsa|pollster|ffmpeg-sidecar|naga|ash) '; then
  echo "a graphics/audio crate leaked into the headless build"; exit 1
fi

echo "== headless server builds, lints and passes its tests without the gfx feature =="
cargo build --locked --release --no-default-features --bin red_server --bin red_bot
cargo clippy --locked --no-default-features --all-targets -- -D warnings
cargo test --locked --no-default-features

echo "CI OK"
