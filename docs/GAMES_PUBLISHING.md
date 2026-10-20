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

On a push to `main` that changes the engine, a published source, a page, the manifest, or the publisher,
`.github/workflows/publish-games.yml` checks out `RedEngineGames`, writes the files the
manifest publishes into the four collections and the pages (and removes files an earlier export published that the manifest no longer does, as the catalog records), and pushes only when the copy
changed. **It never deletes anything else**: games added to `RedEngineGames/games` by hand are left alone. It can also be run
manually. The workflow authenticates with the repository-scoped SSH deploy key in
the `GAMES_REPO_DEPLOY_KEY` Actions secret. The key can write only to
`RedEngineGames`.

The generated `.games-catalog.json` records the exact RedEngine commit and SHA-256
digest of every copied file. The publisher rejects missing sources, path traversal,
symlinks, and destination collisions before replacing any managed collection.

## Playable releases

The manifest's `playables` array is also copied into the catalog. Each entry names a download, its main published file, every published file or directory it needs, and
the command-line arguments its launcher uses. Its launcher runs the engine's client its `kind` needs (`re2`, or `re2d` for `"kind": "2d"`), or the game's own client
when it names one: `"binary": "killchain"`, a package of the engine's workspace whose binary has its name (ADR 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library).
`RedEngineGames` builds the cataloged RedEngine commit on a Windows runner and releases **each game on its own**, only when
that game's content changed: a per-user installer (desktop shortcut offered, uninstaller, saves in `Saved Games\<game>`) and a portable ZIP, as a permanent GitHub Release
tagged `<slug>-v<N>`. An installed game updates itself in place, and the download site lists every version. Add an entry only after its content is self-contained and
manually playable with the listed arguments. How it works, forcing a re-release after an engine change, and code signing: `docs/DISTRIBUTION.md` in RedEngineGames;
the decision: ADR "Games ship as versioned installers that update in place". The engine reads `RE2_SAVE_DIR` (a folder for a game's settings and saved variables).

A playable may name its **page** on the download site: `"page": "pages/<slug>"`, a folder holding `wiki.json` (the fields and rules of RedEngineGames
`site/wiki.py`), the pictures it names (PNG or JPEG) and `thumb.png` (the site list's picture). The export publishes it to `site/wiki/<slug>/` and
`site/thumbs/<slug>.png` and records those files in the catalog like any other, so the game, its page and its thumbnail arrive in one pull request (a game sent
from a fork: `CONTRIBUTING.md`). The page is not part of the game (it never makes a new version). It is held to the site's own rules before it is published:
`publish_games.py check --games-repo ../RedEngineGames` (the hosted python-tools job checks out RedEngineGames' `site/` for it) and every export into a
checkout that has `site/wiki.py`; that check also refuses a playable with no page at all, unless RedEngineGames has one for it (`site/wiki/<slug>/`) or lists
it as pending. A page without a `page` folder here is maintained in RedEngineGames, by hand.

A playable's optional `"maintainer"` (a GitHub login) says whose game it is: a listed contributor's pull request that stays inside the games they maintain
is marked safe to merge (`scripts/contributed_games.py`, `CONTRIBUTING.md`).

Each sync commit carries one `<slug>: <subject>` line per engine commit of the push that changed a game (`publish_games.py notes --since BEFORE`, merges and
the page left out); RedEngineGames' release tool reads them as the **What changed** of the game's next version, so write commit subjects a player can read.

Every game of the `games` collection that `re2` plays (3D, no `binary`) is played by the CLI in CI's `content` job: `publish_games.py play-check` runs `lint`
(no errors), `nav` when the scene has a nav graph and `verify` when it has `checks`.

A playable's `files` must hold everything its scene names: the score files of its `audio` block, for one. A manifest that leaves one out would ship a game that silently plays no
music, so `cargo test` fails (`tools::publish_check`, test `every_published_playable_ships_everything_its_scene_needs`) and names the playable, the file and the fix. A scene that
names a score that does not exist, or a file that is not a score, does not load at all.

## Saved progress

A project's settings and saved variables are filed under the `id` in its `game.json` (`new-game` writes one), so they follow the game when it is unpacked into another folder or updated. A project without an `id` is filed under its name and folder, as before; when you add one, the engine copies what the old place held to the new one once. Installed games also set `RE2_SAVE_DIR` (`Saved Games\<game>`), which names the folder outright.

## Publishing a game project

A standalone game (made with `new-game`) is published with one command from its directory: `red_engine2 game publish ../RedEngineGames` (or `scripts/red game publish
../RedEngineGames`). It copies the project to `projects/<name>` without `out/`, `.red`, `deploy/` or anything that looks like a key, points `game.json` at the sibling engine,
and adds the playable to `.release-games.json` (`--host` in its launcher arguments when the map has bots, since bots live in a server; `--no-host` to open the map directly).
A project whose `game.json` names its own client (`"binary": "killchain"`) is listed with that binary and without `--host` (such a client hosts from
its own menu); `scripts/red play-local` builds and runs it, and `package` builds every such game of the workspace into the release. It never commits or pushes: review `git status` in the games repository and push it yourself; its Windows workflow builds the ZIP. Push the engine first, so the release is
built from a commit that has your engine changes.
