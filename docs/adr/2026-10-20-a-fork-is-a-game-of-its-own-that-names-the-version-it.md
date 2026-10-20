# 2026-10-20. A fork is a game of its own that names the version it came from
Status: accepted
Summary: scripts/games.py fork copies a released version (RedEngineGames at its repo_commit) into a new game here whose playable says forked_from {slug, version} and whose maintainer is the forker; the publisher refuses TODO(fork) and unknown parents; RedEngineGames carries the edge into every release and draws the family

## Context
The owner asked (2026-10-20) for forking like GitHub's, built into the site and the app: take a game, tweak it, publish it as your version, and keep a
reliable history of how games evolve and branch. Two maintainers (the owner and meastwood13) and their AI sessions make and update games; anyone's
improvement to someone else's game had no route but editing that game (which needs its maintainer) or starting over. A game's history already exists in
pieces: every version is a permanent release `<slug>-v<N>` whose record (`redengine-release` in its notes) names the RedEngineGames commit it was built
from (`repo_commit`) and the engine commit (`engine_revision`). Nothing said that one game came from another.

## Decision
- **A fork is a new game, not a branch of the old one.** It has its own slug, maintainer, page, saves and versions, and travels the same route as every
  game made here (examples/, `pages/`, `games-publish.json`, one pull request, CI plays it, the merge publishes it). Its playable says
  `"forked_from": {"slug": <parent>, "version": <N>}`: the edge of the family tree, pinned to the version it started from. `"maintainer"` is the forker.
- **It starts from what players played.** `scripts/games.py fork SOURCE[@vN] NEW` reads the version from the site's `catalog.json` and copies the files of
  its playable from RedEngineGames at that version's `repo_commit` (`games_repo_at`, a partial clone when there is no checkout), and its page as it was
  then (or the current one, when the page came later: `page_at`). The engine's own copy may have moved on; the release has not.
- **A 2D fork renames whole strings only**: the game's `id` (saves are filed under it) and `title`, in the game and in its generator folder alike
  (`requote`), so a generator that wrote the original byte for byte writes the fork byte for byte (`scripts/game_generators.py check` proves it for
  Python; Honeybound's forked generator does). A 3D game folder moves whole to `examples/<new>/`, paths into the old folder rewritten, the original's
  development notes (`STATUS.md`, `CLAUDE.md`, ...) and sibling-engine shims (`scripts/red*`) left behind; a `game.json` gets the new `id`.
- **An unfinished fork never ships.** The page starts as the parent's with what must change marked `TODO(fork)` (tagline, overview, the prompt used;
  an AI game's `why_made` removed, since a person's fork has a prompt); `publish_games.py` (`fork_problems`, in `check` and in every export) refuses any
  published file that still says it, and a `forked_from` naming a game neither this manifest nor RedEngineGames publishes. `load_manifest` holds the
  field's shape (a slug that is not the game's own, a version of 1 or more).
- **The one-tap check accepts it** (`PLAYABLE_KEYS`): a fork by a listed contributor is a new game of theirs.
- Not forked by this tool: a game with its own client (`binary`: engine code, reviewed as engine code) and an upload (no source; the app forks those).

## Consequences
The family tree is data: `games.py tree` reads it from the catalog, and RedEngineGames records `forked_from` in the fork's releases and draws it on the
site and in the app (its own change). A fork's history is two lines that meet: the parent's versions up to N, then the fork's own. Merging a fork's
idea back into its parent is an ordinary pull request to the parent's game, which its maintainer reviews. The game's content now exists twice in
the engine repository (each fork is a full copy): fine for JSON games (tens of KB), worth revisiting if large assets get forked often. Undo: drop
`forked_from` from the playables (the games stay, the edges go) and the two checks.
