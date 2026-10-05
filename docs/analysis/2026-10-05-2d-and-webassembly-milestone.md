# 2D + WebAssembly milestone (2026-10-05)

Branch `web2d` (from `reliability`). Goal: PRESENTATION (2d, 3d) x PLATFORM (native, browser), and a supported path from "an AI makes a small game" to a tested, playable URL, without rewriting the 3D engine.
Contract under test: **AI intent -> capabilities -> 2D game -> headless verification -> rendering -> WASM -> static package -> real browser -> publication.** Where a claim is made below, the check that backs it is named.

## 1. Architecture chosen (ADRs `2026-10-05-2d-games-...`, `...-web-platform-...`, `...-capabilities-...`)

* **`crates/red2d`** (workspace member, `rlib` for the CLI + `cdylib` = the WebAssembly player): `game.rs` strict parser, `sim.rs` fixed-60 Hz simulation with seeded RNG and a state hash, `render.rs` CPU rasteriser + letterbox/pointer mapping, `sound.rs`, `script.rs` scenarios + whole-game `verify`, `host.rs` **the platform boundary**, `web.rs` a 34-export C ABI (wasm32 only; the module imports nothing), `caps.rs` the support matrix, `reference.rs` the one-page authoring reference.
* **Shared with 3D, not copied:** `fields.rs`/`suggest.rs` (strict-field validation, extracted from `strict.rs`), `rules_expr` (one expression language), `synth/dsp/audio_analysis/voice_spec/audio_fx/score` (one audio model), `font_glyphs.rs` (one glyph table). Included by `#[path]`; no refactor of the 3D crate.
* **Not shared, on purpose:** renderers. The CPU rasteriser is identical natively and in the browser; there is no GPU/WebGPU dependence.
* **Module ownership:** simulation `sim.rs` | presentation-2d `render.rs`+`font.rs` | presentation-3d `src/render.rs`,`gpu.rs` (untouched) | platform-native `src/tools/game2d.rs` | platform-web `web.rs`+`web/runtime.js`+`web/audio-worker.js`+`web/index.html` | audio `sound.rs`+shared audio files | storage `Sim::save_json/load_save` (format) + `runtime.js` (localStorage) | network transport `src/net` (3D only) | package `webpkg.rs` | browser verification `webverify.rs`+`web/browser_verify.py` | publisher `publish2d.rs` | planning `propose.rs`. Registered in `docs/features.json` as `web2d`.

## 2. What exists

| | |
|---|---|
| Capability declaration | `capabilities` block in a 2D game and (optional) in a 3D project's `game.json`; `capabilities [--file F] [Q]`; four levels Supported / Unverified / Prepared / Not supported; only the first two pass; messages carry the reason and the way out |
| 2D | sprites (paletted pixel art, animation frames, `flip`), primitives (rect, circle), text with `{var}` placeholders, layers, camera follow + world bigger than the screen + shake, HUD (text, bars, panels, buttons with key bindings, `show` conditions), particles (bursts and emitters), AABB collision with solid resolution (no tunnelling), gravity/bounce/friction/drag, movers (keys top-down and platformer, pointer, chase, drift, wander, patrol), tile maps, mouse (click rules, hover, buttons), keyboard, gamepad and touch (mapped to the same actions, **unverified**), audio (voices, scores, music toggle), resolution independence (fit / integer scale, any aspect ratio, pointer mapped through the module) |
| Not in 2D | rotation and non-integer scaling of sprites (translation, integer scale and mirroring only); tweened animation; a native window; networking; scripting beyond rules + expressions (the format is data on purpose: strictness and token cost over expressiveness) |
| Headless | scripted players (`hold`, `hold_until`, `press`, `click`, `button`, `approach`, `wait_until`) and assertions on variables, outcome, counts, positions, events, sounds, state hash; deterministic (two runs, one hash, checked on every scenario) |
| Front door | `describe 2d` (5.6 KB, drift-tested against the parser), `new-game --kind 2d` (a verified starter), `propose`, `capabilities`, `search` indexes the 2D material |
| Browser | `web build / check / serve / verify / setup-browser`; `publish` |

## 3. The three games (all verified headless and in a real browser)

| Game | Kind | Exercises | Scenarios | Verify | `web verify` |
|---|---|---|---|---|---|
| Coin Dash | top-down arcade (keys) | collision with solids, chasers, sprite animation, particles, persisted best, music toggle, restart | 4 | 0.5 s | 38 checks |
| Moon Hopper | platformer (keys + gamepad), scrolling camera, tile map | gravity, jump, bounce pad, hazards, teleport, persisted best | 4 | 0.9 s | 38 |
| Tiny Station (showcase) | mouse-driven management, 7.5-minute run | 6 build tools, economy balance, meteors with shield, solar flares, pause, restart, best score, HUD, 10 sounds, generated music | 7 (one is a full 450 s run) | 2.0 s | 39 |

