# 0001. Maps are JSON; the engine describes itself so AIs never read Rust
Status: accepted

## Context
The maps and the tools around them are built mostly by AI agents. Every session that has to explore
`src/` to learn how something works burns context and still gets things subtly wrong (origins, lint
rules, physics numbers). Rust source is the most expensive, least structured way to learn the engine.

## Decision
- A map is one JSON scene (`SPEC.md`). Authoring never requires Rust.
- The CLI is the engine's documentation: `describe`, `search`, `catalog`, `recipe`, `src map|find|show`.
  An agent asks a question and gets the best few fragments, not files.
- The feedback loop is executable: `lint` → `plan`/`tour` (look) → `verify` (the scene's own `checks`).
- Entry points for an agent, cheapest first: `CLAUDE.md` → `describe` → `search` → `AGENTS.md`.

## Consequences
- New capabilities should ship with a CLI/describe surface, not just code. If an agent would need
  to read source to use a feature, the feature is unfinished.
- Adding an object type / lint code / command makes `cargo test` say which docs to update (ADR 0007).
- Rust stays free to change as long as the JSON language and CLI contracts hold.
