# 2026-10-20. A game and its page are sent in one pull request
Status: accepted
Summary: A playable may name its page folder; the publisher copies it to RedEngineGames' site, checked by the site's own rules, and the sync commit carries each game's commit subjects as its release notes, so a contributor needs one pull request and no toolchain.

## Context
Chrono Clash (#199) was the first game sent from a fork, by a contributor on Windows with the installed game and no Rust toolchain. Publishing it took
four things the contributor could not do: run `lint`/`nav`/`verify` (the pull request said so), approve its first CI run, write its page and thumbnail
(those lived only in RedEngineGames, whose site build fails for a released game without a page), and say what changed (RedEngineGames' release tool skips
the `Sync RedEngine content` commits that are the only way a synced game changes there, so every synced game's release notes were empty). The owner did
them by hand. Every later update would have needed the same.

## Decision
One pull request to this repository carries a game, its page and its publish entry; CI runs the checks; the merge does the rest.
- A playable may name `"page": "pages/<slug>"` (`wiki.json`, its pictures, `thumb.png`). `scripts/publish_games.py` (`page_files`) exports it to
  RedEngineGames `site/wiki/<slug>/` and `site/thumbs/<slug>.png`, records the files in the catalog (`is_managed`), so dropping it removes them and a page
  made by hand there is never touched.
- The page is held to **the site's own rules**, never a copy: `page_problems` loads RedEngineGames `site/wiki.py`. `check --games-repo` runs it (the
  python-tools job checks out RedEngineGames' `site/`), and so does every export into a checkout that has it; it also refuses a playable with no page anywhere.
- `play-check` runs `lint` (no errors), `nav` (when the scene has one) and `verify` (when it has `checks`) on every `games` playable `re2` plays, in the
  `content` job.
- `notes --since BEFORE` writes `<slug>: <subject>` per engine commit of the push into the sync commit; RedEngineGames' `change_notes` reads them.
- A pages-only pull request compiles nothing (it counts as documentation in the workflow's `changes` step); `publish-games.yml` also runs on `games/**`
  (RedCraft's and Frameless's content never triggered a sync before) and `pages/**`.

## Consequences
A contributor's whole loop is in `CONTRIBUTING.md`; after the first merged pull request their CI needs no approval. A page published from here is
edited here (RedEngineGames' `site/wiki.py` says so); an export overwrites a hand edit there. A page change updates the site without a new version
(a release's fingerprint is the game's files, name, arguments and binary). The engine now reads another repository's Python at check time; if the
site's rules move, `page_problems` follows them with no change here. To undo: drop the `page` keys (the next sync removes those pages; restore them in
RedEngineGames by hand) and the `notes` step.
