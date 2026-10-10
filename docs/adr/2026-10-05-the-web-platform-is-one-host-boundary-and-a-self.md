# 2026-10-05. The web platform is one host boundary and a self-checking static package

> **Superseded 2026-10-07:** the browser target was removed (ADR 2026-10-07-native-executables-only-the-browser-target-is-removed). Kept as history.

Status: superseded by 2026-10-07-native-executables-only-the-browser-target-is-removed
Summary: A browser game is game.wasm (no imports) + runtime.js + the game text behind red2d::host; the package is deterministic and verified from the directory alone, and WebAssembly compilation is never called browser support.

## Context
"It compiles to WebAssembly" proves nothing about a browser: the first browser run of this work failed twice for reasons no native test could see (a Content-Security-Policy without `wasm-unsafe-eval`; `WebAssembly.instantiate` returning `{module, instance}`). The platform also differs in storage, audio start-up (autoplay policy), time and input.

## Decision
Everything a platform needs is `red2d::host::Host`: ticks in, input in, saved text in; RGBA pixels, sound requests, save text and a JSON snapshot out. `web.rs` is a 34-export C ABI over it (no wasm-bindgen) and the module **imports nothing**, so it has no environment, files, clock, network or JavaScript to depend on. `runtime.js` owns only what a browser owns (animation-frame loop, canvas, events, `localStorage` behind try/catch, Web Audio created inside the first gesture, Gamepad). A game becomes a static package: `index.html`, `runtime.js`, `game.wasm`, `assets/game.json`, `thumbnail.png`, `manifest.json` (SHA-256 of every file, the game's declared capabilities, the engine revision, and the **native** hashes of the first state, the first frame and every scenario). The package is deterministic (no timestamp, sorted, relative paths only) and `web check` validates it from the directory alone. `web verify` drives a real headless Chromium (Playwright) and reports each claim separately: browser rows and `browser-audio` rows; "configuration validated", "waveform analyzed", "browser audio initialized" and "human listening verified" are four different claims and the last is never made. The test driver polls with CDP evaluate, never `wait_for_function`, so the package's real CSP stays on during every test.

## Consequences
Easier: most of the web build is tested natively through `Host`; the browser run adds exactly the I/O the module cannot see, and checks pixel equality with the native renderer. Harder: a Python + Chromium dependency for `web verify` (`web setup-browser`; CI installs it and treats a skip as a failure); `localStorage`, audio and the gamepad paths exist only in JavaScript and are tested only in the browser (touch and gamepad are declared **Unverified**). To undo: remove `runtime.js`/`web.rs`; `Host` and the native verbs are independent of them.
