# wgpu in the browser: feasibility spike (not part of the build)

Evidence for `docs/analysis/2026-10-05-3d-in-the-browser.md`. A standalone crate (own `[workspace]`, never built by CI) that draws a lit, depth-tested cube with
wgpu 30 on a canvas, on the WebGPU backend and on the WebGL2 backend, and reads the pixels back from the GPU to prove it.

```bash
cd examples/external/wgpu-web-spike
cargo build --release --target wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked --root /tmp/wb     # the glue generator; version = the wasm-bindgen in Cargo.lock
/tmp/wb/bin/wasm-bindgen --target web --out-dir web target/wasm32-unknown-unknown/release/spike.wasm
python3 -m http.server 8765 --directory web &                                # a secure context (localhost) is needed for navigator.gpu
PY=~/.cache/red_engine2/browser/bin/python3                                  # the `web setup-browser` Python with Playwright
$PY run.py                                                                   # both backends: init, 60 fps loop, canvas + offscreen readback
(cd ../.. && $PY examples/external/wgpu-web-spike/shaders.py)                      # the engine's real WGSL through the browser's own compiler (needs web/index.html served)
```

Headless Chromium needs `--enable-unsafe-webgpu --enable-features=Vulkan,WebGPU --use-angle=swiftshader --use-webgpu-adapter=swiftshader` for a (software) WebGPU adapter.
Known limit: in that headless setup *no* WebGPU canvas is ever composited into a screenshot (a 10-line plain-JavaScript clear-to-red shows white too), so WebGPU
output is proven by offscreen readback; the WebGL2 canvas screenshots fine.
