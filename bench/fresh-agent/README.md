# Fresh-agent benchmark: can a model that has never seen RedEngine make, prove and modify a native 2D game?

The point is not whether a strong model can do it with enough reading. It is **where a weaker model needs help** (documentation, source exploration, retries, manual repair) so that the
engine's commands, diagnostics, schemas and templates can remove that need, instead of adding more prose.

* `TASK.md` is the whole prompt. It prescribes only the interface a scorer needs (scene id `p`, variable `timeleft`, file names).
* `scripts/agent_bench.py score DIR` scores the end state, from the engine's own machine-readable answers (`verify`, the scenarios in the game file).
  `--before before.game2d.json` checks the requested change.
* `RED_TRACE=trace.jsonl` makes every `red_engine2` command append one line (`src/tools/agent_trace.rs`). `scripts/agent_bench.py summary trace.jsonl` turns that into the friction report:
  which `describe` topics and `search` questions were the documentation the agent needed, how many commands failed and with what first error line, retries, repair cycles, and CLI source exploration.
  Reading the engine's files directly is invisible to the CLI: add `--transcript session.jsonl` (a Claude Code transcript) to count Read/Grep/Glob/shell reads of the engine's source and docs.
* `scripts/agent_bench.py reference WORKDIR` is a deterministic stand-in agent that follows only what `describe 2d` says. It proves the benchmark is passable with zero source exploration and
  zero retries, and it is run by `tests/fresh_agent_bench.rs`.

## Running it for real (spends model tokens: ask first)

```bash
mkdir /tmp/bench && cd /tmp/bench && export RED_ENGINE=/path/to/red_engine2 RED_TRACE=/tmp/bench/trace.jsonl
claude -p "$(cat /path/to/RedEngine/bench/fresh-agent/TASK.md)" --allowedTools "Bash Read Edit Write" --output-format stream-json > session.jsonl
python3 /path/to/RedEngine/scripts/agent_bench.py score star-dash --before before.game2d.json
python3 /path/to/RedEngine/scripts/agent_bench.py summary trace.jsonl --transcript session.jsonl
```

Record each live run in `docs/analysis/` with: model, date, engine revision, the scorecard, the friction report, and what you changed in the commands or templates because of it.
No live run has been recorded yet (see the analysis note of this milestone for what the reference run measured and which friction points were already removed).
