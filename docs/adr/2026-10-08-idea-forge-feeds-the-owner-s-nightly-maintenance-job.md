# 2026-10-08. Idea Forge feeds the owner's nightly maintenance job rather than running its own improver
Status: accepted
Summary: nightly writes the ranked backlog to a *feedback*.md note the existing 23:00 redengine-nightly job already watches; this tool's own improvement agent is opt-in; the schedule is a systemd user timer like the machine's other jobs.

## Context
The nightly loop (ADR "Idea Forge closes the loop") included its own engine-improvement agent. The machine already runs one: `redengine-nightly.timer` (23:00) starts `~/.local/bin/redengine-nightly`, which gates on new feedback (GitHub issues and comments, `.gauntlet/feedback.md`, any `*feedback*.md` note changed under `~/workspace`), runs Claude headless on a branch, re-runs the full CI itself and pushes to `main` only if green. It fixed `teleport` the night before this decision. That job never saw Idea Forge's findings (they sit deeper than its search depth and are not named `*feedback*`), and a second agent editing the same engine is duplication with a conflict risk.

## Decision
- `nightly` exports the open backlog to `../idea-forge-feedback.md` (`export_feedback_note`): a `*feedback*.md` file directly in the workspace, which that job's existing gate and prompt already pick up. It is rewritten only when its content changed. No change to the maintenance job's script or prompt is needed or made.
- The file says how a fix is recorded (`fixes.json`), so the backlog can mark items addressed or `partial`, and flag `recurring` ones.
- This tool's own improvement agent stays (`improve`, `nightly --improve`) but is **off by default**.
- The schedule is a **systemd user timer** (`schedule --backend auto|systemd|cron`), the machine's convention: oneshot, `Nice=10`, best-effort IO priority 7, `TimeoutStartSec=10h`, PATH baked in. A night is skipped below 20 GB free disk (the maintenance job's own guard).

## Consequences
- The owner's maintenance job decides what to fix and pushes only after its own full CI; Idea Forge's role is to produce evidence, deliver it reliably and keep the ranking honest.
- The file is evidence, not instructions: the maintenance prompt already says to treat feedback that way.
- Closing the status loop (`addressed`) depends on that agent adding a `fixes.json` entry. If it does not, the backlog keeps listing a fixed item, and the maintenance job is told to check main first (its prompt: "if already addressed on main, say so and do nothing").
- Undo: `schedule --uninstall`; delete the exported note.
