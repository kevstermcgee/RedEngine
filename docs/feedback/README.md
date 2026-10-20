# Feedback notes

What a maintainer's AI session (or a person) learned about RedEngine, the RedEngine Games app or the site while making, updating, forking or playing a
game: one note per topic and day, `docs/feedback/<YYYY-MM-DD>-<GitHub login>[-topic].md`, committed with the work it came from (the game's pull request
is fine: `scripts/contributed_games.py` counts your own note as part of a safe game change).

Every note holds the Idea Forge's findings block, so a problem met by anyone ranks as **one** entry, by its `key`, with every report of it: the Idea
Forge's morning runs, your AI's notes and your cousin's AI's notes alike. `python3 scripts/feedback.py digest` is the ranked inbox of everything (these
notes, the Idea Forge's runs, maintainers' issues, merged pull requests' "Engine notes", players' reviews and engine notes); the daily improver reads it
first ([`docs/DAILY_IMPROVER.md`](../DAILY_IMPROVER.md)). On a machine that runs the Idea Forge, the digest also has a section **Idea Forge games merged but not
published**: the games `publish` could not publish (with the reason and since when) and the runs that built and were never shipped for more than two days (with what
`ship` would refuse), read from the forge's ledger ([`docs/IDEA_FORGE.md`](../IDEA_FORGE.md), "Nothing waits in silence"). A note or run file whose findings block is
malformed is never ranked, and never invisible: `digest` names it on a `SKIPPED` line, and `feedback.py check` does too for the Idea Forge's run files.

## Write one

The quick way, one finding at a time (it makes the note, numbers the finding and checks it):

```sh
python3 scripts/feedback.py add "A rule cannot play a one-shot sound" --key no-rule-triggered-one-shot-sound --area runtime --minutes 4 \
  --evidence "searched 'sound on event': only audio.duck exists" --workaround "a looping layer faded by a var" \
  --proposal "a rule action {sound: name}" --topic snake --context "Snake v2"
```

Before you invent a `key`, look for the same problem in `python3 scripts/feedback.py digest --limit 200` and reuse its key: that is what ranks it.
`python3 scripts/feedback.py check` holds every note to the format (CI runs it).

## The format

~~~markdown
# Feedback: <topic> (<date>)

- By: <GitHub login> (AI session|person)
- About: engine | launcher | site | game:<slug>     the default for findings without their own "about"
- Context: what you were doing (the game, its version, the pull request)

## Findings

```json findings
[{"id": "F1", "key": "no-rule-triggered-one-shot-sound", "area": "runtime", "title": "...", "severity": 2, "cost_min": 4,
  "evidence": "...", "workaround": "...", "proposal": "...", "about": "engine"}]
```
~~~

| Field | What it says |
|---|---|
| `key` | the problem's stable kebab-case name; the same problem keeps the same key everywhere |
| `area` | discovery, docs, cli, diagnostics, format, runtime, verify, publish, perf, tooling, ui, network, idea-fit (`worked` for what went well) |
| `severity` | 1 = papercut, 2 = cost real time, 3 = blocked the work or would block most people |
| `cost_min` | minutes it cost |
| `evidence` | the command and the message, or the number: something the next person can check |
| `workaround` | what you did instead (or `none`) |
| `proposal` | the change you want in the engine, the app or the site; not the workaround |
| `about` | optional: engine (default), launcher, site or `game:<slug>` |

## What happens next

The daily improver ranks the open keys and fixes what it can verify; a fix appends the key to `docs/analysis/idea-forge/fixes.json`
(`{"key", "date", "summary"}`, with `"partial": true, "remaining": "..."` if only part is done). The key then leaves the inbox, and comes back marked
`recurring` if anyone reports it after the fix. A `game:<slug>` finding goes to that game's maintainer, never into their game unasked.
