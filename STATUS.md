# STATUS — red_engine2

_Handoff file for whoever (human or AI) resumes this work. Keep it short and current: update it at every checkpoint with
`red_engine2 status --note "what changed" --section done|now|next|blocked|notes`._

## Now (in flight)
- 2026-10-07: foundation milestone: 2D effects (named action lists, apply/with) + medic-run + shared-effect pattern + rule-fire diagnostics committed on branch foundation; affected green (994s); full CI + PR pending; next: effects for scenario scripts/UI, cargo-mutants, schemas

## Done

## Next
- 2026-10-07: DIRECTION CHANGE (user): no web play for any game; Windows EXE downloads only, remove web from the site. Plan: (1) re2d native 2D player DONE on branch native-2d (Xvfb-verified draw+tick; input/audio untested); (2) package2d -> EXE zip/installer + make 2d on windows SUPPORTED in caps; (3) switch publish2d/site to EXE download pages (RedEngineGames), remove web build/serve/verify/publish, wasm, CI web stage, docs; (4) replace browser checks in scenarios

## Failing / blocked

## Decisions & gotchas
