# 2026-10-09. The front-door documents are a pointer, one entry point and checked claims
Status: accepted
Summary: README.md is a short pointer page, AGENTS.md is the single entry point, and the claims a reader acts on (protocol version, missing features, file sizes, MCP tools, cargo features) are checked against the repository.

## Context
A review on 2026-10-09 found the entry documents wrong in ways that cost a fresh agent time: the README gave the wire protocol as v5 (the code has 15), listed
lag compensation as missing (it is built, ADR 0053), called CLAUDE.md 4 KB (it was 8 KB), told the reader to read SPEC.md "once" (63 KB; AGENTS.md says never open it
whole), offered an MCP `start` tool that does not exist, and showed `red_engine2 render` without the cargo feature the default build lacks. Following the documents
literally cost about 124 KB of reading before the first useful command (README 28 KB, AGENTS 14 KB, CLAUDE 8 KB, SPEC 63 KB), most of it the same ladder written twice.
ADR 0007/0025 already keep derived numbers current and ban test counts; these claims were prose about the code, which nothing compared with the code.

## Decision
- `README.md` is a pointer page of about 3 KB: what Red is, the first command, a table of where to go next. The long form moved to `docs/ENGINE_OVERVIEW.md`; the
  prop hunt viewer era to `docs/VIEWER_HISTORY.md`.
- `AGENTS.md` is the single entry point and holds the workflow and the cheap-by-default ladder once. `CLAUDE.md` (what Claude Code loads first) is a pointer plus the
  derived facts block.
- `src/tools/doc_claims.rs` compares the claims a reader acts on with the repository, in `red_engine2 preflight` and `tests/docs_fresh.rs`: a protocol version written
  as a plain number (write `<!--fact:protocol-->N<!--/fact-->`), a feature the docs call missing while its code is in the tree, a size written next to a document's name,
  an MCP tool or tool count that `src/cli/mcp.rs` does not have, a `render` command shown without `--features video`. `<!--fact:mcp-tools-->` and `<!--fact:brief-kb-->` join
  `protocol` as derived inline facts.
- There is no MCP `start` tool: `start` is the Python launchpad (`scripts/launchpad.py`), the server is a Rust process with no Python, and every tool added costs every
  session's context (`tools/list` has a byte budget). An MCP client runs `scripts/dev start` from the shell.

## Consequences
A new claim of a kind the checks know cannot drift silently; a new kind needs a new check (add it to `doc_claims.rs` with the sentence that showed it). The README no longer
teaches anything: a reader who wanted the design goes one link further. Undo: restore the old README from git history; the checks stay valid for any document in `CLAIM_DOCS`.
