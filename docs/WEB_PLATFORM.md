# The web platform: 2D games in the browser

RedEngine has two independent axes: **presentation** (`2d` or `3d`) and **platform** (`web`, `windows`, `linux`). `red_engine2 capabilities` prints what is built for every pair; a game
declares what it needs in its `capabilities` block and an unsupported pair fails at `validate`, naming the target and the way out. Nothing is ever downgraded silently.

| | web (browser) | windows / linux |
|---|---|---|
| **2d** | **SUPPORTED**: `*.game2d.json` -> `web build` -> static package -> `web verify` in a real headless Chromium | PREPARED: runs headless (`sim`, `verify`, `frame`) today; a native window is not built (the app layer's `shell` could show the CPU frame) |
| **3d** | NOT SUPPORTED: needs wgpu and the engine library on WebAssembly | SUPPORTED: the existing engine |

"Supported" means a test runs it. "Unverified" means it is built and nothing here has run it (touch, gamepad). "Prepared" means the design allows it and the code does not exist.

## Architecture

```
                 +--------------------------- red2d (one crate, no GPU, no window, no network) ---------------------------+
 NAME.game2d.json |  game.rs   strict parser, did-you-mean, capability cross-checks                                         |
                 |  sim.rs    fixed 60 Hz simulation, seeded RNG, state hash        <- identical natively and in the browser |
                 |  render.rs CPU rasteriser to an RGBA buffer + letterbox/pointer mapping                                  |
                 |  sound.rs  voices and scores rendered to samples, measured with the shared audio analysis                 |
                 |  script.rs scripted playthroughs, expectations, whole-game verification                                  |
                 |  host.rs   THE PLATFORM BOUNDARY: ticks in, input in, pixels/sounds/save text out                        |
                 |  web.rs    a C ABI over host.rs (wasm32 only), 34 exports, imports nothing                               |
                 +-------------------------------------------------------------------------------------------------------+
                       native: red_engine2 validate|verify|sim|frame|web build          browser: runtime.js + game.wasm
```

* **Simulation, rules, input actions, persistence format, audio descriptions, verification and package metadata are shared.** Only the drawing target and the I/O differ.
* **The renderer is a CPU rasteriser on purpose.** The browser puts its RGBA buffer on a canvas; a screenshot taken natively is the player's pixels. There is no GPU/WebGPU dependence, and `web verify` checks the first canvas frame is *byte-identical* to the native renderer's.
* **3D is unchanged.** The 3D engine does not depend on `red2d`'s renderer; `red2d` shares only dependency-free modules by `#[path]` (`fields`, `suggest`, `rules_expr`, `synth`, `dsp`, `audio_analysis`, `voice_spec`, `audio_fx`, `score`). `rules_expr` is the same expression language for 3D and 2D rules.

Module ownership: simulation `sim.rs` · 2D presentation `render.rs` + `font.rs` · 3D presentation `src/render.rs`, `gpu.rs` (untouched) · platform-native `src/tools/game2d.rs` (headless) · platform-web `web.rs`, `web/runtime.js`, `web/index.html` · audio `sound.rs` + shared `synth`/`dsp`/`score` · storage `Sim::save_json`/`load_save` (format) and `runtime.js` (localStorage) · network transport `src/net` (3D only) · package `src/tools/webpkg.rs` · browser verification `src/tools/webverify.rs` + `web/browser_verify.py` · publisher `src/tools/publish2d.rs`.

## Platform audit (what the browser cannot do, and what the code does instead)

| Area | Native (headless / tools) | Browser | Notes |
|---|---|---|---|
| Rendering | CPU rasteriser -> RGBA -> PNG | same rasteriser -> canvas `putImageData`, CSS scaling, `image-rendering: pixelated` | no GPU path; pixel-identical (checked) |
| Event loop | the caller steps `Sim` | `requestAnimationFrame` + fixed-step accumulator (max 6 ticks/frame, 100 ms clamp) | simulation never sees wall time |
| Input | `Sim::key/set_action/click/set_pointer` | DOM key + pointer events + Gamepad polling, all become the same actions | blur releases every key; first key/click only starts the game |
| Audio | voices/scores -> samples -> measured | the module renders the same samples (the music loop in `audio-worker.js`, off the main thread: rendering it on the page stalled it for up to 1.6 s); `AudioContext` created inside the first gesture; buffers cached | autoplay policy honoured; music starts after the gesture |
| Storage | save text returned by `take_save_if_dirty` | `localStorage` key `red2d:<game id>` behind try/catch | see below |
| Assets | everything is in `game.json` (sprites/sounds/music are descriptions) | `assets/game.json` + `game.wasm`, relative URLs only | no external files; SHA-256 of each in the manifest |
| Timing | `DT = 1/60`, tick counts | same | `time` is `t_ticks / 60`, never a clock |
| Randomness | seeded xorshift64*, `?seed=N` in the page | same code | no `Math.random`, no OS entropy |
| Threads | none | none | one wasm instance, one thread; no `SharedArrayBuffer`, so no cross-origin-isolation headers needed |
| Filesystem | the CLI reads the game file | no filesystem; the game text is fetched | the module has **no imports** (checked by `web check`) |
| Environment variables | CLI only | none | |
| Subprocess | `web build` runs cargo; `web verify` runs Python/Chromium | none | build-time only |
| Networking | none | `fetch` of the three package files, then nothing | see below |
| Startup | `Sim::new` | fetch -> instantiate (`'wasm-unsafe-eval'` in the CSP) -> `init` -> start screen -> first gesture -> running | a CSP without `wasm-unsafe-eval` is a fatal error; `web verify` found this once |

### Browser persistence

* **What persists:** variables named in `persist` and the `music_on` setting (when the game declares `settings`). Nothing else: not the scene, not timers.
* **Format:** `{"red2d_save":1,"game":"<id>","vars":{...},"settings":{"music":true}}`. Values are matched by name, so adding or removing variables is safe. Written only when something worth keeping changed.
* **Failure behaviour (each tested in a real browser):** storage blocked (`SecurityError`) -> the game plays, `status.storage = "unavailable"`, progress is not kept; quota/write failure -> the game keeps running and marks storage unavailable; corrupt text, another game's save, or a newer format -> ignored, the original is copied to `red2d:<id>:unreadable` before anything overwrites it.
* **Version compatibility:** a save with a different `red2d_save` number or a different `game` id is `incompatible`; a save with extra or missing variables loads what matches.
* **Reset:** the game's `reset_save` action, or `__red2d.resetSave()` (also what the test uses) removes the key; the next load is `fresh`.
* **Testability:** `Sim::load_save`/`take_save_if_dirty` are unit-tested natively; the localStorage paths are exercised by `web verify`.

### Browser audio (four separate claims)

1. **Configuration validated** - `validate`: every voice and score parses (paths and did-you-mean).
2. **Waveform analyzed** - `verify`: each sound/track rendered and measured (finite, not clipped, not silent, no DC, no end click, loop seam, loudness).
3. **Browser audio initialised** - `web verify` (`browser-audio` rows): after a real key press the `AudioContext` is `running`, a sound goes through `createBuffer` -> `BufferSource`, music reaches `playing`.
4. **Human listening verified** - never claimed. No check here can say a sound is pleasant or that the right sound plays at the right moment.

### Browser networking

| Mode | Status | Why |
|---|---|---|
| offline | **SUPPORTED NOW** | the game never touches the network after loading |
| authoritative, browser, 2D | **NOT SUPPORTED** | there is no 2D netcode; the authoritative server runs the 3D `MatchSim` |
| authoritative, browser, any presentation | **ARCHITECTURALLY PREPARED** | the simulation does not depend on the transport, so a WebTransport (or WebSocket relay in front of `red_server`) could carry it; not implemented, and the native UDP/QUIC transport cannot run in a browser |
| authoritative, native 3D | **SUPPORTED** | the existing server and clients |

`capabilities` states this in the diagnostic: *"Browser target cannot use the native UDP transport (or QUIC). Supported networking for web games: offline. A browser-compatible authoritative transport ... is architecturally prepared ... but not implemented yet."*

## The static package

```
index.html  runtime.js  audio-worker.js  game.wasm  assets/game.json  thumbnail.png  manifest.json
```

Deterministic: no timestamp, no absolute path, sorted files, `package_id` = hash of (path, SHA-256) pairs; the same game and the same engine give the same bytes. The manifest also holds the game's declared capabilities, screen, engine revision (`+dirty` if the tree was), and the **native** initial state hash, first-frame pixel hash and every scenario's final state hash, which `web verify` makes the browser reproduce.
`web check DIR` needs nothing but the directory: sizes and hashes, no undeclared files, the module imports nothing and exports the whole ABI, no `/home/`-style paths in any file or in the wasm, every file the page loads is declared, the game parses to the revision the manifest names.

## What each command proves

| Command | Proves | Does not prove |
|---|---|---|
| `validate` | the file is well formed; every name resolves; the declaration is buildable | that it plays |
| `verify` | scripted playthroughs end as asserted (deterministic); first/last frames are not blank; sounds are finite/unclipped; saves round-trip | a browser; that it is fun; that it sounds good |
| `web build` | the package exists and is internally consistent | that it runs |
| `web verify` | in headless Chromium: HTML loads, module initialises, state and first frame equal native, scenarios replay hash for hash, real keys/clicks change state, saves survive reload and bad storage is survivable, audio starts after a gesture, console and network are clean | a human played it; other browsers; touch; gamepad |
| `publish` | see `docs/PUBLISHING_2D.md` | |

"Successful WebAssembly compilation" is not browser support: only `web verify` is.
