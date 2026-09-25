# 0018. The self-describing engine pattern (a reusable design)
Status: accepted (rationale for ADR 0001 and 0007, written so another project can port it)

## Context
An AI working on a codebase spends most of its context window *finding* things: reading files to learn what
exists, what a command takes, which values are legal, why something is built as it is. Most of that reading is
wasted, and stale prose makes it worse. The goal here is the inverse: **the engine answers questions about itself,
in a bounded number of tokens, and cannot lie.**

## Decision: the pattern (each part is small; the value is in having all of them)
1. **One cheap entry point** — `red_engine2 describe` (about 50 lines, or `describe --brief --json` for ~1 KB):
   what exists, every command, the topics to drill into. It is the *only* thing an agent must read first.
2. **Topics on demand** — `describe objects|lint|physics|conventions|glossary|decisions`, each a bounded page.
   Depth is opt-in; ordinary tasks never load `SPEC.md`/`AGENTS.md` whole.
3. **Search across every knowledge kind at once** — `search "<question>"` ranks fragments of docs, ADRs, glossary,
   assets, lint codes, recipes, CLI commands *and Rust symbols*, returns only the best few with a `where to read
   more` pointer. (`src/tools/search.rs`: a plain scored index built on demand, never stale.)
4. **Catalogues with paste-ready snippets** — `catalog <name>` prints real measured size, parameters and a snippet
   that already validates. An agent copies instead of inventing.
5. **Known-good complete examples** — `recipe`: each one is linted and verified by a test, so it is always a safe
   starting point.
6. **Checks that return evidence, not opinions** — `lint`/`reach`/`walk`/`verify` run the *same* code the game runs
   (ADR 0003), print stable machine-readable codes and a fix, and exit non-zero. `verify` runs a scene's own
   `checks` block: "did I break anything?" is one command.
7. **Source navigation as a last resort** — `src map|find|show|refs|coverage` prints one item's signature and docs,
   bounded. Reading a whole file is the exception the tools are designed to avoid.
8. **Every editing command validates and refuses to write invalid data** — the failure arrives with the edit, with
   an `object.field: message` path and, where possible, a suggestion.
9. **Docs cannot drift (ADR 0007)** — anything a doc states that code can check *is* checked by a test: the command
   list is read from the real clap definition, object examples are parsed, lint codes are cross-checked, recipes
   are verified, ADR files must be registered, and the top-level entry point is generated from the live registry.
   A stale doc therefore fails CI instead of misleading the next agent.
10. **Machine-readable twins** — every human answer has a `--json` twin with a stable envelope
    (`{"ok", "schema", "data" | "error"}`), so an MCP layer is a thin pass-through, never a second implementation.

## Why it works
Cost is paid once, by the engine, not by every agent on every task. Truth has one source (the code or a test), so
answers are current. Bounded outputs make token cost predictable. And the loop "look it up, copy a snippet, edit,
validate, verify" needs no file reads at all for the common tasks.

## Porting checklist (e.g. to Blue Engine)
- Put every command behind one CLI parsed by a declarative library; generate `describe commands` from that definition.
- Write `search` over your docs + symbols before anything else (a 400-line scorer is enough).
- Give each error a stable code, a path to the offending data, and a suggested fix; return them as JSON too.
- For each prose claim, ask "can a test check this?" — if yes, generate or test it.
- Write down the canonical tasks an agent will do and measure how many tokens they take
  (`tests/ai_tasks.rs` here): treat growth in source reading as a regression.

## Consequences
Every new subsystem needs a describe topic, a search-indexable doc and a check — that is the cost. To undo any part,
delete the topic; nothing else depends on it.
