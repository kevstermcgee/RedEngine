# 2026-09-30. Publishing to RedEngineGames never deletes what it does not own
Status: accepted
Summary: publish_games.py export writes the files the manifest publishes and removes only files an earlier export published (per its catalog); everything else in games/, prototypes/, tests/ and demos/ is left alone, because the old delete-the-folder behaviour would have wiped hand-added games on the next engine push.

## Context
`.github/workflows/publish-games.yml` runs on every push to `main` that touches `src/**`, the manifest, the publisher or the published sources, checks out `RedEngineGames`, runs `scripts/publish_games.py export` and pushes the result.
`publish()` did `shutil.rmtree` on each of `games/`, `prototypes/`, `tests/` and `demos/` and moved the staged copy in. `RedEngineGames/games/` had since gained ten minigames (and their launcher entries) that exist only there, so the first engine push after
that would have deleted them from `RedEngineGames` `main` and broken its release playables. It was found while preparing to push the engine work, before the push, by reading the workflow and the exporter.

## Decision
- `publish()` writes exactly the files the export produces (the catalog already records each with its hash), removes the files a **previous** export published (read from the `.games-catalog.json` it left) that this one no longer does, together with the
  folders that leaves empty, and touches nothing else. A catalog path that is not under a managed folder or that contains `..` is ignored, never followed; a symlink or directory where a file is to be written is an error, as before.
- `docs/GAMES_PUBLISHING.md` says so. The manifest still wins over a hand-added file at the same path.
- `tests/publish_games.rs` runs the real script on a throwaway repository: hand-added files survive, previously published files that are no longer produced are removed (folders pruned), an escaping catalog path is ignored, a second export changes nothing, a dropped manifest
  entry is removed next time. It fails against the old exporter (verified) and was checked against a copy of the real `RedEngineGames` `main`: all ten minigames intact, only the catalog and managed files changed.

## Consequences
- Hand-added content in the managed folders is safe; moving it into RedEngine (so the engine is its source of truth) remains possible and is now a choice, not a rescue.
- A file that was published once and later hand-edited in place is overwritten while the manifest still publishes it, as before. The mirror no longer self-cleans anything that was never in a catalog (files added by hand stay until someone removes them).
