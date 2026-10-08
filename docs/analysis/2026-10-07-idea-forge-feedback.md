# What building Idea Forge taught us about Red (2026-10-07)

An AI was asked: *"Create a utility that can create ideas for video games. Each idea must be centered on a totally unique, engaging, fun and creative gameplay mechanic; a story is optional. Build it in RedEngine, publish it to RedEngineGames, and keep track of the issues with development so the engine can improve on future runs."*
It built **Idea Forge**, a native 2D RedEngine game that is also a tool, with `describe 2d`, `validate`, `verify`, `frame`, `sim`, `play2d` and the shipped player `re2d`. It read `crates/red2d/src/reference.rs` (the text of `describe 2d`) before the CLI had finished its cold build, and later read small parts of `game.rs`, `rules_expr.rs` and `font.rs` to answer three questions the reference did not (where `show` goes, what the expression language offers, which glyphs exist); each such place is marked below because it is itself a finding. This file is the issue log of that run, ranked by what it would save the next run. The game itself: `examples/2d/idea-forge.game2d.json` (generated), source and README in `examples/2d/idea-forge/`.

## The short version

* **The engine was enough to build the tool, but only by working around two missing primitives: randomness and string choice.** Every other problem was a papercut. Fixing those two would have cut the game file from 72 KB / 231 hand-expanded text widgets to a few KB, and removed the part of the design that had to be invented (an arithmetic walk that stands in for a random number generator).
* **There is no documented way to publish a 2D game.** `game publish` is 3D-only, `describe 2d` and `search` say nothing, and `docs/PLAY_2D.md` still says packaging is "not built yet". The route exists (RedEngineGames ships 2D installers) and was found by reading that repository's git history.
* **Nothing proves the *shipped* player works, and nothing re-checks a published game against the engine that builds it.** The three 2D releases that exited on start carried browser-only keys that `validate` would have rejected; the release workflow built installers from them anyway. I closed the other half of the gap by hand: I drove the real `re2d` window with real key events (finding 4), which took longer than building the game.
* **Discovery did not help.** The four questions this game needed answered (random numbers, text chosen from a list, a clock, a tool-style screen) found nothing relevant in `search` or `recipe`.
* What worked: `validate`/`verify` (1.3 s for the whole game, including a determinism re-run and a save round trip), `frame` (the picture caught every layout bug), `sim --every` (printed the variables that exposed two real bugs), `effects`, `persist` + the `loaded` event, and error messages that name the fix.

## What was built

| | |
|---|---|
| Idea | one integer `cur` in a mixed-radix space: title adjective x noun x **YOU** (what the hands do) x **BUT** (the law that bends) x **PUSHBACK** x **GOAL** x **STORY** (optional) = 24 x 24 x 32 x 32 x 24 x 24 x 24 = **8,153,726,976** ideas, each centered on a single unusual verb plus the one rule that makes it strange |
| Never repeats | a strike adds a step coprime to the space size (`gcd = 1`), so the walk visits every idea exactly once; seeded from the timing of the first strike and the pointer (finding 1). Locks (keep YOU, change the rest) make a remix and can revisit |
| Player features | forge, five locks, story on/off, save ring of 8 + browse, back 8 deep, music toggle, reel animation, particle burst, six synthesized sounds, an ambient score, state persisted between sessions, a 10-digit code that decodes offline (`build.py decode`) |
| Size | vocabulary 10 KB -> game 72 KB (255 text widgets of which 231 are phrases, 43 rules, 2 effects, 6 scenarios) |
| Verification | `validate` clean; `verify` 26 checks pass in 1.3 s (six scenarios incl. "twelve strikes never repeat an idea", determinism, save round trip); `frame` of splash, mid-reel and settled card; **`re2d` under a virtual display with real key events and a mouse click, then a relaunch that restored the saved idea** |
| Not verified | a human playing it; a real display (frame pacing, vsync); audible sound; the Windows installer actually running on Windows (see "Publishing") |

The vocabulary is the product; quality of the *ideas* is not machine-verified. The scenarios prove the walk and the UI, not that a given combination is fun.

