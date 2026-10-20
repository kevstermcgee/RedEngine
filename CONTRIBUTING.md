# Sending a game, or an update to one

A game on [RedEngine Games](https://kevstermcgee.github.io/RedEngineGames/) is published from this repository. You change it with **one pull request from
your fork**: the game, its page on the site and its entry in the publish list travel together. Once the owner merges it, everything else is automatic.
You need no Rust toolchain: CI runs the checks you cannot.

## Your game's files

Chrono Clash as the example. Everything for one game lives in two folders, plus one entry in the publish list.

| Path | What it is | Ships in the download |
|---|---|---|
| `examples/chrono-clash/` | the game: its scenes (`speakeasy.json`, ...) and `README.md` (the first paragraph is the game's description on the site) | yes |
| `pages/chrono-clash/` | its page on the site: `wiki.json`, the pictures it names (PNG or JPEG) and `thumb.png` (640 x 360, the picture on the site's list) | no |
| `games-publish.json` | a `games` entry (`examples/chrono-clash` -> `chrono-clash`) and a playable (name, main scene, launch arguments, `"page": "pages/chrono-clash"`) | |

The page's fields and their rules are in [RedEngineGames `site/wiki.py`](https://github.com/kevstermcgee/RedEngineGames/blob/main/site/wiki.py): a tagline, an
overview, the story, the goal, at least 3 rules, the controls, the characters (each with a picture), at least 3 gallery pictures of different moments, and the
prompt that made the game (or `prompt_unavailable`, saying honestly that it was not recorded). `pages/chrono-clash/wiki.json` is a complete example.

Stay inside your game's folders. An engine change is welcome in a **separate** pull request that says what the game needs and why.

## An update, step by step

1. **Start from the newest main.** `git fetch upstream && git switch -c <short-name> upstream/main` (`upstream` = `https://github.com/kevstermcgee/RedEngine`).
2. **Change the game** in `examples/<slug>/`: a new map, bots, rules, music. A new map that players should reach needs a way in: for now each map is its own
   scene, and the playable's `arguments` name the one it opens.
3. **Play it.** With the installed game (`RedEngine.exe` in the game's folder) and no toolchain:
   `RedEngine.exe --playtest --secs 40 --shots 8 --out out\check examples\chrono-clash\speakeasy.json` plays a headless match with your bots and writes
   pictures and a report: it must say `OK`. Play it yourself too: `RedEngine.exe --host examples\chrono-clash\speakeasy.json`.
4. **Update the page** in `pages/<slug>/` when what it says changes (a new map is a new gallery picture and a line in the overview). The playtest's pictures
   (`out\check\shots\`) and F12 in a normal session are good pictures; save them as JPEG at 960 x 540 or so.
5. **Commit with a subject a player can read.** Each commit subject that changes the game becomes a line of the release's **What changed**
   ("Add the Colosseum map", "Bots no longer get stuck on the west stairs"). One idea per commit; no "wip" or "fix".
6. **Push and open a pull request** to `kevstermcgee/RedEngine` `main`. Say what changed and what you played.
7. **Read CI.** `gh pr checks <number>` shows it; `gh run view <run> --log-failed` shows why something failed. The checks that matter for a game:
   - *content validates*: every scene loads; then every published game lints with no errors, its bots' `nav` graph can be travelled with the real movement,
     and its own `checks` pass (`scripts/publish_games.py play-check`). A failure names the game, the command and the engine's message.
   - *python tools*: the publish list is sound and your page meets the site's rules.
   - the rest is the engine's own suite: a game change cannot break it, but it runs.

   Fix what fails and push again to the same branch; the pull request updates itself.

## After the merge

Nothing for you to do. Within a few minutes the game and its page are copied to RedEngineGames; when the game's files changed, a Windows installer and
portable ZIP are built as a new version (`chrono-clash-v2`, `-v3`, ...) with your commit subjects as its **What changed**, and the site lists it. A copy
installed from an earlier version updates itself in place and keeps its saves. A change to the page alone updates the site without a new version.

## Running the checks yourself (optional)

The full command-line tool (`lint`, `nav`, `walk`, `frame`, `verify`, ...) runs on Linux, and on Windows inside WSL, without compiling anything: in a clone,
`sh scripts/bootstrap.sh` installs the prebuilt release built from exactly that checkout's engine sources. Then:

```bash
scripts/dev red lint examples/chrono-clash/speakeasy.json      # 0 errors (warnings are worth reading)
scripts/dev red nav examples/chrono-clash/speakeasy.json       # every edge of the bots' routes can be travelled
python3 scripts/publish_games.py play-check --engine "scripts/dev red"   # what CI runs, for every published game
python3 scripts/publish_games.py check --games-repo ../RedEngineGames    # the publish list, and every page against the site's rules
```

How to build a map, the scene format and the tools: [`AGENTS.md`](AGENTS.md) and [`docs/MAKING_GAMES.md`](docs/MAKING_GAMES.md). How publishing works:
[`docs/GAMES_PUBLISHING.md`](docs/GAMES_PUBLISHING.md).
