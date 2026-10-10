# 2026-10-06. A native MCP server
Status: accepted
Summary: red_engine2 mcp serves eleven typed tools over stdio, running the CLI's commands in process; mcp_server.py becomes a launcher

## Context
`mcp_server.py` was a 500-line Python wrapper with 36 tools. Every call spawned the CLI; every scene-taking tool (`validate_scene`, `lint_map`, `verify_map`, `plan_map`, ...) took the
scene as a **string argument**, so an agent working on a 10 KB map resent those 10 KB with every call; and the 36 tool definitions were loaded into the agent's context at the start of
every session. It also needed Python and the `mcp` package (`mcp>=1.20,<2`: the 2.x line renamed `FastMCP` and breaks it).
Measured with `scripts/mcp_bench.py` on one fixed nine-call task (orient, find a recipe, validate, lint, edit, validate, lint, verify, look at a plan, run the sim) on
`recipes/rooms_and_door.json`, median of 5 runs, same CLI binary behind both servers: **21,618 B of `tools/list`, 73,431 B sent, 9,462 B of text read (104,511 B), 1.07 s wall.**

## Decision
`red_engine2 mcp` is a stdio MCP server built on `rmcp`. Each tool's arguments are a typed struct (`serde` + `schemars`), so the schema cannot drift from the code. A call runs the same
command line the CLI would **in this process**, on a blocking thread with its output captured (`tools::envelope::begin_capture_text`, the capture `--json` already uses, now with the
text form kept): no spawn, no second implementation, and what an agent is told is what `red_engine2 <command>` prints. Eleven tools: `describe`, `search`, `context`, `validate`,
`lint`, `analyze` (reach, walk, ray, ls, info), `patch`, `verify`, `sim`, `view` (plan, frame or tour, returned as an image), and `run`, which reaches every other command. Files are
paths in the server's working directory: a scene is never sent as text. A non-zero exit is an error result. Commands that never return (`game serve|play|play-local`, `web serve`,
`portmap keep`, `mcp`) are refused with a message, not run. Standard output belongs to the protocol: the one library `println!` that could reach it (`patch --dry-run`) now goes through
the capture, and a test fails on any stdout line that is not a protocol message.

One cache, because it measured as worth it: the search index (docs, ADRs, assets, every public symbol) is built once per process and kept 30 s; building it cost 0.26 s per question and
a cached question costs about 0.09 s. A cache of loaded scenes by content hash was **not built**: parsing is cheap (`validate` of the 136 KB office map takes 50 ms; building its world, 0.11 s)
against 1.2 s for `lint`, so it could save about a tenth of a call at the cost of changing every analysis command's signature (`MapWorld` is neither `Clone` nor shared).

`mcp_server.py` is now a 45-line launcher that finds the executable (`scripts/red_resolve.py` when present, else the target directory) and becomes it, so existing client
configurations keep working with no Python packages; `requirements.txt` is gone.

## Consequences
Same task, same binary, median of 5 (`python3 scripts/mcp_bench.py --server "red_engine2 mcp" --task new --repeat 5`): **4,927 B of `tools/list` (-77%), 225 B sent (-99.7%), 4,857 B read
(-49%; the text form of `describe` and `search` instead of their JSON envelope, with identical findings in `lint`, `verify` and `sim`), 10,009 B in all (-90%), 0.37 s wall (-65%, mostly the
Python start-up; the calls themselves 0.46 -> 0.36 s).** The picture is the same 68 KB PNG either way. Budgets are tests: `tools/list` <= 5,200 B (`src/cli/mcp.rs`) and the whole
task <= 11,500 B (`tests/ai_tasks.rs`). The binary gains `rmcp` and `schemars` (also what published JSON Schemas will use). Not measured: a model using either server (a fresh-agent trial
needs a model attached to each; the benchmark prices the tool surface, not an agent's skill), Windows (the integration tests run in hosted CI), and clients other than the scripted one.
Undo: restore the previous `mcp_server.py` from git history; nothing else depends on it.
