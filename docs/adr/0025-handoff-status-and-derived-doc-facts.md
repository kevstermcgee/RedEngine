# 0025. Handoff and honest docs: `status`, STATUS.md and derived facts
Status: accepted

## Context
Two costs recurred for an AI resuming work: (1) after its terminal closed, the only record of "done / in flight / failing / next"
was a 13 MB session transcript parsed with an ad-hoc script; (2) `CLAUDE.md` confidently said multiplayer "is not built" and
"95+ tests" when the repo had a secure server and 248 unit tests, which made the AI distrust correct code. ADR 0007 already
made *command* docs test-checked; prose facts about the repo were still free text.

## Decision
- `red_engine2 status` prints one screen to resume from: facts derived from the files (binaries, features, ADR/suite counts),
  git branch/commit/uncommitted files/recent commits, and the project's `STATUS.md`. `--init` creates the handoff file
  (sections: now, done, next, blocked, notes); `--note "..." --section next` appends a dated bullet.
- **Facts are derived, never typed.** A doc may hold a region between `<!-- facts:begin -->` and `<!-- facts:end -->`;
  `status --sync-docs CLAUDE.md` rewrites it from the repo, and `tests/docs_fresh.rs` fails when the committed text differs.
  The same test forbids hand-written test counts and "not built" claims in the AI-facing docs and checks that every `scripts/...`
  and `docs/...` path a doc mentions exists.
- `scripts/dev` / `dev.ps1` are the single entry point for humans and AIs: they find the toolchain, run from the repo root,
  set timeouts, and write full command output to `out/logs/` while printing only a summary, so a 300-test run costs a few
  lines of context instead of thousands.

## Consequences
Stale claims become test failures. The handoff is a convention with a tool, so it only works if agents record checkpoints;
`CLAUDE.md` in scaffolded projects says so. `status` needs `git` for the git section (it degrades to facts + STATUS.md without it).
