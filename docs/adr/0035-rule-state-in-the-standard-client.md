# 0035. Rule state in the standard single-player client
Status: accepted

## Context
Scenes could declare and headlessly prove complete gameplay with `vars`, `rules` and `checks.sim`, while the normal `re2` client
did not run that rule state machine. `hide` only changed headless state, variables and events had no visible presentation, and an
outcome did not appear to the player. A game author therefore had to duplicate otherwise-valid gameplay in a second application
layer merely to present it.

## Decision
- Offline `re2` owns a `RulesEngine` created from the parsed scene and steps it at the client's existing fixed 60 Hz tick. The
  player's real body feeds rule volumes; pickup, drop, shot and hit feed the already-defined engine events.
- Generic effects are applied at the client boundary: teleport moves the local player and impulse addresses an existing loose prop.
  The pure rules module stays free of window, renderer and physics dependencies.
- `LiveRenderer::set_hidden_objects` accepts object ids and suppresses their complete mesh ancestry in both colour and shadow
  passes. It is a small reusable application surface for custom clients as well as the built-in one; authored transforms and
  collision are not mutated.
- The existing audited UI kit renders a compact, genre-neutral rules HUD: scene-defined variables, the newest event for two
  seconds, and the terminal outcome. No new scene schema or benchmark-specific HUD declaration is introduced.
- Protocol v3 was unchanged by this decision. ADR 0036 later added online replication and reused this same generic HUD.

## Consequences
A scene game proven with `sim` now has a direct visible single-player path without a parallel gameplay implementation. The local
client still has older weapon/prop orchestration outside `MatchSim`; moving all of that onto a local `MatchSim` would be a larger
consolidation and is deliberately not hidden inside this change. A future network protocol can carry a bounded rule-state delta
and feed the same renderer/UI surfaces without changing rule semantics.
