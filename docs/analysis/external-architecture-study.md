# External architecture study: what eleven projects teach RedEngine, and what it already does (2026-09-30)

Scope: eleven reference projects were read for one technique each and compared with the RedEngine subsystem they bear on. The rule was to adapt an idea only where RedEngine has a
*measured* problem, and to say so plainly where it does not. Everything below is either measured on the dev box (Intel N97, 4 cores, 15 GB, which also hosts the game server; engine
commits `dc88e22`..`a924b16`, rustc 1.98.1) or marked **not measured**. Raw numbers for the workflow half are in `benches/history/build-times.json` (run
`2026-09-30-dev-workflow-iterate`), decisions in ADR `2026-09-30-the-engine-says-what-it-did`, ADR `2026-09-30-bounded-iteration-explicit-configuration-and-safe-green` and
ADR `2026-09-30-evidence-driven-refinements-from-the-external`.

**How the references were read, and its limits.** Each was read through its published documentation or one or two source files, fetched and summarised by a tool, not read line by line:
rust-analyzer `architecture.md`; Salsa's overview; `quinn-proto`'s crate docs and `tests/util.rs`; Bevy's `render_asset.rs`; Flecs' `Queries.md`; Jolt's `Architecture.md`;
FoundationDB's testing page; proptest's shrinking tutorial; PettingZoo's `ParallelEnv` page; Tracy's README; nextest's test-groups page; sccache's `Rust.md`. **Not read:** Salsa's or
rust-analyzer's implementation, Bevy's `ExtractSchedule` and instance data, Tracy's instrumentation internals, nextest or sccache source. A claim about a reference below is a claim
about what those pages say.

## Summary of decisions

| | Finding | Class |
|---|---|---|
| Reachability flood fill (found while asking the Salsa question) | 92% of `reach` time was a linear scan over every box top, per cell and neighbour. A grid index: office map 7.85 s -> 0.54 s, 80/80 outputs identical | **IMPLEMENT NOW** (done) |
| `lint` output order | Same-code findings printed in a different order on different runs of one binary | **IMPLEMENT NOW** (done) |
| In-memory virtual-clock network (Quinn, FoundationDB) | lobby-to-rematch flow 5.27 s over UDP -> about 0.03 s; seeded loss/jitter/partition; two runs with one seed identical | **IMPLEMENT NOW** (done) |
| Agent interface (PettingZoo) | `sim::env`: `reset/observe/step/terminated/debug_state`; observation by real line of sight | **IMPLEMENT NOW** (done) |
| Property test with shrinking (proptest) | prop ownership vs a model; a broken guard shrinks to `Pick(2,2) -> Pick(0,2)` | **IMPLEMENT NOW** (done) |
| Development workflow (nextest/sccache questions, build evidence) | `iterate` tier, safe stamps, runtime index, tree-only preflight, lock; see the two workflow ADRs | **IMPLEMENT NOW** (done) |
| Props ignore a rule's `collision: false` and are not woken (Jolt) | a crate stays on a hidden, "non-solid" platform; docs now say so | **EXPERIMENT** |
| One awake prop among 4 000 sleeping ones costs about 1 ms per tick (Jolt, Flecs) | 117x the cost of 1 000 fully asleep for 4x the props; cause not found | **EXPERIMENT** |
| Migrate more network *logic* tests to the virtual link | net suites are 88% of test run time | **EXPERIMENT** (the prototype is the evidence) |
| Byte-exact replay of a handshake; a JSON repro artifact (FoundationDB) | handshake nonces come from the OS | **EXPERIMENT** |
| Opt-in zone timers (Tracy) | `perf` and `gdb` are both blocked on this box; I localised the reach cost by hand | **EXPERIMENT** |
| Salsa / rust-analyzer incrementality | after the index fix, derived costs are sub-second even on the worst legacy map | **KEEP AS REFERENCE** (a geometry-keyed reach cache would save about 0.5 s) |
| Bevy extract/prepare, byte budget | retained staging already exists; moving one object uploads one slot | **KEEP AS REFERENCE** |
| Flecs cached queries, change lists | scans are 1-10 ns per entity; an ECS is not warranted | **KEEP AS REFERENCE** |
| Jolt islands and sleeping | promotion + Rapier sleeping + wake pass already exist and are benchmarked | **KEEP AS REFERENCE** |
| nextest | `ci.sh` already has serial and parallel groups; the scheduler is not the bottleneck | **KEEP AS REFERENCE** |
| Adopting Salsa, an ECS, nextest globally, or the `proptest` crate | no measured need | **REJECT** |
| sccache | 0% Rust cache hits across target dirs; no edit-loop benefit | **REJECT** |

