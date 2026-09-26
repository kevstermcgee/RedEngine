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
`.github/workflows/publish-games.yml` checks out `RedEngineGames`, rebuilds the four
managed collections, and pushes only when the copy changed. It can also be run
manually. The workflow authenticates with the repository-scoped SSH deploy key in
the `GAMES_REPO_DEPLOY_KEY` Actions secret. The key can write only to
`RedEngineGames`.

The generated `.games-catalog.json` records the exact RedEngine commit and SHA-256
digest of every copied file. The publisher rejects missing sources, path traversal,
symlinks, and destination collisions before replacing any managed collection.

## Playable releases

The manifest's `playables` array is also copied into the catalog. Each entry names a
download, its main published file, every published file or directory it needs, and
the command-line arguments used by its launcher. `RedEngineGames` builds the exact
cataloged RedEngine commit on a Windows runner and creates a permanent GitHub
Release containing one ZIP per playable. Each ZIP includes the engine and a small
`Play-<slug>.exe` launcher. Add an entry only after its content is self-contained and
manually playable with the listed arguments.
