# Feedback: collaboration-loop (2026-10-20)

- By: kevstermcgee (AI session)
- About: engine
- Context: this session: mapping the collaboration loop (feedback, publishing, forks, the nightly) for the two maintainers

Findings are ranked with everyone else's by `key` (scripts/feedback.py digest). `severity` 1 = papercut, 2 = cost real time, 3 = blocked the work or
would block most people; `proposal` is the change you want (engine, app or site), not the workaround.

## Findings

```json findings
[
 {
  "id": "F1",
  "key": "no-windows-cli-prebuilt",
  "area": "tooling",
  "title": "A Windows maintainer's AI cannot run validate, verify or lint: the CLI is prebuilt for Linux only",
  "severity": 3,
  "cost_min": 0.0,
  "evidence": "RedEngine#201 (Snake, by meastwood13 on Windows): 'Please run (couldn't on Windows) ... none of these ran: validate / verify, the 6 checks.scenarios, publish check'; .github/workflows/release.yml builds only linux-x86_64",
  "workaround": "CI ran the checks after each push",
  "proposal": "release.yml also builds the headless red_engine2.exe for Windows into the same prebuilt release, and scripts/dev start fetches it in Git Bash"
 },
 {
  "id": "F2",
  "key": "nightly-work-stranded-on-branches",
  "area": "tooling",
  "title": "The nightly's work is stranded when main moves or its CI fails",
  "severity": 2,
  "cost_min": 0.0,
  "evidence": "~/nightly-reports 2026-10-10..19: 2 of 10 runs landed on main, 5 had nothing to do, 4 left origin/nightly/{2026-10-08,10-11,10-16,10-19} (6 commits) unmerged and unreviewed",
  "workaround": "none",
  "proposal": "land each night's work as a pull request that merges itself when hosted CI is green, rebased when main moved; one that fails stays open for a maintainer with the reason"
 },
 {
  "id": "F3",
  "key": "backlog-fixes-not-recorded",
  "area": "tooling",
  "title": "The keyed backlog closes almost nothing: 6 fixes recorded against 132 keys",
  "severity": 2,
  "cost_min": 0.0,
  "evidence": "feedback.py digest 2026-10-20: 126 open, 4 recurring, 2 partial, 0 addressed; e.g. no-per-entity-timers is open although #182 added entity timers",
  "workaround": "none",
  "proposal": "every daily run checks the two oldest open keys against main and records a fix (or partial) in fixes.json, with the commit that did it"
 },
 {
  "id": "F4",
  "key": "app-feedback-is-unsigned",
  "area": "ui",
  "title": "A maintainer's note in the app is anonymous, so it weighs no more than a stranger's",
  "severity": 2,
  "cost_min": 0.0,
  "evidence": "docs/player-reviews/reviews.jsonl mNzqfd4xW0Ds: Killchain 6/10 signed 'Kevin' ('multiplayer with my cousin not working is an absolute dealbreaker'): the name is free text, anyone could type it",
  "workaround": "none",
  "proposal": "with a GitHub token saved, the app sends a note or review as a GitHub issue by that account (trusted, threaded, closed by the fix); without one, it stays anonymous",
  "about": "launcher"
 }
]
```
