# The 3D engine in a browser: one bounded step, measured

> **Superseded 2026-10-07:** the browser target was removed (ADR 2026-10-07-native-executables-only-the-browser-target-is-removed). Kept as history.

Follows `2026-10-05-3d-in-the-browser.md` (the plan; phase 1 is merged: Marcel draws and walks in Chromium on WebGPU). This step does **not** add a feature to the player. It adds the two things phase 1 lacked
before anyone can say "the engine runs in a browser": a **second, authored, rule-driven scene** (`recipes/gated_garden.json`: a key, a gate a rule opens, a bench that ends the match) and a repeatable
**measurement and parity harness** (`scripts/web3d_measure.sh`, `crates/web3d/measure.py`, `examples/web3d_parity.rs`). Two small additions to the player make it testable: `Web3d.run(ticks)` (exactly that many
simulation ticks, no drawing, no real time) and `Web3d.state()` (tick, simulation checksum, position, outcome, what the rules switched off or hid).

```bash
scripts/web3d_measure.sh                      # recipes/gated_garden.json and examples/marcel/marcel.json
scripts/web3d_measure.sh path/to/scene.json   # any scene with a spawn
```

It needs the wasm32 target, `wasm-bindgen-cli 0.2.128` (installed to `/tmp/wb` if missing) and a Playwright Chromium (`red_engine2 web setup-browser`). It is **not a CI gate**: it needs a browser with WebGPU and, on CI,
a software adapter whose frame times mean nothing.

## What was measured (headless Chromium 153, **software** WebGPU adapter, 4 CPUs shared with other builds)

The host load average is recorded in every report (`measure-<scene>.json`, `host.loadavg_1m`): a software GPU shares the CPUs with whatever else is running, and it was busy (load 6.8 and 12.7 on 4 CPUs).
Read every time as "this adapter, on this loaded machine, at 480 x 270"; none of them says anything about a laptop or a phone.

| | `recipes/gated_garden` (7.4 KB scene, 12 objects, 3 rules) | `examples/marcel` (3.4 KB scene, generated world) |
|---|---|---|
| **build size** | module 6.98 MB raw, **2.13 MB gzip**; glue 122 KB (20 KB gzip); page 2.5 KB; same module for both | same |
| **build time** | `cargo build -p web3d --target wasm32-unknown-unknown --release`: 110 s after a source change on this box | same |
| **startup** (navigation to a ready game) | 389 to 456 ms: module fetch + compile 102 to 104 ms, scene fetch 9 to 11 ms, GPU device + every pipeline 71 to 100 ms | 358 to 1933 ms: module 110 ms, scene 6 to 14 ms, GPU + pipelines 54 to 1522 ms (the spread is machine load, not the scene) |
| **shaders / pipelines** | 0 shader or pipeline messages; every pipeline built (the engine's WGSL passed the browser's own compiler in the earlier spike: 9 of 9) | same, 0 messages |
| **first picture** | 1,944 distinct colours at 480 x 270 (2,552 at 960 x 540): key, gate, sign text, shadows (`out/web3d/gated_garden.png`) | 21,929 colours (49,091 at 960 x 540), 96 chunks resident (`marcel.png`) |
| **frame time, CPU side** (simulation ticks + recording and submitting GPU commands) | median 2.9 ms, p95 11 ms | median 3.4 ms, p95 10.4 ms |
| **frame time, whole frame incl. GPU + read-back** | median 347 ms | median 5,855 ms (software GPU, load 12.7: unusable as a frame rate, valid only as "the CPU side is not the problem") |
| **memory** | JS heap 9.0 MB; wasm linear memory 3.4 MB | JS heap 3.3 MB; wasm linear memory **207 MB** (the streamed world's chunks and meshes) |
| **asset loading** | module, glue and one scene JSON; **no texture, model or audio file is fetched** (the scenes are JSON and procedural); every asset 6 to 75 ms from localhost | same |
| **input** | keyboard (real key events through the page): **works**; mouse look (`look()` as the page's pointer-lock handler calls it): **works**; touch: **not wired**; gamepad: **not wired** | same |
| **parity with native** | a scripted run (walk to the key, strafe, walk through the gate, onto the bench) ends in **the same 64-bit simulation checksum** as the native engine, `005baf6fa4125c76`, tick 450, outcome `victory`, `gate` collision off, `gate` and `key` hidden, position difference 0.000000 m | the same, `9fa17d6cba28706c` |
| **WebGL2 fallback** | does not start: "this browser has no WebGPU, and the WebGL2 fallback is not built yet (the renderer needs a single-sample path first)" | same |

## What this proves, and what it does not

* **Proven** (it ran, on this machine): the *real* engine renderer and the *real* simulation, built for `wasm32`, run in Chromium on WebGPU for two different kinds of scene; keyboard and mouse look drive the player; **a rule
  that opens a gate changes the collision world in the browser exactly as it does natively** (the world is `collide::PhysicalWorld`, the same in the wasm build; `docs/adr/2026-10-06-one-physicalworld...`); and the
  simulation is **bit-identical across x86_64 native and wasm32** for those scripts. That last result is the one that matters most for a future `web verify` of 3D games: a scenario can be replayed in a browser and compared hash for hash.
* **Not proven**: any real GPU (Chrome on a laptop, Safari, a phone); a frame rate; WebGL2; audio, HUD, menus, touch, gamepad; packaging, publishing, install, offline; browser multiplayer; Marcel's 207 MB on a phone.
* **Found, not fixed**: Marcel's world takes 207 MB of wasm memory after the settle; a phone will need a smaller streaming budget before 3D is promised there.
* **Parity is checked at two scripts**, both bounded; it does not prove every float operation in the engine agrees across platforms. It shows the engine was built for it (`libm` in glam, rapier `enhanced-determinism`) and that
  the claim can now be tested for any scene with one command.

## Proven / experimental / hybrid / planned (kept apart on purpose)

| | |
|---|---|
| **proven, full engine** | the table above |
| **experimental** | the harness, the 7 MB module, the page; no packaging, verification, publishing or install; WebGPU only |
| **hybrid (shipped)** | 2D games with 3D parts drawn by the built-in software renderer, verified in Chromium on every CI run. A different renderer, **not** a smaller build of the engine |
| **planned** | WebGL2 single-sample path, audio through the shared Web Audio path, HUD and menus, touch pads, `web build/verify/publish` for 3D, installability, a real-device performance floor, a smaller streaming budget |

`red_engine2 describe web3d` prints this on one page for a model that has never seen it; `capabilities 3d web` says why a 3D game still cannot declare the web.
