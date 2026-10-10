# Publishing to RedEngineGames

`RedEngineGames` is the public, browsable copy of games, prototypes, test content,
and demos produced with RedEngine. RedEngine remains the source of truth.

The publication list is explicit in `games-publish.json`. Each entry maps a tracked
file or directory into one of four stable collections: `games/`, `prototypes/`,
`tests/`, or `demos/`. Add a manifest entry when new engine-made content is ready
to share; do not point it at build directories, logs, secrets, or unreviewed scratch
output.

Validate the complete export without changing any files:

```text
python scripts/publish_games.py check
```

To inspect the exact result locally, export into a separate directory:

```text
python scripts/publish_games.py export --output ../RedEngineGames-preview
```

On a push to `main` that changes the engine, a published source, the manifest, or the publisher,
`.github/workflows/publish-games.yml` checks out `RedEngineGames`, writes the files the
manifest publishes into the four collections (and removes files an earlier export published that the manifest no longer does, as the catalog records), and pushes only when the copy
changed. **It never deletes anything else**: games added to `RedEngineGames/games` by hand are left alone. It can also be run
manually. The workflow authenticates with the repository-scoped SSH deploy key in
the `GAMES_REPO_DEPLOY_KEY` Actions secret. The key can write only to
`RedEngineGames`.

The generated `.games-catalog.json` records the exact RedEngine commit and SHA-256
digest of every copied file. The publisher rejects missing sources, path traversal,
symlinks, and destination collisions before replacing any managed collection.

## Playable releases

The manifest's `playables` array is also copied into the catalog. Each entry names a download, its main published file, every published file or directory it needs, and
the command-line arguments its launcher uses. `RedEngineGames` builds the cataloged RedEngine commit on a Windows runner and releases **each game on its own**, only when
that game's content changed: a per-user installer (desktop shortcut offered, uninstaller, saves in `Saved Games\<game>`) and a portable ZIP, as a permanent GitHub Release
tagged `<slug>-v<N>`. An installed game updates itself in place, and the download site lists every version. Add an entry only after its content is self-contained and
manually playable with the listed arguments. How it works, forcing a re-release after an engine change, and code signing: `docs/DISTRIBUTION.md` in RedEngineGames;
the decision: ADR "Games ship as versioned installers that update in place". The engine reads `RE2_SAVE_DIR` (a folder for a game's settings and saved variables).

A playable's `files` must hold everything its scene names: the score files of its `audio` block, for one. A manifest that leaves one out would ship a game that silently plays no
music, so `cargo test` fails (`tools::publish_check`, test `every_published_playable_ships_everything_its_scene_needs`) and names the playable, the file and the fix. A scene that
names a score that does not exist, or a file that is not a score, does not load at all.

## Saved progress

A project's settings and saved variables are filed under the `id` in its `game.json` (`new-game` writes one), so they follow the game when it is unpacked into another folder or updated. A project without an `id` is filed under its name and folder, as before; when you add one, the engine copies what the old place held to the new one once. Installed games also set `RE2_SAVE_DIR` (`Saved Games\<game>`), which names the folder outright.

## Publishing a game project

A standalone game (made with `new-game`) is published with one command from its directory: `red_engine2 game publish ../RedEngineGames` (or `scripts/red game publish
../RedEngineGames`). It copies the project to `projects/<name>` without `out/`, `.red`, `deploy/` or anything that looks like a key, points `game.json` at the sibling engine,
and adds the playable to `.release-games.json` (`--host` in its launcher arguments when the map has bots, since bots live in a server; `--no-host` to open the map directly).
It never commits or pushes: review `git status` in the games repository and push it yourself; its Windows workflow builds the ZIP. Push the engine first, so the release is
built from a commit that has your engine changes.