## Ranked engine findings

Each: what happened, what it cost, what I did, what the engine should do, how to know it is fixed.

### 1. No random numbers, no entropy, no integer helpers  *(highest value)*

* **Happened:** a generator needs a random number. Rules have no `random`; the only randomness is a position range (`at: {"x":[a,b]}`) used by `spawn`/`teleport`. `play2d` defaults to `--seed 1`, so that source repeats identically on every launch, and rules have no clock. Floats from a random range are fractional (`cur = 4973254184.25`), and the expression language has no `floor`/`min`/`max`/`abs`.
* **Cost:** the largest design detour. I built a hidden "die" entity, teleported it to a random position and read `die_x` (which is refreshed only at tick boundaries, so reading it in the same rule pass returns the old value: first run seeded 0 and every "random" idea was idea 0), floored with `x - x % 1`, and mixed in `tick` and `mouse_x/y` because the die alone is the same for everyone. Two scenario failures and one silent wrong-result bug (fractional digits: `v == 7` never matched, only a scenario that asserted an exact integer revealed it) came from this.
* **Engine change:** (a) an expression function or action `random(a,b)` / `{"roll": ["var", lo, hi]}` drawing from the seeded sim RNG (deterministic under `--seed`, so scenarios stay reproducible); (b) **default `play2d`/`re2d` seed per launch** (print it; `--seed N` reproduces) so shipped games do not replay one history; (c) `floor round min max abs` (+ `sqrt` if cheap); (d) document that `<sceneid>_x/_y` lag by a tick, or refresh them after `teleport`.
* **Verify:** a 10-line recipe `random-pick` (draw an integer, show one of N things) with a scenario that fixes `--seed` and asserts the draw.

### 2. No strings: choosing text means one widget per phrase

* **Happened:** `text` interpolates numbers only (`{score}`); there are no string variables or tables. To show phrase #7 of 32 I emit 32 text widgets each with `show: "v_you == 7"`, and wrap lines by hand in the generator (no `width`/wrap on text). 184 phrases became 231 widgets and 72 KB; the vocabulary is 10 KB.
* **Cost:** a build script whose only job is to unroll data into widgets, so the game file is generated rather than authored (a generated artifact has to be committed and kept in sync; see finding 7), and edits to wording require a rebuild.
* **Engine change:** a `table` (named list of strings) plus `{"text":{"pick":"you","by":"v_you"}}`, and `wrap`/`width` on text widgets; later, string-valued vars. Keep it declarative and checked: `validate` reports an out-of-range index constant, `verify` can assert `{"text":"...","contains":...}` (the 3D game UI ADR already has `/card` text assertions; 2D scenarios cannot assert on what a player reads).
* **Verify:** the Idea Forge game rewritten on tables is under ~15 KB and its phrase list is edited in place.

### 3. Publishing a 2D game is undocumented and not wired to a command

* **Happened:** after the browser target was removed (ADR 2026-10-07) nothing says how a `.game2d.json` reaches RedEngineGames. `game publish` needs a `game.json` project (3D); `describe 2d` has no publish line; `search "publish a 2d game to RedEngineGames"` returns the ADR that deleted the old path, and `docs/PLAY_2D.md` lists "Packaging a 2D game as its own Windows .exe" under *Not built yet*, while RedEngineGames already releases three 2D installers (`riff-rooftop-rush-v2` etc.). The engine's own `games-publish.json` deliberately holds 2D playables out "until a Windows dry run has built re2d".
* **What I did:** read RedEngineGames' history; the working route is `projects/<dir>/<slug>.game2d.json` plus a `kind:"2d"` entry in `.release-games.json`; `releases.yml` then builds the installer on a Windows runner.
* **Engine change:** `red_engine2 game publish2d FILE.game2d.json ../RedEngineGames` (copy, add the entry, run `validate` + `verify` + the `re2d` smoke from finding 4, never commit); a `describe 2d` line pointing at it; fix the stale `PLAY_2D.md` line; record in `STATUS.md` whether the Windows 2D path has been *played* (releases exist; the last three installs exited at once because the files were invalid for `re2d`).
* **Verify:** a fresh agent, given only "make and publish a 2D game", finds the command from `describe 2d`.

