# Making RedEngine together

RedEngine has two maintainers with equal say: **@kevstermcgee** and **@meastwood13** (`.github/contributors.json`). Each has write access
to RedEngine and RedEngineGames. Each publishes games, updates and forks them, reviews the other's work, and sends feedback the project
treats as its own. Their AI sessions work the same way, from the same instructions. A third person can be a **contributor**: their game
pull requests are checked the same way and wait for a maintainer's tap.

## The loop

```text
 make or fork a game ──> one pull request ──> CI plays it ──> merge ──> RedEngineGames: installer, site, app (about 15 min)
        ^                                                                            │
        │                                                     players, the app, the site, AI sessions
        │                                                                            │
 the next game is easier <── daily improver (23:00): PRs that merge on green <── feedback inbox
```

## Start a fresh AI session

Open Claude Code in a RedEngine clone and use a skill (`.claude/skills/`). Each one knows the whole route:

| Skill | What it does |
|---|---|
| `/make-game <idea>` | a new game, from the idea to its release on the site and in the app, with its page and its feedback note |
| `/update-game <slug> <what to change>` | a new version of a game you maintain |
| `/fork-game <slug> <your name for it> <what to change>` | your own version of anyone's game, in its family on the site |
| `/engine-feedback` | what the engine, the app or the site cost this session, as findings the daily improver ranks |
| `/inbox` | what everyone said lately, open pull requests, and what tonight's improver will see |

An AI without skills gets the same from: "Read docs/COLLABORATING.md, then make a game about ..." (or update, or fork).

## Who changes what

| Change | Who merges it |
|---|---|
| your own game (its playable says `"maintainer": "<you>"`) | you, once CI passes and the check comments **Safe to merge** (`gh pr merge <n> --merge`) |
| someone else's game | fork it (`scripts/games.py fork`, or Fork in the app), or send a pull request its maintainer merges |
| a new game | you: it is yours from the pull request that adds it |
| the engine, the app, the site | the other maintainer, after a look; the daily improver's own pull requests merge when every check passes |
| `.github/contributors.json`, CI, releases of the app | both maintainers agree first |

Nobody, the daily improver included, changes a maintained game without its maintainer. Findings about a game go to its maintainer.

## Publishing

A merge to RedEngine's main publishes by itself. RedEngine copies the game and its page to RedEngineGames, and RedEngineGames builds a
Windows installer and a portable ZIP as a new version (`<slug>-v<N>`), with your commit subjects as **What changed**. The site and the app
list it, and installed copies update in place. The route, what CI checks and how to write the page: [`CONTRIBUTING.md`](../CONTRIBUTING.md)
and [`docs/GAMES_PUBLISHING.md`](GAMES_PUBLISHING.md). A ready-made game (a ZIP or an .exe) is published from the app: drop it on the window,
then choose **Publish for everyone**.

## Forks and families

A fork is a new game made from a version of another one. It has its own page, versions and maintainer, and remembers its parent
(`"forked_from"`). Every game page on the site and in the app has a **Family** box, and the site's `/family/` page draws every family.
- From source, with an AI (it becomes part of RedEngine: CI plays it and every engine update rebuilds it): `/fork-game`, or
  `python3 scripts/games.py fork <slug>[@vN] <new-slug> --name "<Name>"`.
- In the app: **Fork…** copies the installed game. Change the copy, then drop its folder back on the window to publish it.

`python3 scripts/games.py tree <slug>` prints a game's family.

## Feedback

Everything below reaches one inbox: `python3 scripts/feedback.py digest` ([`docs/feedback/README.md`](feedback/README.md)). The
[daily improver](DAILY_IMPROVER.md) reads it every night. It fixes what it can verify, closes the issue or the key it fixed, and its pull
request tells both maintainers what changed.

| From | Where | Trusted as |
|---|---|---|
| an AI session (yours or your cousin's) | `/engine-feedback`: a note in `docs/feedback/`, sent with the game's pull request | a maintainer's word, ranked with the Idea Forge's by `key` |
| you, about the engine, the app or the site | the app's **Engine feedback** tab sent signed (a GitHub issue), or an issue | a maintainer's word |
| a pull request | a section headed **Engine notes** in its description | its author's word |
| players | a review or an engine note on the site or in the app (anonymous) | opinions, quoted short |
| the Idea Forge (06:00, two AI-made games a day) | `docs/analysis/idea-forge/` | ranked findings |

## Setting up as a maintainer (once)

1. Accept the GitHub invitations to `kevstermcgee/RedEngine` and `kevstermcgee/RedEngineGames`.
2. Install Git, the GitHub CLI and Python 3. On Windows: `winget install Git.Git GitHub.cli Python.Python.3.12`. Then run
   `gh auth login`. Node is only needed for JavaScript generators.
3. `gh repo clone kevstermcgee/RedEngine` and `gh repo clone kevstermcgee/RedEngineGames` side by side. A maintainer works on branches
   in the repository itself, with no fork needed.
4. In the RedEngine Games app, open **Publishing…** and paste a token. A collaborator uses a classic token with the `public_repo` scope:
   fine-grained tokens only reach your own repositories. The token publishes dropped games and sends your engine notes signed.
5. The command-line engine (`scripts/dev red ...`: `validate`, `verify`, `frame`, ...) is prebuilt for Linux. On Windows use WSL
   (`sh scripts/bootstrap.sh`), or let CI run the checks on your pull request. `RedEngine.exe` and `RedEngine2D.exe` from any installed game
   play your files.
