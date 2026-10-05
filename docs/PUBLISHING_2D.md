# Publishing a 2D game

```bash
red_engine2 web verify GAME            # BUILD + play it in a real headless browser. Offline, no credentials.
red_engine2 publish GAME               # the whole pipeline; local backend by default (a static site directory)
red_engine2 publish GAME --backend github-pages --repo ../RedEngineGames [--push]
red_engine2 publish --package out/web/ID   # upload a package `web verify` already passed (refused otherwise)
```

**BUILD is separate from PUBLISH.** Everything through the browser smoke (stages 1-6) works with no credentials and no internet. Stages 7-10 run only after 1-6 passed.

| # | Stage | What it does | If it fails |
|---|---|---|---|
| 1 | validate | parse the game, resolve every name, check the declared capabilities | the file's errors, with paths and fixes |
| 2 | gameplay tests | `verify`: scripted playthroughs, first/last frame, audio waveform, save round trip | the failing rows, with what was found |
| 3 | wasm build | the player (`red2d`, `--profile web`) or `RED2D_WASM` | cargo's tail and the fix (`rustup target add wasm32-unknown-unknown`) |
| 4 | static package | `index.html runtime.js game.wasm assets/game.json thumbnail.png manifest.json` | the reason (a non-web game, a directory that is not a package) |
| 5 | integrity check | `web check` | the files that differ, the path that leaks, the import the module has |
| 6 | browser smoke | `web verify` in headless Chromium | the failing checks (a screenshot is in `out/publish/ID/browser/`) |
| 7 | publication metadata | the game's record: ids, revisions, capabilities, verification, build time | |
| 8 | upload | the backend stores the build | the backend's error |
| 9 | remote smoke | the browser plays the *deployed* copy (waits for a deploy; checks it serves this build) | "never served build X: ..." |
| 10 | url | printed only if stage 9 reached it | |

A failure names the stage and stops. Nothing after it runs.

## Four separate results (never merged, never reported unless they happened)

| State | Means |
|---|---|
| **BUILD SUCCESS** | the package exists, every hash matches, nothing leaks, the module imports nothing |
| **LOCAL BROWSER SUCCESS** | a real headless Chromium loaded it from a local server and played it (pixels equal native, scenarios replay, input, saves, audio started) |
| **UPLOAD SUCCESS** | the files are in the backend's storage (for `github-pages`: pushed) |
| **REMOTE PLAYABLE SUCCESS** | a real browser played the deployed copy at a non-loopback URL |

Not claimed by any of them: that a human played it, that it is fun, that it sounds right, other browsers, touch, a gamepad.

## Backends

* **local** (default, `--site DIR`, default `out/site`): writes the static library. With `--base-url` (where something serves that directory) the URL is real and the remote smoke runs against it. Without it **no URL is claimed**: the stage-9 check runs against a loopback server on the directory and says "this is not a remote check"; the report ends `URL: none. PUBLICATION UNAVAILABLE: the files are at DIR` and names the remaining external step.
* **github-pages** (`--repo` = a checkout of `kevstermcgee/RedEngineGames`): the site is written to `webgames/` and committed. Nothing leaves the machine unless `--push`; the Pages workflow (`site/generate.py`) copies `webgames/` to `<site>/play/`. With `--push` stage 9 waits (up to 10 minutes) for `<site>/play/games/<id>/manifest.json` to serve this build, then runs the browser against it.

## The site (one static library)

```
index.html                       the library page
catalog.json                     every game (schema red2d-catalog/1)
games/<id>/                      STABLE URL: always the newest build (index.html, runtime.js, game.wasm, ...)
games/<id>/game.json             the game's record (schema red2d-game-meta/1)
games/<id>/builds/<build_id>/    IMMUTABLE: every build ever published (build_id = the package id, a hash of its contents)
```

Re-publishing a build is idempotent; a different package under an existing build id is refused. The package itself carries no timestamp, so `build_timestamp` lives only in the record.

## The catalog contract

`catalog.json`: `{"schema":"red2d-catalog/1","games":[...]}`; each entry is the subset of the game's record a library page needs:
`id title description url thumbnail presentation platforms input networking persistence build_id build_timestamp game_revision engine_revision compatibility verification record`.
`games/<id>/game.json` (`red2d-game-meta/1`) adds: `engine_dirty`, `screen`, `urls.stable`, `urls.immutable`, `builds[]` (every build: id, time, revisions, path) and
`verification`: `native.scenarios`, `browser.{engine,checks,passed,package_id}`, `audio.{claims, human_listening_verified:false}`, `human_playtest:false`.
`compatibility`: `requires` (WebAssembly, Canvas 2D), `optional` (localStorage, Web Audio, Gamepad API), `networking` (none after loading), `browsers_verified` (the exact browser build that played it) and `browsers_other: "untested"`.

It is not a storefront: no accounts, prices or ratings. A host that already has a catalog reads `catalog.json` (the RedEngineGames Pages site does: see its `site/generate.py`).

## What is not done by this tool

Anything a human must do: review and `git push` when `--push` was not given; enabling GitHub Pages for the repository; a custom domain. The report names the remaining external step, never a URL that does not exist.
