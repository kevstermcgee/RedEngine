# 2026-09-30. Evidence-driven refinements from the external architecture study
Status: accepted
Summary: A grid index for ground lookups (reach spent 92% of its time there), a total order for lint findings, an in-memory virtual-clock network, sim::env with line-of-sight observations, a property test with shrinking for prop ownership; sccache rejected; the rest kept as reference or experiments.

## Context
Eleven projects were read for one technique each (`docs/analysis/external-architecture-study.md`, which also says what was and was not read). The rule was to adapt an idea only where
RedEngine has a measured problem. Four things were measured to be real: `reach` (and so `lint`, `plan`, `walk`, `verify`) cost 7-10 s on the big legacy maps, almost all of it one linear scan
(`ground_height_at` over every box top: 7.45 of 8.06 s on `office.json`, 2 849 tops, 2.96 M calls) and slower in the release build than in dev, so algorithmic; `lint` printed same-code findings in a
different order on different runs of one binary; 88% of test run time is the serial real-time network suites, although the game logic already takes explicit time behind transport traits; and there was no
way for a bot or a test to play a match that could not also read hidden state.

## Decision
- **`GroundCandidates` keeps a lazily built uniform-grid index over its box tops** (`BoxIndex`, dropped by `append`); `ground_height_at` reads the bin of the point. Same answer by construction (an order-independent maximum
  over the tops that contain the point), proven by a randomized test against the old full scan and by byte-identical output of 80 command runs over the ten examples. office `reach` 7.85 -> 0.54 s; `maps_verify` 11.7 -> 1.6 s.
- **`lint` sorts by severity, code, then message**, a total order, with a test that fails without the tie-break.
- **`net::memnet`**: an in-memory `ServerTransport`/`ClientTransport` pair with a virtual clock and a seeded link (loss, delay, jitter that reorders, partition of one peer), and `NetClient::with_transport`.
  `tests/net_virtual.rs` replays the lobby-to-rematch flow (5.27 s over UDP -> about 0.03 s), a lossy link, a vanished peer and determinism. The real-UDP, QUIC and process suites stay.
- **`sim::env`** (`reset/observe/step/terminated/truncated/debug_state`, PettingZoo's `ParallelEnv` shape) drives the real `MatchSim`. Observations are line of sight (`MatchSim::probe`) within a field of view and range, plus
  the HUD's variables; the server's per-client filter is **not** used because it is room relevance for bandwidth (a neighbouring room is relevant, a map without rooms sends everyone). Privileged state is only in `debug_state`.
- **A property test with a hand-rolled shrinker** (no `proptest` dependency) for prop ownership against a model; with the two-owner guard removed it shrinks to `[Pick(2,2), Pick(0,2)]`.
- **The `collision` and `deactivate` action text and SPEC say what they do**: for players; loose props still collide with the object and a prop resting on it is not woken (found with a probe: a crate stays on a hidden, non-solid platform).
- **Rejected:** sccache (0% Rust hits across target directories, +7-9% on every cold build; measured), an ECS, Salsa, nextest, the `proptest` crate: no measured need. **Kept as reference:** Bevy's extract/prepare (retained staging
  already exists: one moved or recoloured object dirties one 256-byte slot), Jolt's islands (promotion, sleeping and a wake pass already exist and are benchmarked).

## Consequences
- Map analysis on the largest legacy maps is about 6-8x faster and its output unchanged; the player's per-tick ground query uses the same function (effect on a server tick **not measured**).
- Network *logic* can be tested deterministically in milliseconds; the existing suites are not removed, only a few duplicated assertions may move over later. The virtual link models datagrams, not the OS, MTU discovery or TLS.
- Open experiments (see the study): props ignoring a rule-disabled object, one awake prop in a 4 000-prop pile costing about 1 ms a tick, a JSON repro artifact, opt-in zone timers, running independent serial suites two at a time.
