# 2026-10-20. RedEngine has two equal maintainers and one loop
Status: accepted
Summary: The owner and meastwood13 are equal maintainers (.github/contributors.json maintainers): each merges their own safe game, the other reviews engine, app and site changes, both publish from the app, and their notes are trusted feedback; Claude Code skills give every fresh session the same route from idea to published game, fork and feedback

## Context
On 2026-10-20 the owner asked that their cousin (meastwood13) have the same access to publish, update, review and edit games, and that his
and his AI's feedback count as much as theirs: "this is becoming a collaborative project". Either of them should be able to prompt a fresh
AI session to make a game, publish it to the launcher, give feedback through the launcher and send updated versions. The tooling assumed
one owner instead. ADR 2026-10-20-a-game-and-its-page-are-sent-in-one-pull-request has "the owner taps Merge": the contributed-games check
mentioned the owner and said "only the owner edits" the contributor list. The app said "only the site's owner can publish". Feedback
counted the owner's word and treated everything else as anonymous. meastwood13 already had write access to RedEngine, but not to
RedEngineGames.

## Decision
- `.github/contributors.json` lists **`maintainers`** (`kevstermcgee`, `meastwood13`), who are equal, and `contributors` (none yet), whose
  safe game pull requests wait for a maintainer. The maintainers change the list together.
- **Merging** (docs/COLLABORATING.md):
  - A maintainer merges their own game's pull request once `scripts/contributed_games.py` comments **Safe to merge**. The comment now tells
    the author to merge it and mentions the other maintainer. It no longer waits for the owner's tap.
  - Engine, app and site changes are merged by the other maintainer after a look. The daily improver's merge on green CI, under its own
    rules.
  - A maintainer's pull request that changes none of their games gets no comment: it is engine work, reviewed as such.
  - A maintainer's own feedback note may ride with their game and stay safe.
- **Ownership of games** is the playable's `"maintainer"`. Nobody (the daily improver included) changes a maintained game without its
  maintainer: they fork it or send a pull request that the maintainer merges.
- **Feedback** from either maintainer is trusted (`scripts/feedback.py`): their issues, their pull requests' engine notes, their
  `docs/feedback/` notes, and the app's signed notes (RedEngineGames 0.5.0).
- **Fresh sessions** get the route as Claude Code project skills in `.claude/skills/`: `make-game`, `update-game`, `fork-game`,
  `engine-feedback` and `inbox`. Each is short and points at the page that holds the rules (CONTRIBUTING.md, GAMES_PUBLISHING.md,
  MAKING_GAMES.md, feedback/README.md), so the rules live in one place.
- What only the owner can grant stays with the owner: collaborator access to RedEngineGames, repository settings, secrets and code
  signing.

## Consequences
A game goes from idea to the site with no one but its author involved, once CI passes. That is the same trust the write access already gave.
Engine work gets a second pair of eyes from the other maintainer instead of waiting for one person. The skills make an AI session on either
machine follow the same route; on Windows they say where the CLI is missing (key `no-windows-cli-prebuilt`). To undo: move a name from
`maintainers` to `contributors` (their safe pull requests then wait for a tap), or delete the skills.
