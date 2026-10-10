# 2026-10-09. preflight warns when the handoff file has fallen behind
Status: accepted
Summary: scripts/dev preflight prints a warning, without failing, when STATUS.md is more than 20 commits behind main, because an out-of-date handoff file is believed.

## Context
STATUS.md is the file the next person (or AI) reads to learn what is done, in flight and blocked (ADR 0025). On 2026-10-09 it had last been edited on October 6, 59 commits earlier: its "Done"
and "Failing / blocked" sections were empty, it listed a finished stage (the browser removal) as open and said "Nothing pushed yet" about work that had merged. Nothing noticed, because the
file is prose: ADR 0025 keeps derived facts current (`<!--fact:...-->`) but cannot know what a note should say.

## Decision
`red_engine2 preflight` (and so `scripts/dev preflight`) counts the commits on the main line (`origin/main`, else `main`, else `HEAD`) that came after the commit which last changed STATUS.md
(`tools::status::status_age`). More than `STATUS_STALE_COMMITS` (20) earns a warning that names the count and the one command that updates the file. It is a **warning, not a problem**: the run
still passes and `--json` keeps `ok: true` with a `warnings` array, because failing every contributor's commit for a file one person owns would teach people to touch the file to silence
it. An uncommitted edit to STATUS.md (someone is updating it now), a project without git or without a committed STATUS.md, say nothing.

## Consequences
A handoff file that stops being maintained is visible at the next `preflight`, which every commit runs. The count is of commits, not days, so a quiet week is not stale and a busy day is. 20 is a
judgement: the review's file was 59 behind. Undo: drop `status_warning` from `preflight::run`.
