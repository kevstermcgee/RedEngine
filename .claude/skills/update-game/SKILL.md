---
name: update-game
description: Change a RedEngine game its maintainer owns and publish the change as its next version (a new map, rules, art, music, a fix). Use when asked to update, improve, fix or add to an existing game.
---

# Update a game and publish its next version

1. **Whose game is it?** Look it up in `games-publish.json` (`"maintainer"`) or with `python3 scripts/games.py list`.
   - If it belongs to the person you work for, go on.
   - If it belongs to someone else, stop and offer two routes: `/fork-game` (their own version, in its family), or a pull request that the
     game's maintainer reviews and merges.
   - A game with its own client (`"binary"`) is engine code: the other maintainer reviews any change to it.
2. **Branch from the newest main.** `git fetch origin && git switch -c update/<slug> origin/main`.
3. **Change it where its source is.** Find the source path in the playable's `games` entry: `examples/2d/<slug>.game2d.json`, or
   `examples/<slug>/`. A 2D game with a generator (`examples/2d/<slug>/build.py` or `build.js`) changes **through the generator**: edit
   it, run it, and the file it writes is what ships. `python3 scripts/game_generators.py check examples/2d/<slug>` proves the two still match.
   A new map that players should reach needs a way in: the playable's `arguments` open its main scene.
4. **Prove it as you would a new game** (`/make-game` step 4): `validate`, `verify`, look at it. On Windows without the CLI, play it with the
   installed `RedEngine.exe` or `RedEngine2D.exe` and let CI check. Keep its `checks` passing and add one for what you changed.
5. **Update its page** (`pages/<slug>/`) when what it says changed. A new map means a new gallery picture and a line in the overview.
6. **Feedback:** run `/engine-feedback` for what the engine made hard; the note goes in this pull request.
7. **Send it and merge it** (`/make-game` steps 9 and 10). One idea per commit, with a subject a player can read: each subject becomes a
   line of the new version's **What changed**. The merge builds `<slug>-v<N+1>`, and installed copies offer the update.
