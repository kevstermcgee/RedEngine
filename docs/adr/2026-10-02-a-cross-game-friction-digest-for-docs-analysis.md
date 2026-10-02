# 2026-10-02. A cross-game friction digest for docs/analysis
Status: accepted
Summary: analysis digest groups every analysis note's feedback by the engine feature it matches, ranked by how many different games raised it, regenerating docs/analysis/README.md the same way the ADR index is generated.

## Context
The engine already has a working loop for turning one game's build experience into engine fixes: a builder (human
or AI) writes a report, `analysis new --from <report>` saves it as a dated note under `docs/analysis/`, and a
person or agent reads it and writes ADRs for whatever gets adopted. This happened nine times before this decision
(Cheddar, Trigger Happy, Great Outdoors, three physics games, ten minigames comparison, etc.) and worked — it is
the same pattern Killchain's `ENGINE_FEEDBACK.md` used this session, leading to three real engine fixes.

What it cannot do: show that the same friction recurs *across* games. Each note only tells the next builder what
one game's build taught us; nothing reads more than one at a time. The one time several games' feedback was
merged and ranked by hand (the three-physics-games note), that merging happened in a scratch file outside the
repo, not through any engine command — so the result was never checked in, never regenerated, and could not be
redone cheaply as the eighth and ninth notes arrived. `analysis::list()` already excluded a file literally named
`README.md` from being parsed as a note (`src/tools/analysis.rs`), which is a strong signal this gap was
anticipated but never filled.

The broader motivation: RedEngine should get measurably better at being used by AI agents as more games are built
on it, and agents should spend less time reinventing something that already exists. A digest that surfaces
recurring friction, grouped by the engine feature it is about, serves both — it is the natural place to look
before hand-writing something that feels like it should already exist, and it turns "this keeps coming up" from a
hunch into a ranked, regenerable fact.

## Decision
Add `red_engine2 analysis digest [--write]`. It reads every note `analysis::list()` finds, extracts each note's
feedback rows (its `| # | feedback | status | what now happens |`-shaped markdown table, where one exists — the
parser is lenient about column names, since existing notes vary slightly; a note with no table falls back to one
row built from its title and summary, so nothing is silently dropped), and matches each row's text to the engine
feature it is about using `context::resolve` (`src/tools/context.rs`) — the same word-overlap scoring `context`
already uses to answer "what do I need to know to change this?" No new text-matching algorithm was written.
Short words (under 4 characters) are filtered out of a row's text before matching, because `context::resolve`
matches by substring and a short word like "a" or "its" would otherwise swamp the signal by matching almost any
feature's text.

Rows are grouped by feature and ranked by how many *distinct* notes raised something about it, not raw row count,
so one chatty note cannot dominate the ranking. Rows matching no feature land in an "unassigned" bucket, listed
last, never dropped.

The result is written to `docs/analysis/README.md`, generated exactly the way `docs/adr/README.md`'s index is:
between `<!-- analysis-index:begin/end -->` markers, created from a small template the first time, any
hand-written preamble above the markers preserved on regeneration. `red_engine2 preflight` gained a matching
staleness check (`Fix::AnalysisDigest`), modeled directly on the existing ADR-index check, so the digest cannot
silently rot the way the scratch-file merge did.

`AGENTS.md` gained two sentences (not a hard rule like ADRs — `analysis new` stays as informal as it always was):
check `search` and `docs/analysis/README.md` before hand-writing something that feels like it should already
exist, and write a note after finishing a game build or a real patch session.

## Consequences
Recurring friction across games is now a ranked, checked-in fact instead of something only noticed if someone
happens to reread several old notes. The feature-matching is a first-pass heuristic (keyword/substring overlap,
not semantic understanding) and visibly imperfect on the nine existing notes — some rows about networking land
under `bots` because of incidental word overlap — but it is cheap, transparent, consistent with how `search` and
`context` already work in this engine, and good enough to be useful without inventing a second matching system.
It gets better as more notes accumulate (more distinct-note overlap sharpens the ranking) and can be revisited if
the false-positive rate turns out to matter in practice.

This is additive: the nine existing notes needed no changes, nothing enforces that a note has a table, and the
digest itself is excluded from being re-ingested as a note (true today because `list()` already filters out
`README.md` by name). Undoing this means deleting `docs/analysis/README.md`, the `digest` subcommand, and the
preflight check — no existing note's format or the `analysis new`/`list` commands depend on it.