### 4. Nothing exercises the shipped player, and releases are not re-validated

* **Happened:** `verify` and `play2d --max-ticks` run the engine's simulation; users run `re2d`, which validates on start and, on failure, exits at once with the reason only in `re2d.log`. The previous three 2D releases failed exactly this way: the game files in RedEngineGames were written for the removed browser target (platform `web`, `touch`, `controls`, `checks.browser`), `validate` rejects those, and `releases.yml` built and published installers from them regardless (fixed by RedEngineGames PR #2 and releases v2, after users had v1). Separately, nothing shows that a key reaches a game through the OS window.
* **What I did:** built `re2d`, started it under `Xvfb`, set X input focus by hand (a bare X server has no window manager, so focus is nobody's), and injected real key events and a click with python-xlib in a throwaway venv, screenshotting between steps, then relaunched to prove persistence. Space, `1`, `S`, `F`, `V`, `B`, a click on a ui button, the save file contents and the restore all worked. About 20 minutes of tooling nobody should repeat.
* **Engine change:** (a) the release tool runs `red_engine2 validate` (and `verify`) on every 2D playable with the engine build it is about to ship, and fails the release on error; (b) `re2d --smoke` / `verify --player` (start the real binary headless, confirm it opened and ticked, print `re2d.log` on failure); (c) a dependency-free `play2d --drive script.json` that injects keys/clicks inside the player and writes screenshots (same step grammar as scenarios).
* **Verify:** reintroduce `"platforms":["web"]` in a 2D file: the release plan fails with the validate message instead of publishing.

### 5. The native player burns a core on a static screen

* **Measured:** `re2d` (fast profile, Xvfb, software) takes **~93% of one core** while idle on coin-dash, tiny-station and idea-forge alike (RSS 60 MB). It is the present loop, not any game. A tool that sits open all day will spin a fan. Not measured on a real display (vsync may help).
* **Engine change:** pace the loop to the sim clock and redraw only when the frame changed (hash the buffer, or a dirty flag from the sim) and `sleep` to the next tick; record idle CPU in `verify --player`.
* **Verify:** idle CPU under ~5% on the same box.

### 6. Discovery and recipes assume arcade games

* `search` for "random number in a 2d game rule", "show different text depending on a variable", "pick a random item from a list 2d", "wall clock time or date in a rule" returned a generic `describe 2d` chunk, an unrelated UI ADR, the collect-then-exit and timer-lose recipes, and a 3D `wall_clock` asset. The 2D recipe list (key-door, timer-lose, collect-then-exit, health-damage, checkpoint-respawn, spawner-waves, survive-then-escape, shared-effect) has nothing for menu/tool/card screens, text, randomness or save-backed state.
* **Engine change:** recipes `random-pick`, `text-table`, `saved-list` (ring buffer in persisted vars + browse), `tool-screen` (buttons bound to keys, locks, toggles); make `search` surface them for those words. Add this game's buttons-emit-events pattern (`{"button":{..,"do":[{"emit":"x"}]}}` then `event:x` rules with `if` guards: a button's `do` cannot be conditional).

### 7. Authoring papercuts (each cost one failed run)

