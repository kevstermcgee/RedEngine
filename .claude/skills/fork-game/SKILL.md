---
name: fork-game
description: Fork a game from RedEngine Games (anyone's) into a new game of your own that keeps its family history, change it, and publish it. Use when asked to fork, remix, make your own version of, or build on an existing game.
---

# Fork a game

A fork is a **new game**: its own slug, page, saves, versions and maintainer. It remembers which game and version it came from, and the site
and the app show it in that game's family (docs/GAMES_PUBLISHING.md, "Forking a game"). The original stays exactly as it is.

1. **Pick the source and the version.** `python3 scripts/games.py list` lists every game, and `python3 scripts/games.py tree <slug>` shows
   its family. The newest version is the default; `<slug>@vN` takes an older one.
2. **Branch, then fork.**
   - `git fetch origin && git switch -c fork/<new-slug> origin/main`
   - `python3 scripts/games.py fork <slug>[@vN] <new-slug> --name "<New Name>"`. The maintainer defaults to `gh`'s login; pass
     `--maintainer <login>` when you work for someone else. It needs a RedEngineGames clone beside this one, or it makes a temporary one.
   - It copies the files that version shipped (and a 2D game's generator), gives the fork its own id and title, adds the playable with
     `"forked_from"`, and starts the page from the original's, with `TODO(fork)` where it must change.
   - It refuses a game with its own client (engine code) and an uploaded one (no source; the app's **Fork…** handles those).
3. **Make it yours.** Change what the person asked for, through the generator when there is one (`/update-game` step 3), and prove it
   (`/make-game` step 4). A fork that only renames the original is not worth publishing.
4. **Write its page.** Replace every `TODO(fork)` in `pages/<new-slug>/wiki.json`: the tagline, an overview that says what is different
   and why, and the person's prompt exactly as given. Add pictures of what changed. `python3 scripts/publish_games.py check` refuses a
   page that still says `TODO(fork)`.
5. **Feedback, send, merge:** `/engine-feedback`, then `/make-game` steps 9 and 10. The new game belongs to the person you work for, so its
   check says **Safe to merge**. Its first release, `<new-slug>-v1`, records the parent, and the site shows the family right away.
