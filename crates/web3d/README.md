# web3d: the 3D player in a browser

The engine's own renderer and simulation (`red_engine2::web3d`) behind a canvas. This crate is only the `cdylib` wrapper `wasm-bindgen` needs.
Status and plan: `docs/analysis/2026-10-05-3d-in-the-browser.md` (phase 1: **a real engine scene (Marcel) draws in Chromium on WebGPU**, the player walks on its hills; not yet: audio, menus, packaging, the WebGL2 fallback).

```bash
cargo build -p web3d --target wasm32-unknown-unknown --release
cargo install wasm-bindgen-cli --version 0.2.128 --locked --root /tmp/wb          # = the wasm-bindgen in Cargo.lock
/tmp/wb/bin/wasm-bindgen --target web --out-dir /tmp/w3d target/wasm32-unknown-unknown/release/web3d.wasm
cp crates/web3d/web/index.html /tmp/w3d/ && cp examples/marcel/marcel.json /tmp/w3d/scene.json
(cd /tmp/w3d && python3 -m http.server 8770 --bind 127.0.0.1) &                    # localhost: WebGPU needs a secure context
$HOME/.cache/red_engine2/browser/bin/python3 crates/web3d/verify.py                # headless Chromium: start, settle the world, read the picture back from the GPU
```

Open `http://127.0.0.1:8770/index.html?run=1` in a WebGPU browser to play (WASD, Shift, Space; `look(dx, dy)` is wired by the page, not yet by this one).
`cargo clippy -p red_engine2 --lib --target wasm32-unknown-unknown --no-default-features --features web -- -D warnings` is the CI stage `scripts/ci.sh web3d`.