| Finding | Evidence | Fix |
|---|---|---|
| `show` on a `panel`/`button` goes **beside** the widget, on `text` **inside** it | `ui[59].panel.show: unknown field — valid here: at, size, color` (x10) | the error should say "put `show` next to `panel`, not inside"; or accept both |
| `start` trigger is `{"start": true}`, not `"start"` | `when: expected an object, got string "start"`; the reference lists `start \| every:seconds \| ...` | show the object form in the TRIGGER line; accept the bare string |
| scenario `expect` cannot compare two variables or assert a variable is *not* equal to another | to prove "back returns to the previous idea" I hard-coded a 10-digit number tied to this vocabulary's size; to prove "no repeats" I added a `dups` counter to the **shipped** rules | `{"expr":"cur == h1"}` and `{"var":"a","eq_var":"b"}` expectations |
| `<sceneid>_x` stale in the same pass as a `teleport` | seed 0 on first strike | see finding 1 |
| numbers print rounded in `sim` (`cur=4973254184` while the value was `...184.25`) | the failing expectation named the true value, `sim` hid it | print with a decimal when not integral |
| generated game files | `idea-forge.game2d.json` is produced by `build.py`; the reference has no notion of a source file or `include`; the examples test globs only `examples/2d/*.game2d.json` | an `include`/`vars from table` form (finding 2) removes the need; until then a generator beside the file works and the test reads the generated file |
| font is uppercase-only with a 6 px advance | all prose renders in capitals; 78 characters fit a 480 px line | fine for a pixel tool; a lowercase face would help long text |

### 8. Developer environment

* `scripts/dev worktree` could not seed a target (the main checkout had never built: "no donor target directory"): **cold CLI build 409 s** on the 4-core box (`RED_PROFILE=fast`). `re2d` afterwards was 35 s incremental.
* `scripts/dev preflight` ("~1 s") started a **second cold build in the dev profile** (the fast and dev profiles share nothing) and took **442 s** before its own 19 s of checks. It should use any CLI that already exists, or say up front that it is building and for how long.
* `scripts/dev start` correctly refused to guess ("executable MISSING") and gave one next command; that was good and cheap.

## What the engine did well (keep it)

* `describe 2d` (9 KB) was enough to author the whole file. `validate` named every fix except the two in finding 7.
* `verify` ran six scenarios twice (determinism) plus waveform and save round trip in **1.3 s**, which made about ten build-test-fix cycles cheap.
* `sim --only NAME --every S` printing all variables per interval is what exposed the stale-die and fractional-digit bugs; `frame` found the overlapping footer and the hidden sprite (UI draws above entities) immediately.
* `persist` + the `loaded` event made saved-state restore a three-line rule; the save round trip is verified automatically.
* Effects with params kept the forge/unpack logic in one place.

## Publishing

Route used (finding 3): the game goes to RedEngineGames as `projects/idea-forge/idea-forge.game2d.json` with a `kind:"2d"` entry in `.release-games.json`; its release workflow builds `re2d` and the installer on Windows and publishes `idea-forge-v1`. See "Result" below for what was actually done and what was observed.

## For the next run

1. Start from the findings above in order; 1 and 2 shrink this game by an order of magnitude and are the reason a tool-style game is expensive today.
2. To make more ideas, edit `examples/2d/idea-forge/vocabulary.json` (the part sizes may change; `python3 build.py check` lints characters, line counts and the full-period walk) and rebuild. To use the tool in a workflow: forge, lock the part you like, re-forge the rest, note the CODE, then `new-game --kind 2d` and start from `recipe`.
3. Do not report a 2D game as published until the release workflow has run and the release exists; do not call it playable on Windows until a person has installed it.

## Issue ledger (chronological, as found)

1. Worktree could not be seeded: cold build 409 s. 2. `preflight` second cold build (>2 min). 3. No string variables. 4. No random expression. 5. `show` placement on panel/button (10 errors, 1 run). 6. `start` trigger form. 7. `die_x` stale within the pass that moved it (first strike seeded 0). 8. Fractional digits from random floats (wrong cards, caught by an exact-value assertion). 9. No `floor`/`min`/`max`. 10. `--seed 1` default makes the only random source identical every launch. 11. No documented 2D publish route; stale `PLAY_2D.md`. 12. `verify` does not run `re2d`. 13. Discovery found nothing for four relevant queries. 14. Player at ~93% CPU on a static screen. 15. No scripted native-input proof; hand-built Xlib driver. 16. Footer text overlapped buttons and the anvil sprite was hidden behind the header panel (UI over entities; found by `frame`, fixed in the game). 17. A dead rule (`wipe`) nothing emitted was accepted without a warning (a rule whose `event:` trigger no rule or button emits could be reported by `validate`).
