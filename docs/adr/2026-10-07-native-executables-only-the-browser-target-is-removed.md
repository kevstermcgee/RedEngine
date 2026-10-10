# 2026-10-07. Native executables only: the browser target is removed
Status: accepted
Summary: Every game ships as a native executable (re2d / the 3D client); the WebAssembly players, the web CLI, the package/publish pipeline and the browser site pages are gone.

## Context
For a week the engine grew a second platform: a WebAssembly 2D player, a 3D player compiled for wasm32, a `red_engine2 web` command family (build, serve, verify in headless Chromium, publish to GitHub Pages), phone on-screen pads and a service worker. It cost a CI job with a real browser, a wasm lint stage, a `web` platform in the capability matrix and a parallel set of games on the download site. The owner's direction is that all games are native `.exe` downloads for now, so none of that is used.

## Decision
- Removed: `crates/web3d`, `crates/red2d/src/web.rs` and `crates/red2d/web/`, `red_engine2 web ...`, `publish`, the browser evidence pipeline and its tests, the `web` cargo feature and profile, the `web` platform and `touch` input, the `controls` key (phone pad), CI's `web2d` job and the wasm32 targets and stages.
- One native 2D player: `red_engine2 play2d` (`src/play2d.rs`: winit + softbuffer + rodio over `red2d::host::Host`). `re2d` (`src/bin/re2d`) is the same code as a small GUI-subsystem program that defaults to `game.game2d.json` beside it and logs failures to `re2d.log`; Windows/Linux releases ship it (`kind: "2d"`, see ADR 2026-10-07-tooling-dependencies-are-optional-features and `docs/PLAY_2D.md`).
- `RedEngineGames` dropped `webgames/` and the browser pages; its Date Night Arcade 2D games are native installers.
- The older web ADRs and analysis files stay as history and carry a superseded banner.

## Consequences
Easier: one platform to test, a smaller CI, no browser-specific evidence rules. Harder: nothing can be tried with a single link any more. 2D on Windows is Supported on the strength of the headless `Host` checks plus a window smoke test on a virtual display; nobody had run it on a real Windows desktop when this landed. To undo: the browser code is in git history before the merge of PR #55; it would come back as a new platform in `crates/red2d/src/caps.rs`.