Engine fixes made *because* of the games (not worked around): `hold_until` script step and `sim --every N` (a balance that is off needs a time series), `{var:.0}` formatting, sprite `flip: auto`, `count_<tag>` and `<id>_x/_y` variables, "rules run in order, each seeing the changes before it" (a "not enough credits" rule that ran after the purchase rule fired on every purchase; now documented in the reference), shared glyph table.

## 4. What the real browser found that nothing native could

1. The page's CSP lacked `'wasm-unsafe-eval'`: the module would not instantiate. (`web verify`, first run.)
2. `WebAssembly.instantiate(bytes)` returns `{module, instance}`; the runtime used the wrong object. (same run)
3. Music rendered on the main thread froze the page for 0.6 s (Coin Dash), 1.1 s (Moon Hopper) and 1.6 s (Tiny Station) after the first key press. Now rendered in `audio-worker.js`; the page stalls 0 ms (checked with a long-task observer).
4. Playwright's `wait_for_function` evaluates a string in the page, which the real CSP forbids; the driver polls through CDP so the policy stays on for every test.

Native-side finds: a full CI run flagged the 2D examples being walked as 3D scenes by `tests/checks_wellformed.rs` (fixed); the first-read budget test (`describe` + `describe --brief`) rose from 9.5 KB to 9.75 KB with four new commands (budget set to 9.8 KB with the reason in the test).

## 5. Platform boundary, persistence, audio, networking

See `docs/WEB_PLATFORM.md` for the audit (rendering, event loop, input, audio, storage, assets, timing, randomness, threads, filesystem, env vars, subprocess, networking, startup) and the four separate audio claims. Summary:

* **Persistence (browser):** `localStorage` key `red2d:<id>`; `persist`ed variables + `music_on`. Tested in a real browser: survives reload; storage blocked (SecurityError) -> plays, says so; quota failure -> keeps running; corrupt, other-game and newer-format saves -> ignored and copied to `red2d:<id>:unreadable`; `reset` -> fresh. Native: `Sim::load_save/take_save_if_dirty` unit-tested.
* **Audio:** config validated, waveform analysed, browser audio initialised (context `running` after a real key press, a sound through `createBuffer`, music `playing`) are three separate rows; "human listening verified" is never claimed.
* **Networking:** SUPPORTED NOW: offline. ARCHITECTURALLY PREPARED: browser-authoritative (WebTransport or a WebSocket relay in front of `red_server`; the simulation is transport-independent). NOT SUPPORTED: native UDP/QUIC in a browser; any 2D netcode.

## 6. Package, verification, publishing

* **Package:** `index.html runtime.js audio-worker.js game.wasm assets/game.json thumbnail.png manifest.json`; deterministic (two builds are byte-identical: tested); `web check` needs only the directory (hashes, undeclared files, absolute paths, module imports/exports, game revision). The module is 554 KB (210 KB gzipped), a package 0.6 MB.
* **`web verify` rows (26 browser rows per game):** HTML loads; module initialises; initial state hash equals native; first canvas frame is **pixel-identical** to the native renderer; every file resolves and matches its SHA-256; every scenario replays in the browser to the native final hash; resolution independence at four window shapes; waits for the player; audio; time advances; the game's own real-input checks; persistence across reload; storage failure modes; clean console.
* **`publish`:** ten named stages, the four successes separate, a URL only when one exists. Backends: `local` (static site: `games/<id>/` newest, `games/<id>/builds/<build_id>/` immutable, `game.json`, `catalog.json`, `index.html`) and `github-pages` (commits into a RedEngineGames checkout; `--push` pushes; waits for the deploy; browser-checks the deployed copy). Contract in `docs/PUBLISHING_2D.md`. The RedEngineGames site generator (`site/generate.py`) got a 49-line patch that lists `webgames/` under "Play in your browser".
* **What was actually exercised:** the whole pipeline to the local backend (all ten stages; ends `PUBLICATION UNAVAILABLE`, no URL); the Pages backend against a local bare remote and a loopback stand-in for Pages (all ten stages, `UPLOAD SUCCESS: yes`, `REMOTE PLAYABLE SUCCESS: no` because the URL is loopback, and the report says so); `generate.py` run on the result. **Not done: pushing to the real GitHub repository.** That is the one remaining external step; nothing was published, no URL was claimed.

## 7. Adversarial findings (all are tests; `tests/web2d_games.rs`, `web2d_package.rs`, `web2d_publish.rs`)

