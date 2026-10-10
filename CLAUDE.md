# Red Engine 2

**Read [`AGENTS.md`](AGENTS.md): it is the single entry point** (the workflow, the cheap-by-default ladder, the tools). This file only holds what Claude Code loads first and the facts derived from the code.

- First command: `scripts/dev start "<your task>"` (in a game project `scripts/red start "<task>"`); then `scripts/dev red describe --brief` and `scripts/dev red search "<question>"`.
- You never read engine source and never guess: the engine describes itself, checks your work and explains its failures. Do not open `SPEC.md` or `docs/AGENT_REFERENCE.md` whole and do not `@`-import either (10k+ tokens every session): use `search`.
- Changing Rust: `scripts/dev red context <feature|file|words>` first, then `scripts/dev iterate` while you edit, `scripts/dev affected` before you say done, `scripts/dev affected --full` before pushing, `scripts/dev preflight` before each commit.

## Facts (derived from the repo: `red_engine2 status --sync-docs CLAUDE.md` rewrites this block; a test fails if it is stale)
<!-- facts:begin -->
- Crate `red_engine2`; binaries: `re2`, `re2d`, `red_bot`, `red_engine2`, `red_relay`, `red_server`.
- Cargo features: `default`, `video`, `mcp`, `render`, `gfx`.
- Wire protocol v15 (`src/net/protocol.rs`): a client and a server must be built from the same version. Counts (suites, maps, ADRs) and test totals: `red_engine2 status`, `scripts/dev test`.
<!-- facts:end -->
