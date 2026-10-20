---
name: engine-feedback
description: Record what RedEngine, its app or its site cost this session (missing features, confusing errors, slow steps, things that worked) as keyed findings the daily improver ranks and fixes. Use at the end of any session that made, updated or forked a game, or when asked to give engine feedback.
---

# Leave engine feedback

The engine improves every night from findings like these (docs/DAILY_IMPROVER.md). Findings that name a real cost, with evidence, get
fixed; vague ones get skipped.

1. **Look back** at what this session hit:
   - a capability the game needed that the engine lacks;
   - an error that did not say how to fix it;
   - a command that was slow, missing or wrong;
   - a document that misled you;
   - an app or site problem;
   - and what worked well.
   Count the minutes each one cost.
2. **Reuse known keys.** `python3 scripts/feedback.py digest --limit 200 --offline` lists the open problems with their `key`. When yours is
   one of them, use the same key: that is what ranks it.
3. **Add each finding** (it writes `docs/feedback/<date>-<login>-<topic>.md` and checks it):
   ```sh
   python3 scripts/feedback.py add "<one-line title>" --key <kebab-key> --area <area> --severity <1-3> --minutes <n> \
     --evidence "<the command and the message, or the number>" --workaround "<what you did instead, or none>" \
     --proposal "<the change you want in the engine, app or site>" --topic <game-slug> --context "<what you were doing>"
   ```
   - Areas: discovery, docs, cli, diagnostics, format, runtime, verify, publish, perf, tooling, ui, network, idea-fit.
   - Severity: 1 = papercut, 2 = cost real time, 3 = blocked the work.
   - Use `--about launcher`, `--about site` or `--about game:<slug>` when the finding is not about the engine.
   - Use `--by <login>` when `gh` is signed in as someone else.
   - Use `--person` when the person, not you, is the source.
4. **Commit the note** with the work, in the same pull request. `python3 scripts/feedback.py check` must pass; CI runs it.

Without a checkout (in the RedEngine Games app), use the **Engine feedback** tab instead. With a publishing token saved there, send it
**signed**: it becomes a GitHub issue that the inbox trusts.