web + native-only target -> "prepared, not built"; browser networking -> the exact UDP/QUIC message; wrong persistence config (persist without `progress`, `cloud`, music without `settings`) -> the fix; 3D-only capability in 2D (`lights`, `mesh`, `rotation`, `presentation: 3d`) -> unknown field / "3D cannot target web"; missing sprite / sound / prefab / tag / variable / scene id -> did-you-mean with what exists; wrong-width sprite row, unknown palette letter, bad colour; vacuous scenarios and empty `do` refused; incompatible declarations (`wev`, empty, `macos`, `p2p`); missing web asset, modified file, extra file, absolute path, external URL, undeclared fetch, runtime calling a non-export, module importing the host or not a module, revision mismatch -> `web check`; publish before verification, after a failed verification, after the package changed, with a tampered package -> refused with the command to run first; a game that fails validation or its playthrough stops at stage 1 / 2 naming it. Bugs these found in my own code: a build id that did not cover the manifest (immutability check now compares the file listing); HTTP/1.0 replies rejected by the remote smoke; `propose` read "shop" as "hop".

## 8. Fresh-agent result (a context that took no part in building this)

Prompt, verbatim: "Create a small 2D arcade game. Make it playable in the browser and publish it." (plus: work in /tmp, edit nothing in the repo, no push, keep a log). The model was interrupted by an API safety classifier twice and resumed; the log notes it.

* Built "Starfall" (catch stars, dodge rocks, 3 lives, 4 sounds, particles, persisted best) from `new-game --kind 2d` in one authored version: `validate`, `verify` (16/16), `frame`, `web verify` (32 checks), `publish` all passed first time; **no rule or balance repairs**.
* Counts (its own log, approximate): 8 text files + 1 image read; about 37 shell commands in 21 tool calls; 1 repair cycle; no material wrong conclusions.
* Path it took: `CLAUDE.md`, `AGENTS.md` -> `describe --brief` -> `describe 2d` -> `capabilities`, `propose`, `new-game` -> read the starter and one example -> wrote the game. It never opened a source file or any 3D doc.
* Outcome: `BUILD SUCCESS yes, LOCAL BROWSER SUCCESS yes, UPLOAD SUCCESS yes (a local directory), REMOTE PLAYABLE SUCCESS no, URL none` and it reported exactly that, not a URL.
* Friction found and fixed (generic, bounded): (1) the scaffolded `scripts/red` was not executable, so the `next:` hint failed with "Permission denied" (every `new-game` kind; now `chmod +x`, tested); (2) its one `search` ("spawn falling objects random position 2d") returned only 3D fragments (the 2D reference and docs are now in the search corpus; tested); (3) `web verify` needed a one-time `web setup-browser` — the error already said so, it did it, ~1-2 minutes, internet needed once.

**3D comparison run (partial).** A second fresh model was given the equivalent 3D task ("collect five gems in a first-person room before a 20 s timer; prove it headless"). It was interrupted by the same safety classifier four times and I stopped it before it reported; its log (25 steps) is the only evidence: it read `describe rules`, `describe sim`, `describe ui`, ran `recipe`, `search` x3, `new-game`, read the scaffold, wrote a 6.8 KB blueprint, then `build-all`, `check`, `sim`, `ui-check`, `ui-shot`, with one repair cycle (a malformed shell command of its own). One run each, one model, interrupted: **no conclusion about 2D being cheaper for an AI rests on this**, and none is claimed.

## 9. Equivalent minimal mechanic, 2D vs 3D (informational; the author knew both)

Mechanic: collect five gems with the movement keys before a 20-second timer; win/lose; two headless checks.

| | 2D (`new-game --kind 2d` starter) | 3D (`docs/analysis/gem-grab-3d/gem-grab-3d.json`) |
|---|---|---|
| Authored file | 3.3 KB (2.6 KB compact), 1 file; sprites-free (rect + circle), 1 sound, saved best, restart button, particles | 3.9 KB (2.2 KB compact), 1 file; default avatar, camera, lighting and generic HUD; no sound, no save, no restart, no particles |
| Read first (bytes of tool output) | `describe 2d` 5.6 KB | `describe rules` 6.1 KB + `describe sim` 3.8 KB (+ `describe objects` 11.7 KB if the object fields are not known) |
| Verify | 0.01 s | 0.01 s |
| Repair cycles authoring it | 0 | 1 (`color` is `material.color`; the engine said so) |
| Output | runs in a browser; publishable | windows/linux only; **cannot** be built for the browser |

What this does and does not show: the *authored* sizes are comparable (the 3D engine supplies a lot by default; the 2D starter carries sound, saves and a restart too). The 2D path is smaller to *read* and ends in a URL; the 3D path has more to configure for look and feel. The 2D format has nothing to replace a 3D world, physics props or networking. Token counts were not measured and are not claimed.

