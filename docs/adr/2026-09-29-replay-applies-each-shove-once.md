# 2026-09-29. Replay applies each shove once: inputs and external pushes are recorded, strikes and rule impulses are re-derived
Status: accepted
Summary: A trace records inputs and external pushes only; strikes, bullets and rule impulses are outputs the replay re-derives, so it applies each shove exactly once.

## Context
ADR 0021 promised that a recorded match replays bit-exactly, and routed "every server-side push" through `MatchSim::apply_impulse` so the
recording would capture it. When the bat strike, hitscan hits (ADR 0022) and the rule `impulse` action (ADR 0020) were added, they were routed
through the same function. Two physics games built on the engine (Domino Halls, Knockdown Alley) found the consequence independently: any trace in
which a bat strikes a prop diverges at the strike tick (`DIVERGED ... first divergence at tick 6: props differ`), and any scenario with a rule
`impulse` diverges the tick after it fires, while walk-and-pickup traces and `replay --against` of two live runs agree. The live simulation was
deterministic; the replay was not. The cause: the strike is an *output* of a recorded input (and the rule impulse an output of the recorded rules),
so the replay re-derives it by re-running the same inputs and rules, and then also applied the recorded `Impulse` entry: the prop was shoved twice.

## Decision
- A trace records **causes**: joins, leaves, inputs, view lag, and **external pushes**, meaning a shove nothing else in the recording explains (the
  server's demo kick, an operator command). Those go through `MatchSim::apply_impulse`, which records an `Entry::Impulse` and applies it.
- A push the simulation **derives itself** from what is already recorded (the bat strike and hitscan hits in `sim/interact.rs`, the rule `impulse`
  effect in `MatchSim::run_rules`) goes through `MatchSim::shove`, which applies the impulse and records nothing. The replay reproduces it by
  re-running the inputs and the rules, exactly as the live simulation did.
- The contract, stated once here and in `describe sim` / SPEC: **a replay applies every shove exactly once**. `tests/sim_replay.rs` proves it with
  a committed fixture (`tests/fixtures/strike_replay.json`): a bat swing at a dormant crate, a rule `impulse`, and a server-style external push each
  record, positively move the prop (read from the trace dumps), and replay with every checksum matched. The first two failed before this change.

## Consequences
- The engine's headline determinism claim holds for combat and for rule-driven physics, which is what a physics game's `checks.sim` stands on.
- Live simulation results are unchanged (each shove was always applied once while recording), so no blessed trace needed re-recording and the
  protocol version is untouched; only the trace contents and the replay differ. A trace recorded before this change that contains a strike never
  replayed clean and still will not; re-record it.
- Anything new that pushes a prop must pick a side: derived from recorded state, use `shove`; decided outside the simulation, use `apply_impulse`.
  Recording a derived push again is the bug this ADR removes.
