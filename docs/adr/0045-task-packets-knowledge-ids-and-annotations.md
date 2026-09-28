# 0045. Task packets, knowledge IDs and source annotations
Status: accepted

## Context
The route an agent should take (describe -> search -> context -> edit -> affected -> stop) existed, but measuring it on 20 plain-language
engine tasks showed the middle step failing: `context "add replicated doors"` (the quoted form docs suggest) answered 0 of 20; unquoted
words routed 13 of 20 to the right feature, stop words such as "add"/"new" steering several, at 9.2 KB of API listings per packet.
Traps agents actually hit (spawn fallback to the camera, f32 tick counts, lint not seeing rule-driven collision, unregistered ADRs,
gfx modules leaking into the headless build) lived in scattered prose or nowhere, and failing guard tests did not say where the fix was.

## Decision
- `context "<task>"` routes plain words (stop words dropped, light stemming, `search`'s synonyms, rarity weighting, module purposes and
  public symbols normalised by feature size) and prints a **task packet** (`tools::task`, default 5 KB): owners with a confidence and why,
  files and symbols to read first, `AI-*` annotations in them, traps/decisions, canonical examples, ADR pointers, the owners' tests and
  `affected` commands, dependents, features with no dependency path ("probably not needed", advisory), expected scope, and when to stop.
  Feature names and files still get the full feature packet; `--full` gives it for a task; `--json` is smaller than the text.
- `docs/KNOWLEDGE.md` (`tools::knowledge`): traps, check codes and rejected approaches, one ID each, owned by features, validated by a test
  (fields, owners exist, ADRs exist), found by `search` and `context <ID>`. Guard tests now print their ID (`HEADLESS-001`, `SIM-001`,
  `FEAT-001`, `DOCS-001`, `DOCS-002`). Entries are retired when a tool makes the mistake impossible.
- A fixed annotation vocabulary (`// AI-INVARIANT|BOUNDARY|WARNING|HOTPATH|COMPAT|SECURITY|DEPRECATED|CANONICAL: ...`) read by the existing
  symbol scanner; used sparingly (nine sites) where a rule is enforced or expensive to rediscover.
- `features.json` gains `canonical` examples; `features --check` requires each to exist and be run by an indexed suite (or be a recipe or
  example map, which CI loads).
- `AGENTS.md` opens with the route, the completion contract and scope expectations; `tests/ai_tasks.rs` keeps routing at >= 16/20 first and
  19/20 listed, every packet <= 5 KB.

## Consequences
The measured route costs one 2-5 KB call instead of guesswork or 9-12 KB listings. Routing is lexical: a task phrased in words no feature
uses still routes poorly (reported as low confidence, with the escalation to `affected` without `--quick`). No agent-level metrics (tokens,
tool calls, files opened) are measured by the repository itself; Gauntlet rounds are where those belong. Undo: the task path is one module
behind `context`; the feature path is unchanged.