## 10. Duplication audit (2D-native, 2D-web, 3D-native, 3D-web)

Shared semantics (one implementation): simulation (2D native and 2D web run the same `sim.rs`; the check is the state hash), rules expressions, strict-field validation and did-you-mean, audio descriptions and analysis, glyph table, the capability matrix (3D projects declare it in `game.json`; checked by `game check`), verification report shape (rows with a claim), package/publication metadata (2D). Not shared and still duplicated: (a) input mapping tables (2D `action_for_key`, 3D controller bindings); (b) the "game record" schema for the 3D release site (`games/<slug>/latest.json`) and the 2D one (`red2d-game-meta/1`) describe the same things (id, title, revisions, platforms) in different words; (c) 3D "Saved Games" and the 2D save format; (d) the 3D and 2D `verify` row printers. None of these is a bug; (b) is the first thing to unify if the library site becomes one catalog.

## 11. Measurements (this machine: 4 cores, no GPU, Chromium 153 headless shell, loaded by other builds at times)

| | |
|---|---|
| WebAssembly player | 554 KB raw, 210 KB gzip; imports nothing; 34 exports |
| Player build (`--profile web`), warm | about 10-13 s; the CLI itself builds in about 1-2 minutes cold |
| `verify` (native, headless) | Coin Dash 0.5 s, Moon Hopper 0.9 s, Tiny Station 2.0 s (includes a 450 s simulated run, twice, for the determinism row) |
| `web build` (package only) | 0.1 s |
| `web verify` | 14-21 s per game (26 browser rows + 12 package rows) |
| `publish` end to end (local backend) | 19-29 s |
| Music render | 0.6 s / 1.1 s / 1.6 s; off the main thread since the worker change |
| Native vs browser | first frame and every scenario hash identical, all three games |
| `setup-browser` from nothing | about 1-2 minutes, internet needed once |

## 12. Native and 3D regression, full verification

Local, on Linux, a clean worktree (`scripts/ci.sh`, the exact stages CI runs):

* Revision `3fbe42d`: **rustfmt ok; clippy `-D warnings` ok (both packages); tests 1445 passed, 0 failed (lib, every integration suite incl. the real-UDP ones, doctests); benches compile; headless dependency tree has no graphics/audio crate; headless build and clippy ok; headless tests 1352 passed, 0 failed.** The 3D engine's suites (rendering-independent ones, networking, physics, rules, replay, audio, shaders) all ran and passed; no existing test was weakened. Two budgets were raised on purpose and say why: `describe` overview 7.3 -> 7.8 KB and the first-read budget 9.5 -> 9.8 KB (four new commands, two topics).
* Revision `eef53f5` (adds only: the Pages-generator patch script, ownership entries, and the external example's refreshed `Cargo.lock`): **external custom client ok; 2D browser stage ok** (WebAssembly player built, all three games `web verify`: 38/39/39 checks, then `publish` to the local site: BUILD yes, LOCAL BROWSER yes, UPLOAD yes, REMOTE PLAYABLE no).
* What the first full runs found (all fixed): the checks walker treating 2D games as 3D scenes (`checks_wellformed`), unregistered new files (`features_index`, `docs_fresh`), the first-read budget, a stale package-file list in a test after the worker was added, the external example's lock behind the new `red2d` dependency.
* Not run: the Windows leg and the hosted workflow (they run on the pull request); the Windows leg compiles `red2d` for the first time there. `scripts/ci.sh` does not run the 3D GPU-dependent presentation tests when no adapter exists (`RED_OFFSCREEN_OPTIONAL=1`, as hosted CI).

## 13. Known unsupported or unverified combinations

3D in a browser; any networking in a browser; any 2D networking; a native window for a 2D game (headless works); macOS; touch and gamepad in the browser (built, never run on a device); other browsers than Chromium; sprite rotation; a human playtest or listening test (none was done); real GitHub Pages deployment (not done); Windows-hosted `web verify` (written portably, never run there); music loops longer than a few seconds on a very slow phone (worker, but still seconds of CPU).

## 14. Next three

1. Run the Pages publish for real (one `--push`), then unify the 2D and 3D game records so the library lists installers and browser games from one catalog.
2. A native window for 2D games through the app layer's shell (the CPU frame as a texture), which turns the Prepared row into Supported and gives 2D games Windows/Linux installers.
3. Browser-authoritative networking for the existing simulation (WebTransport or a WebSocket relay in front of `red_server`) — the first step is a transport trait and a loopback test, then the matrix row moves from Prepared.
