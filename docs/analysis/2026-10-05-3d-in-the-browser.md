# 3D games in the browser: what is measured, what is not, and a plan

Question: can the real 3D engine (wgpu, rapier, the scene renderer) run in a browser, so a 3D game can be played from a URL like the 2D and hybrid ones?
Short answer: **yes in principle, and the two scariest unknowns are now measured and fine; the work is a bounded engine refactor, not research.** Nothing of the engine itself runs in a browser yet.
Evidence: `examples/external/wgpu-web-spike/` (a standalone crate, never built by CI) and the commands below.

## Measured (headless Chromium 153 here, software GPU)

| Question | Result |
|---|---|
| Do wgpu 30, rapier3d 0.35 (`enhanced-determinism`), glam (`libm`), image (png) and serde_json compile for `wasm32-unknown-unknown`? | **Yes**, one release build in 1m39s. |
| Does wgpu initialise asynchronously on a canvas and run a depth-tested, lit, indexed-mesh pipeline? | **Yes, on both backends**: `BrowserWebGpu` and `Gl` (WebGL2). 120 frames in 2 s, console clean. |
| Does it draw real pixels? | **Yes**, proven by reading the pixels back from the GPU (copy to buffer, async map): 5 distinct colours, cube at the centre, on both backends. The WebGL2 canvas also screenshots and reads back correctly. |
| Does the browser's own WGSL compiler accept the engine's real shaders? | **Yes**: scene, shadow, sky+background, ocean, crosshair, postfx (single and multisampled depth), fx, overlay: **9 of 9, zero errors** (`shaders.py`). |
| Size | A wasm with wgpu (both backends) and a cube: **3.8 MB** after `wasm-bindgen` (4.9 MB raw), glue 110 KB. The engine proper will add to this; not measured. |
| Does the engine ask for anything exotic? | **No**: `required_features: empty`, default limits, no compute shaders. The only non-trivial binding is the ocean's read-only storage buffer in a vertex stage, which WebGL2 does not have. The largest uniform block (`Globals`) is about 1.2 KB, far below WebGL2's 16 KB minimum. |
| Which dependencies cannot go to the browser? | `tokio`/`quinn`/`mio` (UDP/QUIC): **confined to `src/net`** (13 k lines). `mio` alone fails `cargo check --target wasm32` today because it is a non-optional dependency of the library. |

## Not measured, or not possible here

* **WebGPU presentation in headless mode.** No WebGPU canvas is ever composited into a screenshot in this headless software setup (a ten-line plain-JavaScript clear-to-red shows white too), so WebGPU output is proven by offscreen readback, not by a screenshot. Real hardware WebGPU was not tried.
* **Performance.** Software rasterisation says nothing about phones or laptops. The engine's frame cost, memory and load time in a browser are unknown.
* **Pixel equality across backends.** The same frame reads back differently on WebGPU and WebGL2 (the surface format differs, so gamma differs): `web verify` for 3D cannot demand the byte-identical frames the 2D path does. It needs a fixed offscreen format and a tolerance, or one canonical backend.
* **Browser support in the field.** I believe WebGPU is available in current Chrome/Edge, Safari 26 and recent Firefox, with WebGL2 nearly everywhere; check before promising it to players.

## What actually has to change (counts outside `bin/`, `tools/`, `cli/`)

| Blocker | Files | Plan |
|---|---|---|
| `tokio`, `quinn`, `rustls`, `rcgen` (network) | 1 + 3 + 3 + 1 | `net` behind a `native` feature; a browser build is offline-only at first |
| `std::fs` (assets, saves) | 18 | embed the 400 KB of assets in the wasm; saves via the same `localStorage` path as 2D |
| `Instant::now` / `SystemTime` (panics in wasm) | 18 | one clock trait; the browser supplies `performance.now()` |
| `std::thread` | 11 | single-threaded sim in the browser (the sim is already deterministic and fixed-step) |
| `std::net`, `std::process` | 15 + 7 | native only |
| `pollster::block_on` for the GPU | 5 (3 in the library, 2 in binaries) | async init (what the spike does) |
| `rodio` (audio) | 4 | the 2D path already renders sounds to samples and plays them through Web Audio; reuse it |
| `ocean.wgsl` storage buffer | 1 | fine on WebGPU; on the WebGL2 fallback, a texture or a simpler ocean |
| `ctrlc`, `clap`, `env_logger`, `ffmpeg`, `gilrs`, `rfd` | tools/bin only | native only |

The simulation (`sim`, 15.7 k lines) and physics (1.9 k) are graphics-free and already run headless on the server, which is the same property that made 2D easy. The client layer (`app::LocalSession` runs the authoritative simulation in-process for one player) is the right shape for a single-player browser game.

One structural consequence: a wgpu module needs `wasm-bindgen` glue (about 110 KB of generated JavaScript), unlike the 2D module, which imports nothing. The package rule "the module imports nothing" must become "imports exactly the generated glue", and the integrity check must still forbid network URLs and absolute paths.

## Options

1. **Port `red_engine2` in place** (recommended). Add a `web` feature set: gate `net`, `tools`, `cli` and the native-only dependencies, replace the clock, embed assets, make GPU init async, add a thin `web3d` entry crate that owns the canvas, input and the frame loop. Pro: one renderer, one simulation, every existing 3D game works unchanged. Con: touches on the order of 50 files (the counts above overlap), and the crate boundary stays muddy.
2. **Extract a `red3d` core crate** (sim + render + assets, no net/tools) and make both `red_engine2` and the browser build depend on it. Cleaner, the 2D approach again, but a much larger and riskier refactor before anything runs.
3. **Stream pixels from a server.** Needs a server per player, latency and money; no. Listed only to rule it out.
4. **Stay hybrid.** The software renderer already gives 3D parts with a URL today. It has no textures, one light and small worlds; it is the right answer for many games and never the answer for a kart racer.

## Recommended phases (each ends with something that runs in a browser and a check that proves it)

1. **Renderer in a browser.** Option 1's feature split until `cargo check --target wasm32 --features web` passes, then load one engine scene and draw it with the engine's own pipelines on WebGL2 and WebGPU; offscreen readback in `web verify`. *Exit: a screenshot of a real engine scene from Chromium; the size and the first frame time are measured.*
2. **Playable offline.** `LocalSession` in the browser, keyboard/mouse/gamepad and the phone pads through the existing input actions, audio through the shared Web Audio path, saves in `localStorage`. *Exit: a scripted run in the browser reaches the same state hash as native.*
3. **Package, verify, publish, install.** `new-game --kind` for 3D gets a `web` platform; `web build/verify/publish` accept a 3D game; the install/offline/backup features come for free from the 2D package. `capabilities` flips `3d` on `web` from NOT SUPPORTED to SUPPORTED, with a measured performance floor stated honestly.
4. **Later, separately:** multiplayer in a browser (WebTransport or WebRTC instead of QUIC over UDP); this is its own project and is not implied by the above.

## Decisions for you

* Is **single-player in the browser** the right first target (the plan above), with multiplayer deferred?
* Is it acceptable that the first release is **WebGPU with a WebGL2 fallback that may lose the ocean and some quality**, rather than WebGPU only?
* Which existing 3D game should be the proving ground (the kart racer is the most demanding; a walk-around map is the safest)?

## Reproduce

```bash
cd examples/external/wgpu-web-spike && cat README.md      # build, serve, and run.py / shaders.py
```