## 1. rust-analyzer and Salsa: incremental computation

1. **Subsystem:** map analysis (`validate`, `lint`, `reach`, `plan`, `walk`, `verify`), scene expansion, `MapWorld`, renderer preparation, authoring commands.
2. **Studied:** rust-analyzer `architecture.md` (inputs vs derived state, `AnalysisHost` applying changes transactionally and handing out immutable `Analysis` snapshots, a revision counter, "typing inside a
   function's body never invalidates global derived data"); Salsa's overview (inputs, tracked functions, revisions, red-green recomputation, durability, "backdating" = early cutoff).
3. **Technique:** derived data is a memoised pure function of inputs; a change bumps a revision; a derived value is recomputed only if something it read changed, and a recomputation that yields the
   same value does not invalidate its dependents.
4. **RedEngine today:** every CLI command is a fresh process that parses and recomputes everything (nothing persists between `lint`, then `plan`, then `walk`). The only reuse is whole-file:
   `scripts/red check` skips maps whose bytes are unchanged; `affected` stamps green results by content.
5. **Is there the problem?** The question "if only a chair's colour changes, what is recomputed?" has a flat answer: everything, per command. Whether that matters depends on the derived costs, so I measured them.
6. **Evidence** (dev build, CPU seconds per command, before the fix below): `validate` (parse and expand) 0.03 s on every example. `lint`: house 0.35, test_lab 0.11, store 1.67, **office 7.27, school 7.26**.
   On the office map `reach` alone was 6.56 s of `lint`'s 7.24 s, and `plan` (7.59 s) and `walk` recomputed the same flood again. The release (LTO) binary was *slower* (8.2 s) than dev (7.2 s), so the
   cost was algorithmic, not a missing optimisation. A temporary counter showed why: 2 849 box tops, 2.96 M `ground_height_at` calls, 7.45 s of the 8.06 s fill inside that one function (92%); collision tests were 0.2 s.
   It scanned every box top linearly per call while collisions already used a bin grid. After the index (`GroundCandidates` keeps a lazily built uniform grid over the tops; dropped by `append`):
   office `reach` 7.85 -> 0.54 s, `lint` 8.55 -> 1.25 s, `walk` 10.45 -> 1.08 s, `maps_verify` 11.71 -> 1.57 s; all 40 commands over the ten examples 90.0 -> 12.3 s.
   Equivalence: a randomized test against the old full scan (40 scenes, every top's corners and edges at +-1e-4), and all 80 output files (text and exit codes) of `lint`/`reach`/`verify`/`walk` over the ten
   example scenes are identical to the pre-change build after normalising printed durations. The player's per-tick ground query uses the same function, so the server tick on furnished maps should benefit; **not measured**.
7. **Adaptation:** the speed-up is the "delete redundant work" answer. A Salsa-style cache on top (key = a hash of the collision inputs `Reach` actually reads, so a colour change hits and a moved doorway misses) would now
   save about 0.5 s on the worst legacy map and about 0.05 s on `test_lab`.
8. **Cost:** a cache needs `Reach` (de)serialisation, an on-disk store and invalidation tests, roughly 150 lines plus a failure mode (a stale entry).
9. **Justified?** The index: yes (done). A cache or Salsa: **no** (KEEP AS REFERENCE); revisit if maps grow by about 10x. Side finding fixed on the way: `lint` sorted findings by severity and code only, so same-code findings followed
   hash-map order and differed between runs of one binary (two `headroom` errors swapped, 2 orderings in 8 runs); the message is now the tie-break, with a test that fails without it (mutation-checked 3/3). A reproducibility bug for agents and for any golden output.

## 2. Quinn: pure protocol logic and deterministic network testing

1. **Subsystem:** game-level networking (`net::server`, `net::client`, `net::bot`) and its tests.
2. **Studied:** `quinn-proto` crate docs ("fully deterministic ... contains no networking code and does not get any relevant timestamps from the operating system": Endpoint/Connection state machines fed datagrams, application decisions and
   time, returning transmits, events and timer needs) and `tests/util.rs` (`Pair` with an explicit `Instant` clock, in-memory `VecDeque` queues, a `latency` field, MTU-based loss, `delay_outbound`/`finish_delay` for reordering).
3. **Technique:** keep protocol logic a function of (input datagrams, events, explicit time); test it with an in-memory pair on a simulated clock; keep few real-socket tests.
4. **RedEngine today:** already shaped that way: `Server::pump/tick/handle/on_input/on_hello/drop_session`, `NetClient::poll`, `Bot::pump` all take `now: Instant`, and everything sits behind `ServerTransport`/`ClientTransport`
   (UDP and QUIC implement them). What was missing was an in-memory implementation and a way to inject a client transport. `netsim.rs` is a lossy *proxy over real sockets*.
5. **Is there the problem?** Yes, in the tests: every `tests/net_*.rs` uses real sockets, threads and `sleep`. From the last full CI log (default features; the log held both configurations, rows halved), the 17 serial suites are
   about 88% of summed test run time (about 167 of 190 s): `net_e2e` 36.8 s, `net_sim` 32.1, `net_processes` 16.0, `net_quic` 15.8, `net_interest` 15.2, `net_flow` 12.2. They cannot overlap. The parallel-safe suites total about 18 s.
6. **Evidence:** `net::memnet` (`MemNet`, `MemServer`, `MemClient`, `LinkModel {loss, delay, jitter}`, `partition(peer)`; a seeded generator) plus `NetClient::with_transport` (one constructor split; `connect_with` now calls it), about 250 lines. `tests/net_virtual.rs`
   replays `net_flow`'s lobby -> countdown -> round -> results -> rematch assertions: **5.27 s over real UDP vs about 0.03 s** (the four-test file finishes in 0.07 s), a 10%-loss, jittery link still gets everyone into the round, a peer that vanishes is
   timed out by the server and can rejoin, and two runs with one seed agree event for event, at the same virtual instants, with the same simulation checksum. Mutation-checked: 95% loss and a missing partition both fail.
7. **Adaptation:** migrate the *logic* assertions of `net_e2e`, `net_interest`, `net_race_lobby`, `net_rule_state` and the rest of `net_flow` to `memnet`; keep `net_quic`, `net_processes`, `net_sim` and one UDP smoke test per feature on real sockets.
8. **Cost:** small, but the model is not the network: no MTU discovery, congestion, OS buffers or TLS. Handshake nonces come from the OS CSPRNG, so the *bytes* differ from run to run (outcomes and timings do not); byte-exact replay needs an injectable entropy source (see §6).
9. **Justified?** The link and the prototype: yes (done). The migration: **EXPERIMENT**, to be done suite by suite with the real-socket twin kept until the virtual one has been seen to fail for the right reason.

## 3. Bevy: extract, prepare, retained GPU resources

1. **Subsystem:** the wgpu renderer's per-frame preparation (`object_staging::SceneStaging`).
2. **Studied:** `render_asset.rs` only: `RenderAsset` (source asset, `prepare_asset`, `byte_len`, `unload_asset`), extraction of `Added/Modified/Unused` asset events into the render world, a per-frame byte budget with a retry queue. **Not read:** `ExtractSchedule`, instance data.
3. **Technique:** the render world holds retained GPU data keyed by asset id; only changed or removed assets are (re)prepared, and upload volume per frame is bounded.
4. **RedEngine today:** `SceneStaging` (commit `2579c64`) keeps per-slot inputs (world matrix + material), restages only slots whose inputs changed, uploads only dirty ranges, shares identical GPU meshes. Tests: `a_parent_move_dirties_every_descendant_and_a_leaf_move_only_itself`,
   `cached_bytes_match_the_full_rebuild_after_every_kind_of_mutation` (moves, materials, tracks), `preparation_allocations_are_flat_after_warmup`.
5. **Is there the problem?** "Does moving one chair redo work that belongs only to the chair's transform?" No: a leaf move or a material change dirties exactly one slot (one 256-byte uniform); a parent move dirties its descendants, as it must.
6. **Evidence** (`cargo bench --bench render_prep`, CPU only, per frame, p50): `test_lab` (350 meshes): old path 45.5 us, 376 allocations, 89 600 B uploaded; camera-only 22.2 us, 0 allocations, 0 B; 1% moving 22.4 us, 1 024 B; all moving 37.3 us.
   `house` (1 787 meshes): old 221 us, 1 843 allocations, 457 472 B; camera-only 128 us, 0, 0 B; 1% moving 130 us, 7 168 B. `synthetic-8000` (34 000 meshes): old 5.30 ms; camera-only 2.02 ms; 1% moving 2.21 ms, 370 KB; all moving 3.93 ms.
   The residual is a per-frame scan for change detection and frustum culling of about 60 ns per mesh: 0.13 ms at house scale, 2 ms at 34 000 meshes, against a 16.7 ms frame. **Not measured:** GPU time, bind-group creation, draw-call counts, buffer allocations on the GPU side.
7. **Adaptation:** a per-frame byte budget and a retry queue (Bevy) answer a problem RedEngine does not have (it streams no external assets). A dirty-set input to skip the scan would save at most a few per cent of a frame at extreme mesh counts.
8. **Cost:** a dirty-set API threads through every scene mutation site.
9. **Justified?** **KEEP AS REFERENCE.** Nothing to implement; the existing design is the Bevy lesson already applied.

## 4. Flecs: cached queries and change-oriented processing

1. **Subsystem:** per-tick loops over props and entities (`PropWorld::step`, `wake_disturbed`, `props_to_send`, `awake_count`), `sim::change::Generation`.
2. **Studied:** Flecs' `Queries.md` caching section (pre-matched archetype lists kept current by events: "match once, iterate many"; more RAM and overhead on structure changes; uncached for ad-hoc queries). The change-detection section was not detailed on that page.
3. **Technique:** cache the result of "which things match" and keep it updated by events instead of rediscovering it.
4. **RedEngine today:** no ECS. Un-promoted props are static instances with no body (0.2 us per step for 1 000-4 000 untouched props); promoted props are visited in one loop with an `is_sleeping` read; replication uses per-entity change generations (`changed <= confirmed` skips).
5. **Is there the problem?** The scans are O(N) with a tiny constant. `bench settled_world` (N = 1 000, p50): settled step 9.2 us (about 9 ns per prop), `awake_count` 3.4 us, `props_to_send` for a confirmed client 1.1 us and for a client that knows nothing (late join) 3.6 us.
6. **Evidence:** a changed-object list would replace about 10-30 us of scanning per tick at N = 1 000-4 000, under 0.2% of a 16.7 ms tick. One anomaly (see §5) is not a scan cost.
7. **Adaptation:** none. **8. Cost:** an event-maintained index adds state to keep coherent with promotion, sleeping, holding and reset.
9. **Justified?** **KEEP AS REFERENCE** (and **REJECT** an ECS). Objects examined per subsystem in settled vs one-moving: *inferred from the code* (every promoted prop for the step, wake and awake passes; every entity for `props_to_send`), not counted by instrumentation.

## 5. Jolt: sleeping, activation and mostly-static worlds

1. **Subsystem:** the Rapier integration in `physics/` and its consumers (scene sync, replication, rules, gameplay queries).
2. **Studied:** Jolt's `Architecture.md`: islands of touching bodies sleep together after coming to rest; sleeping bodies skip simulation and broad-phase cost; bodies wake on contact with an active body or by explicit activation; removing a body does **not** wake its neighbours
   (use `ActivateBodiesInAABox`); `Body::SetLinearVelocity` does not wake, `BodyInterface::SetLinearVelocity` does.
3. **Technique:** separate the active set from all bodies, and make every state change that should wake something say so.
4. **RedEngine today:** props start as static instances (no body, no entity), are *promoted* when disturbed (touching a table promotes what rests on it), sleep under Rapier, publish their resting pose once (`settled`) and then go quiet; `wake_disturbed` activates props overlapped by players or by
   moving bodies. Tests cover promotion, the resting-pose-once behaviour and promoting a supported stack.
5. **Is there the problem?** Mostly no. `bench settled_world`, per tick (p50): **untouched** N = 1 000 / 4 000: 0.2 us; **settled** (all promoted, all asleep) N = 1 000: 9.2 us; **16 moving** N = 1 000: 285 us.
   Two things remain.
   **(a) Anomaly:** N = 4 000 "settled after 3 000 ticks (awake now 1)": 1 077 us per step, 117x the N = 1 000 fully asleep for 4x the props; with 25 awake 2.26 ms. Replication and scene sync stay flat (`props_to_send` 4.4 us, `sync_scene` 0.0 us). Suspects (not tested): the wake pass
   querying and re-activating neighbours of a still-moving prop in a dense pile every tick, or Rapier contact work around that one body. **EXPERIMENT:** count `activate` calls per tick in that scene.
   **(b) A real semantic gap, found with a probe:** a rule `deactivate`s a platform that a loose crate rests on (`collision: false` and hidden). The crate stays at y = 1.00, moved 0.00 m. `collision` (and `deactivate`) change only the *players'* movement colliders and ground (`rebuild_static_world`); the Rapier world keeps the object's collider, so props still
   collide with it, and nothing wakes a prop resting on it: Jolt's "removal does not wake neighbours" caveat, plus props never see the disable at all. The action text and SPEC now say "for players; loose props still collide with it and stay on it". **EXPERIMENT:** make `PropWorld` honour a rule-disabled object
   (disable its fixed colliders, then wake props in its AABB), with a scenario test (crate on a platform that goes away must fall).
6. **Not audited:** save/load (none found), authoritative corrections of props on reconnect beyond the late-join selection cost above, removing a support by other means.
7. **Cost of (b):** per-object collider handles in `PropWorld` and a wake-in-AABB call; moderate. 8/9. **KEEP AS REFERENCE** for islands and sleeping (already present); the two items above are **EXPERIMENT**.

## 6. FoundationDB: deterministic simulation testing

1. **Subsystem:** multiplayer test scenarios and failure reproduction.
2. **Studied:** FoundationDB's testing page: a whole cluster simulated deterministically in one thread, with simulated time, network and disk; faults injected (machine, network; "swizzle-clogging"); a failure is reproduced from its seed; workloads use the same code in simulation and production.
3. **RedEngine today:** `sim` scenarios are deterministic and recordable (`sim --trace`, `replay` names the first divergent tick); `net-test` runs real sockets behind a seeded lossy proxy; and now `memnet` gives a seeded, virtual-clock link. Two runs with one seed are identical (tested).
4. **Gap:** a failing multiplayer scenario still prints a test failure, not a *reproduction artifact*. Everything needed exists as test code (`seed`, `LinkModel`, `MatchSettings`, the bots and their actions); writing it as one small JSON document on failure (and a `replay` that reads it) is about 100 lines. Handshake nonces come from the OS CSPRNG, so the *bytes* are not reproducible
   (outcomes are); byte-exact replay needs an injectable entropy source on client and server.
5. **Whether Engine Gauntlet could consume it:** not examined (no Gauntlet code in this repository was read); `.gauntlet/` is only listed as an ignorable path in `affected`.
6. **Justified?** **EXPERIMENT**; the prerequisite (a deterministic virtual link) is in place.

## 7. proptest: shrinking

1. **Subsystem:** prop ownership (`PropWorld::pick_up_by/drop_held_by/remove_player_slot/reset_prop/place_prop`).
2. **Studied:** the shrinking tutorial: a `ValueTree` with `simplify`/`complicate` searches for the smallest failing input without leaving the strategy's bounds; the page does not detail sequence shrinking.
3. **Implemented:** in `physics/tests.rs`, about 100 lines, no dependency: a seeded xorshift generator of operation sequences (`Join, Leave, Pick, Drop, Reset, Place, Step` over 3 players and 4 props), a model of the documented rules, invariants (no prop with two holders, no player with two props, `held_by`/`holder_of`/`is_held`
   agree), and a chunk-removal shrinker. 40 seeds x 70 operations pass. **Teeth:** with the `is_held(prop)` guard removed, seed 0 fails at operation 10 and shrinks to `[Pick(2, 2), Pick(0, 2)]`; a second test shows the shrinker turning a 70-operation sequence into exactly `Join(s) -> Pick(s,_) -> Leave(s) -> Join(s)`.
4. **Why not the crate:** it would add a dev-dependency tree to a 4-core box where builds are already the cost; **not measured**, and a 100-line generator and shrinker did the job. Revisit if several more state machines want it.
5. **Justified?** **IMPLEMENT NOW** (done, one state machine). Natural next candidates: lobby/round flow (`sim::flow`), reconnect handling, rules with `once` and `cooldown`.

## 8. PettingZoo: one interface for agents and tests

1. **Subsystem:** how bots, tests and AI agents play a match.
2. **Studied:** the `ParallelEnv` page: `reset(seed) -> (observations, infos)`, `step(actions) -> (observations, rewards, terminations, truncations, infos)`, per-agent observations, `possible_agents` vs `agents`, and a separate `state()` for the global view.
3. **Implemented:** `sim::env` (`Env::reset/observe/step/terminated/truncated/debug_state`). It drives the real `MatchSim` (`push_input` + `tick_once`, the path scenarios and the server use): **no second copy of any rule**. A seed shuffles unnamed spawns and two resets with one seed are bit-identical. No rewards: the rules language has none (judge by `terminated` and public variables).
4. **Visible vs privileged.** An `Observation` is own body, other players by **line of sight** (`MatchSim::probe` from the eye to the body, so walls, furniture, loose props and other bodies block it) inside a field of view (110 degrees) and range (40 m), and the HUD's variables (names not starting with `_`).
   Everything else (all positions, `_` variables, game events, checksum) is only in `debug_state`.
   **Important finding:** the server's own per-client filter (`net::snapshots::visible_players`) is **room relevance for bandwidth, not visibility**: an adjacent room counts as relevant, and a map with no rooms sends *every* player to every client. It is correct for bandwidth and the wrong basis for hide-and-seek, where a modified client could read
   positions the player cannot see. Not changed here; a game that depends on hidden players must not treat the network protocol as secret.
5. **Evidence:** `tests/agent_env.rs` finishes the existing two-agent run of `recipes/coin_run.json` (a human and a rat split the coins; the exit is entered only once the public `score` says 3) using observations only and ends in `victory`; a wall hides the other player although both are 8 m apart and `debug_state` has both; turning away hides them; a 3 m range hides them; `_` variables and events are absent from observations; errors and tick-limit truncation.
   Mutation-checked: removing the line-of-sight test fails the wall test.
6. **Justified?** **IMPLEMENT NOW** (done). Natural next: a 2-player hide-and-seek scene as the first consumer, and bots that act through `Env`.

## 9. Tracy: evidence instead of guesses

1. **Subsystem:** profiling and performance evidence for agents.
2. **Studied:** Tracy's README only: a real-time, nanosecond, remote-telemetry, frame-and-sampling profiler; instrumentation is opt-in (`TRACY_ENABLE`); a separate GUI reads the capture; Rust is through third-party bindings. Per-zone overhead and the disabled cost were not on that page.
3. **RedEngine today:** `red_engine2 perf` (server tick p50/p95/p99 against `checks.perf` budgets), criterion-style benches, per-check durations in `verify`. No zone profiler.
4. **Is there the problem?** Yes, in the practical form: on this box `perf` is blocked (`perf_event_paranoid = 4`) and `gdb` cannot attach (ptrace scope), and I will not change system settings. Finding the `reach` hot spot therefore took hand-written temporary counters around the suspect calls (about 10 lines, then reverted). That is the cost future agents would pay again.
5. **Adaptation (not built):** a zero-dependency, runtime-opt-in layer (`RED_ZONES=1`; a guard costing one relaxed atomic load when off) with phase-granularity zones (scene parse and expand, `MapWorld`, `reach` seed/fill, each lint check, each `verify` group, the sim tick stages) and a compact text/JSON summary (zone, calls, total ms, share of the run) printed at exit, so an agent reads five lines and not a trace.
   It would have localised the `reach` cost to its fill loop in one command; **attributing it to `ground_height_at` needed per-call counters**, which phase zones do not give (a sampled inner-loop counter could).
6. **Cost:** about 80 lines plus instrumentation sites and documentation; no Cargo change if it is runtime-switched.
7. **Justified?** **EXPERIMENT**: worth a bounded prototype, not yet evidence of repeated cost beyond this session.

## 10. nextest: test execution on a small machine

1. **Subsystem:** how `scripts/ci.sh` and `affected` schedule tests.
2. **Studied:** nextest's test-groups page: per-group `max-threads` (a group of 1 = serial), assigned by filter; distinct from the global thread count and from `threads-required` for heavy tests. nextest itself was **not installed or benchmarked** (installing it would change the user's environment).
3. **RedEngine today:** `ci.sh` and `affected` already split the suites: the 17 in `serial_suites` (`docs/features.json`) run one test at a time (`RUST_TEST_THREADS=1`), everything else in parallel.
4. **Measured (parallel group: lib, bins and the 27 parallel-safe suites, execution only, warm):** `RUST_TEST_THREADS=1`: 34.8 s wall, 22.0 CPU-s (0.63 cores busy); `=4`: 14.4 s wall, 25.1 CPU-s (1.74 cores), tree memory about 350 MB; both repeatable to 0.1 s. More threads are simply faster here; there is no contention to limit.
   `maps_verify`, the heaviest parallel suite (11.7 s in the last full log), is now 1.6 s because of the ground index. Tests that spawn servers: the `net_*` suites, `server_env`, `sim_replay`, `net_processes` (spawns processes); physics-heavy: `prop_physics`, `maps_verify`, `sim_replay`; load-sensitive: `net_sim`'s cruel-link case (documented earlier). **Not measured:** how the serial suites slow down when run two at a time.
5. **Where the time really is:** the serial network suites, 88% of run time. The scheduler is not the lever; the virtual link (§2) is.
6. **Justified?** **KEEP AS REFERENCE.** Candidate **EXPERIMENT:** which serial suites are really independent (they bind port 0 on loopback) and could run two at a time, judged by flake rate.

## 11. sccache: evaluate, do not assume

1. **Subsystem:** compile times (cold, rebuild, edit loop, revision switch).
2. **Studied:** sccache's `Rust.md`: incremental compilation is not cached (it must be off); crates that invoke the linker (bin, dylib, cdylib, proc-macro) cannot be cached. The page did not address absolute paths or target directories.
3. **Method:** a private `SCCACHE_DIR` and fresh private target dirs (all removed afterwards; the repo, the user's caches and the running server untouched), `cargo build --locked --bin red_engine2`, dev profile, nothing else running. sccache 0.7.7.
4. **Results** (wall; CPU and memory of sccache runs are **undercounted**, the compiling happens in sccache's server outside the build's process tree):

   | step | wall |
   |---|---|
   | 0. cold build, no sccache, fresh target dir | 374.8 s (1 344 CPU-s, peak tree memory 1.96 GB) |
   | 1. cold build, sccache, empty cache | 401.6 s (+7%); 0 hits, 333 misses, 86 uncacheable calls |
   | 2. fresh target dir, **warm** cache | 408.4 s (+9% vs no sccache) |
   | 3. immediate rebuild | 0.5 s (cargo's own no-op) |
   | 4. one-line engine edit, sccache on (incremental) | 5.1 s |
   | 5. the same edit, no sccache | 8.6 s (different target dir; both inside the 2-9 s band measured for this edit elsewhere; sccache cannot cache an incremental crate) |
   | 6. older revision, fresh target dir, warm cache | 401.6 s |

   Cache statistics after step 6: 178 hits, **all 178 C/C++ compiles (89 twice)**, and 732 Rust misses: **a 0% hit rate for Rust crates across target directories**, consistent with the earlier phase-1 result (89 hits vs 245 new misses). The cache was 817 MB; each target dir 1.6-1.8 GB.
5. **Conclusion:** sccache does not improve RedEngine's normal local edit loop, and it does not carry over between target directories either (rustc's command line embeds the target-directory paths; a path-remapping flag might change that and was not tried). It is slower on every cold build measured. CI runners reuse one path, so a CI-only cache might behave differently: **not measured**.
6. **Justified?** **REJECT** for local development. The existing `fast` profile remains the baseline.

## Findings outside the eleven (found while measuring)

* **The dev loop was not the problem, `--release` was** (release binary 3 min 18 s for a one-line edit vs 2-4 s to type-check). Documented and fixed in the workflow ADRs.
* **A stale feature index**: `scripts/dev` planned with a compiled-in copy of `docs/features.json` and never rebuilt it, so an edit to the index was invisible to `context`/`affected`. The index is now read from the checkout.
* **Cargo `-j` is not a lever** for rebuilding the 42 test binaries after a central edit (means 48.5 / 43.5 / 49.5 s for `-j2/-j3/-j4`, spread 36-61 s inside each): about 1.3 GB written, 25-44 s of I/O stall.
* **Rules and physics disagree about `collision: false`** (see §5).

## What to do next, in order

1. Migrate the logic assertions of the remaining network suites to `memnet`, one suite at a time (saves most of the 167 s serial group).
2. Make `PropWorld` honour a rule-disabled object and wake what rests on it (test first).
3. Find why one awake prop among 4 000 costs about 1 ms (count `activate` calls per tick).
4. A JSON repro artifact for a failing virtual-network scenario, then byte-exact replay via injectable entropy.
5. The opt-in zone timers, only if a second profiling question of this kind comes up.
6. Consumers for `sim::env`: a hide-and-seek scene and bots that play through it.

Not measured anywhere above: GPU time, the server tick on large furnished maps with the new ground index, Windows, a CI-side compiler cache, nextest itself, and the real-UDP suites' behaviour under load.
