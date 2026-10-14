# Red Engine 2

**Read [`AGENTS.md`](AGENTS.md): it is the single entry point** (the first commands, then one page: `docs/MAKING_GAMES.md` to make a game or a map, `docs/ENGINE_WORK.md` to change the engine's Rust). This file only holds what Claude Code loads first and the facts derived from the code.

- First command: `scripts/dev start "<your task>"` (in a game project `scripts/red start "<task>"`); then `scripts/dev red describe --brief` and `scripts/dev red search "<question>"`.
- You never read engine source and never guess: the engine describes itself, checks your work and explains its failures. Do not open `SPEC.md` or `docs/AGENT_REFERENCE.md` whole and do not `@`-import either (10k+ tokens every session): use `search`.
- Changing Rust: `scripts/dev red context <feature|file|words>` first, then `scripts/dev iterate` while you edit, `scripts/dev affected` before you say done, `scripts/dev affected --full` before pushing, `scripts/dev preflight` before each commit.

- Content formats (scenes, prefab libraries, 2D games, blueprints, playscripts, scores, voices, `game.json`, ...) each have a typed definition in `src/content/` beside their hand-written checker; `tests/content_drift.rs` fails when the two disagree. Their JSON Schemas are in `schemas/` (generated: `cargo run -- schema`; a test fails when stale). `cargo run -- validate` checks every content file in the checkout (`validate DIR|FILES...` for some), `file:line: message`, in under a second; CI runs it as its fast tier. **A new content type or key needs a typed definition, its schema (`content::json_schema::all`), a `detect` rule in `content::validate` and a place in the drift test** (ADRs 2026-10-11-content-formats-are-typed-and-checked-twice, 2026-10-11-one-validate-for-every-content-file; `docs/content-formats.md`).

## Facts (derived from the repo: `red_engine2 status --sync-docs CLAUDE.md` rewrites this block; a test fails if it is stale)
<!-- facts:begin -->
- Crate `red_engine2`; binaries: `re2`, `re2d`, `red_bot`, `red_engine2`, `red_relay`, `red_server`.
- Cargo features: `default`, `video`, `mcp`, `schema`, `render`, `gfx`.
- Wire protocol v16 (`crates/red_core/src/net/protocol.rs`): a client and a server must be built from the same version. Counts (suites, maps, ADRs) and test totals: `red_engine2 status`, `scripts/dev test`.
<!-- facts:end -->
