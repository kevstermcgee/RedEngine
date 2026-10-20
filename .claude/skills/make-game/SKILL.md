---
name: make-game
description: Make a new RedEngine game from an idea and publish it to RedEngine Games (the website and the app), with its page and the session's engine feedback. Use when asked to make, build, create or publish a new game.
---

# Make a game and publish it

You are making a new game on RedEngine for one of its maintainers (docs/COLLABORATING.md). It is done when it is **merged**, which publishes
it to the site and the app by itself. Work in this order; each step's rules are in the page it names, so open that page instead of guessing.

1. **Branch from the newest main.** `git fetch origin && git switch -c game/<slug> origin/main`. `<slug>` is lowercase words joined by
   hyphens, and is not taken: check `python3 scripts/games.py list`. A maintainer works in this repository; a contributor works in their fork.
2. **Learn the format, cheapest first.** Run `scripts/dev start "make a <2d|3d> game: <the idea>"` and follow its NEXT. Then read
   `docs/MAKING_GAMES.md`. For 2D, `scripts/dev red describe 2d` explains the format, and `examples/2d/gate-meadow.game2d.json` uses every
   feature. On Windows the CLI is not prebuilt (`scripts/dev start` says so). Read `docs/MAKING_GAMES.md`, `docs/PLAY_2D.md` and the
   examples instead, play your file with the installed `RedEngine2D.exe <file>` or `RedEngine.exe <scene>`, and let CI run the checks.
3. **Build the game as data.**
   - 2D: one file, `examples/2d/<slug>.game2d.json`. A generator that writes it goes in `examples/2d/<slug>/build.py` or `build.js`; keep
     the generator, because CI checks that it reproduces the file.
   - 3D: a folder, `examples/<slug>/`, with its scenes and a `README.md` whose first paragraph is the game's description on the site.
   - Game logic is rules (`describe rules`). Give it a look of its own (palette, prefabs), sound, and `checks` that play it.
4. **Prove it.**
   - Run `scripts/dev red validate <file>`, then `scripts/dev red verify <file>`. For 3D, also run `lint` and `nav`.
   - **Look** at it: `frame`, `plan` or `tour`, or screenshots from the installed player.
   - Fix what they say. "It loads" is not "it plays".
5. **Write its page** in `pages/<slug>/`: `wiki.json`, the pictures it names (PNG or JPEG), and `thumb.png` (640 x 360).
   - Fields: tagline, overview (200+ characters), story (150+), goal, at least 3 rules, controls, at least 1 character with a picture, at
     least 3 gallery pictures of different moments, and `prompt`: the maintainer's prompt **exactly as given**.
   - `pages/snake/wiki.json` is a complete example; RedEngineGames `site/wiki.py` holds the rules.
6. **List it in `games-publish.json`.** Add a `games` entry (`{"source": "<your source>", "destination": "<slug>"}`) and a playable:
   `slug`, `name`, `kind` (`"2d"` for 2D), `entry` (`games/<slug>/<main file>`), `files: ["games/<slug>"]`, `arguments`
   (`content/games/<slug>/<main file>`; add `--host` for a 3D game with bots), `page: "pages/<slug>"`, and `maintainer`: the GitHub login
   of the person you work for (`gh api user --jq .login`). Copy Snake's or Chrono Clash's entry.
7. **Check what CI will check:** `python3 scripts/publish_games.py check --games-repo ../RedEngineGames` (it needs a RedEngineGames clone
   beside this one) and `python3 scripts/feedback.py check`.
8. **Leave the engine feedback:** run `/engine-feedback`. Its note goes in this same pull request.
9. **Send it.**
   - Commit with subjects a player can read ("Add Glass Bridge: cross before the panes crack"); they become the release's **What changed**.
   - `git push -u origin game/<slug>`, then `gh pr create --fill`, adding what the game is and what you played.
   - Watch CI with `gh pr checks <n> --watch`. On a failure, `gh run view <run> --log-failed` says why; fix it and push again.
10. **Merge.** When CI passes, a check comments **Safe to merge** (it only changes this maintainer's game, every file is content, and the
    page is there). The maintainer merges it, or you do with `gh pr merge <n> --merge` if they asked you to. **Needs a maintainer's
    eyes** names what to fix, or what the other maintainer must look at. About 15 minutes after the merge the game is on
    https://kevstermcgee.github.io/RedEngineGames/ and in the app.

Report to the person: the game in two sentences, how to play it, what you verified and how, the pull request, and anything you could not
check.
