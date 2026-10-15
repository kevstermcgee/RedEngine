# Game systems milestone (2026-10-14): evidence, the update-path audit, and the roadmap

The owner's brief of 2026-10-14 asked for one bounded foundational milestone toward games with systemic depth, persistent worlds and long
lives (directionally Factorio, Minecraft, Dota 2), a dependency-ordered roadmap after it, and an audit of the in-game update path. The milestone
is ADR 2026-10-14-game-systems-compose-under-one-match-authority (PR #165); this page is its evidence, the audit and the roadmap.

## Starting point

- **Commit:** the brief's review baseline was `3157e2b`; work began on `main` at `cfff882` (that baseline plus #162, the command line as a
  library). Working tree clean; the shooter's move to Killchain (ADR 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the, step 6)
  was paused before its last step and is still paused.
- **Route:** native Linux, the headless development route (`scripts/dev`), a prebuilt `red_engine2` fetched for the checkout (checksum
  verified). Machine: Intel N97 (4 cores), 15 GB RAM, Linux 7.0, rustc 1.98.1.
- **Observed baseline:** game logic as data (`rules`), one `MatchGame` per match with a snapshot part and a client decoder, scene blocks a
  crate defines, `persist` (numbers only), `checks.sim` reading scene variables and events only. No per-instance game state with ids and
  provenance, no conserved resources, no versioned game state, the game's world state outside the match checksum, nothing to ask "why is
  this stuck". `scripts/dev start` routed the task to the upgrade workflow because it mentions "migration" (recorded as friction).

## What was built

The primitive (red_core, game-agnostic): `sim::defs` (namespaced ids, definitions with `file#/pointer`), `sim::ledger` (transactional
stores, conservation audit), `sim::systems` (the host for a game's systems: dependency order, bounded events, commands, one seeded random
stream, versioned save with migration and rollback, inspection). Hooks with defaults that change nothing for other games: `MatchGame::state_hash`,
`MatchGame::inspect`, `MatchSim::set_var`. Scenarios gained `{"game": id, "path", cmp}` and `sim --inspect`. Workshop (`games/workshop`,
`games/workshop/server`) is the reference game on the public API only; `tests/systems_abilities.rs` reuses the primitive for something else.
Engine fixes found by building it: a block's files resolve against its scene everywhere a scene is loaded for play; a game part too big for a
snapshot is left out and counted instead of cut; the custom-client HUD paints the scene's `ui` block.

## Evidence ledger

Each kind of evidence separately; "not tested" is stated, not implied.

| kind | what holds, and how it is known | not covered |
|---|---|---|
| **schema-valid** | `workshop_server cli validate content/workshop.json` (definitions read through the block); content errors carry `file#/pointer`, a stable code and a fix (`content_problems_name_the_file_and_pointer_with_a_fix`: duplicate across files, missing replacement, misspelt cross-namespace reference, unknown key, undeclared scene variable) | no standalone checker for a definitions file (`validate defs/x.json` reads it as a scene: friction) |
| **physically reachable** | `workshop_server cli lint` 0 errors 0 warnings; the lever is reached by a scripted walk (`checks.sim`) and by clicks through the client's input path | no `reach`/`walk` checks on the room |
| **behaviorally correct, tested paths** | primitive unit tests 13; reuse fixture 3 (cost once per cast, cooldown per caster, inspection, save mid-cooldown); Workshop `checks.sim` 3 (congestion without the lever; the lever ships the order; the expansion runs from content alone); crate tests 8: isolated state of two kinds of one behavior, conservation over 2400 ticks of production with stops and reroutes, repeated and impossible commands change nothing, dismantling hands everything back or is refused whole (**corrected 2026-10-14**: only a completely full storage was tested; with room for the input but not the output too, a refused dismantle moved the input and changed the hash, tick 2160 of `workshop.json`; fixed with `Ledger::remove_stores` and the host restoring state on any refused command, `tests/probe_dismantle.rs`), determinism by seed, save mid-production reloads into the same future, v1 to v2 migration; an intentional production cycle (recycling gears into ore) runs and audits | cancellation and despawn are tested at the command level, not as a player action in the client |
| **same authority, CLI and client** | the client (`app::LocalSession`) and a bare `MatchSim` reach the same systems hash at the same tick; the client's lamps and HUD lines read the game's inspection | the client is single-player; online play from the custom client is the engine's "not yet" (`describe custom-client`) |
| **network-tested** | over `net::memnet` (virtual clock, seeded): 10% loss, 40 ms delay, 30 ms jitter; a late joiner after 5 s and a client cut off past the 3 s timeout both decode exactly what the server's game encoded at that tick; malformed parts refused; a part over 1400 bytes is omitted whole and counted, the client stays connected with no bad packets | real UDP/QUIC sockets for Workshop (the engine's socket suites cover the transport, not this game); player decisions online (they are player input like any movement) |
| **migrated** | `content/saves/v1.json` (a real mid-production save rewritten into version 1's shape) loads with the machines migrated 1 to 2 and progress kept (the cycle 75% done has a quarter left); a newer version, a broken migration and another game's save are refused with `state-version` and nothing changes; the file is only read; the next save is atomic and keeps the old file as `.bak`; the client reports a refused save and leaves the file untouched | a real version-1 release never existed: the fixture is authored from version 2's save |
| **visually inspected** | two offscreen pictures of the client (congested: red lamp on the press, the fix on the HUD; shipped: the objective and the order counter) were looked at | not on a real display or Windows |
| **measured at a stated scale** | see Measurements | probe sizes only, never capacities |
| **updated** | nothing: the update path was audited, not changed (below) | everything in the update acceptance list |
| **determinism scope** | same binary, same platform (Linux x86-64): repeated runs, save/reload and client-versus-headless agree to the hash; systems state is integer counts and ticks, ordering is by id, randomness is one saved splitmix64 stream; no physics involved | other platforms (the Windows CI job runs the game crates on `main` pushes, not cross-checked against Linux hashes) |

**Not certified:** fun, strategic depth or longevity. The fresh agent's note that the second furnace barely helps (the mine is the
bottleneck and machines are fed in id order) is a design observation the tests do not judge.

## Measurements

Release build, the machine above, `cargo test -p workshop_server --release --test scale -- --ignored --nocapture`: N machines (half
furnaces, half presses, one behavior) on one shared storage, through the real match (`MatchSim::tick_once`), 120 warm-up then 600 timed ticks.

| N | build | tick p50 | p95 | p99 | max | save (JSON) | save | load | snapshot part | RSS growth |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 0.38 ms | 9.1 us | 12.5 us | 20.9 us | 40 us | 3.0 KB | 0.14 ms | 0.14 ms | 108 B | 2.5 MB |
| 100 | 0.68 ms | 127 us | 147 us | 202 us | 333 us | 26 KB | 1.2 ms | 1.1 ms | 828 B | 1.0 MB |
| 1000 | 5.4 ms | 1.54 ms | 1.73 ms | 2.38 ms | 3.44 ms | 261 KB | 12.3 ms | 12.3 ms | 8.0 KB | 9.3 MB |

The declared budget (stop when p95 passes 4 ms or a run passes 60 s) was not reached. **The first resource bottleneck, measured with
`perf` at N = 1000:** `memcmp` 21.6% (string-keyed map lookups of ids), `malloc`/`free` about 27% (ids formatted and parsed per tick, units
cloned), `Ledger::count`/`free` 13%. **The first network limit:** the whole-state part is 8 bytes per machine; above about 170 machines it no
longer fits a snapshot (1400 bytes) and is left out.

- **Bandwidth** (the part only, 30 snapshots a second per client): the reference workshop's part is 48 bytes (51 on the wire, about 1.5
  KB/s); N = 10: 3.2 KB/s; N = 100: 25 KB/s.
- **Warm authoring loop** (the build `scripts/red` uses): `workshop_server cli validate` under 0.01 s, 12 MB; `sim` of both scenarios (3241
  ticks) 0.02 s, 14 MB; `sim --inspect` under 0.01 s.
- **Cold authoring cost:** 269 s and 1.4 GB for `cargo build -p workshop_server` into an empty target directory (the headless engine and
  its dependencies, debug profile); a one-file change to the machine behavior then rebuilds in about 1 s. A content edit needs no build.
- **Agent costs:** `describe systems` 3.3 KB; the scripted agent task (`games/workshop/server/tests/ai_task.rs`) reads 21 KB in 8 steps with
  one repair; the engine's discovery task (`tests/ai_tasks.rs`) 11 KB.

### Fresh-agent trial

A model with no context from the session, given the task "add a second furnace through content only, prove it valid, the scenarios still
passing and the furnace running, with the file it came from" and the two programs: **succeeded** in 325 s, 19 tool calls, 69.5K tokens
(the harness's count for the whole sub-session), one error caused by a tool (`validate` on a definitions file), no help. It found the loop in
`describe systems`, planted two errors on purpose to prove `validate` reads the definitions, and added its own scenario. Its friction:
`validate` with no paths in a game folder checked the engine repository; a definitions file has no checker of its own; an instance's own file
was only in its state (now `instance_at`, top level); `sim --only` matches substrings; the room's lamps are documented only in the README.
Tool definitions it loaded were the session's standard tools (not measured separately); reading of docs beyond tool output was none.

## The in-game update path: audit

Read on RedEngineGames `160ed2a` (`distribution/play_launcher.rs`, `installer.iss`, `release_tool.py`, `smoke_install.ps1`,
`site/generate.py`, `docs/DISTRIBUTION.md`). Three different things, kept apart: **developer engine upgrades** (`red_engine2 game upgrade
plan|verify`, in the engine), **authored-content upgrades** (a game's save and definitions versions: this milestone's `systems` migration),
and **player-installed updates** (this audit).

What exists and is sound:
- Per-game versions and immutable releases (`<slug>-v<N>`, never rewritten); each game updates on its own; every version stays downloadable.
- Saves in `Saved Games\<Game>`, never in the install folder; the uninstaller keeps them unless asked.
- Offline play: the check has a 4 s timeout and fails silently, logged to `launcher.log`; it is never reported as corruption.
- No downgrade by prompt (only a newer version is offered; "skip this version" is remembered).
- The release runner installs, updates two ways (installer over the top, the in-game prompt against a local server), starts and uninstalls
  a game before anything is published (`smoke_install.ps1`).

Gaps against the brief's bar, most serious first:
1. **No authenticated metadata.** `latest.json` and the SHA-256 it names come from the same site; the hash proves the download matches the
   metadata, not that the metadata is the publisher's. Whoever can change the Pages site (or the repository behind it) can make every
   installed game run any installer, silently, with the player's rights. No pinned trust root, no signature, no expiry (a frozen
   `latest.json` is undetectable), no key rotation.
2. **No platform code signing** (optional, no certificate configured): SmartScreen warns and nothing proves the executable's origin.
3. **No staging or recovery.** The installer replaces the game in place (`InstallDelete` of `content`, files overwritten) while the old one
   closes; no previous vetted executable is kept, no health check of the new version, no recovery state machine if it fails to start.
4. **No save safety around an update.** Nothing backs up the saves before a new version may migrate them, and an older executable run after a
   newer one can read (or overwrite) a newer save.
5. **Silent install without cancel or progress.** One `curl` download (600 s limit) and a silent installer; no progress, cancel, retry,
   resume or disk-space check.
6. **No compatibility gate.** An update is offered by version number alone; nothing compares save schema, content or protocol identity, and
   a multiplayer session pins only the protocol version and map hash, not the executable or content release.
7. **Untested failure modes:** interrupted download, corrupt download beyond the checksum, invalid signature, low disk, startup failure of
   the new version, incompatible save, rollback.

None of this was changed here: the acceptance the brief sets (a real Windows two-version update and relaunch, retained progress, and the
injected failures) needs a Windows machine, and the trust decisions (who holds the offline root key, the code-signing certificate) are the
owner's. Every item in the update acceptance list is **unverified**.

## Roadmap (dependency-ordered; each a bounded increment with an executable acceptance fixture)

1. **Indexed runtime data** (the measured CPU bottleneck; nothing else is blocked by it, everything later is cheaper with it). Definitions
   compile at load into dense indices (an `Id` interned once to a `u32`; stores and resources as slots), the authoring JSON unchanged.
   *Acceptance:* the scale probe at N = 1000 with p95 at most a third of today's 1.73 ms on the same machine; the same systems hash as the
   string version over 3000 ticks of `workshop.json` and `workshop_plus.json` (golden parity); zero allocations per steady-state tick (the
   engine's `alloc_budget` pattern); `describe systems` and the content files unchanged.
2. **Replication of game state: relevance, baseline and deltas, per-client budget** (the measured network limit; needs 1 for cheap diffs).
   A game part becomes a baseline plus changed instances since the client's acknowledged snapshot, filtered by relevance (interest) and a
   per-client byte budget, with hidden-information filtering. *Acceptance:* a 1000-machine workshop over `memnet` at 10% loss converges to
   the server's view hash within a bounded number of snapshots after a late join and after a reconnect, never more than the budget per
   snapshot; a viewer never receives another team's private store (a negative test); the existing 3 online tests unchanged.
3. **Event dispatch, queries and scheduling with bounded work** (needs 1). Blocked machines sleep and are woken by the store change that
   could unblock them; overload is a visible diagnostic, never a drop. *Acceptance:* 10 000 machines with 1% active tick in time
   proportional to the active ones; identical hashes to eager ticking over 3000 ticks; an overload test that halts with `event-overflow`
   and names the chain.
4. **Persistent world regions** on the procgen and streaming facilities (needs 1 and 3). Region chunks own their systems' state; render
   visibility, replication relevance, active simulation, sleeping and unloaded are separate states; each system declares its offscreen
   semantics (simulate, catch up by formula, or freeze). *Acceptance:* unload and reload of a region gives the same hash; a transfer across a
   region boundary conserves the ledger; authored modifications survive a generator version upgrade (seeded regeneration plus the edits);
   collision, persistence and presentation agree at a boundary (one scenario walking across it).
5. **Optional mechanics, extracted only on a second consumer** (by the system-audit rule, ADR 2026-10-14-a-system-stays-in-the-engine-when-two-games-use-it-or):
   inventory and crafting (Workshop plus a second game), abilities and modifiers (the fixture plus a game), progression, agent goals,
   world interactions; reusable state-bound inventory and inspection UI that a game themes. *Acceptance:* two games consume the module,
   each game's tests unchanged, the module's own tests in the engine.
6. **Long-running protection** (alongside 2 to 4). Commands recorded in match traces so a replay reproduces the systems hash; property
   tests of the ledger (random transaction sequences always audit); a soak workload (an hour of a 100-machine workshop, memory flat, no
   halts); a crash artifact (the save and the trace) on panic. *Acceptance:* each as a test; the soak as a measured, scheduled CI job.
7. **The update path** (independent of 1 to 6; needs a Windows machine and the owner's trust decisions). Authenticated metadata with a
   maintained implementation (The Update Framework, e.g. the `tough` crate) and an offline root key, platform code signing, a staged install
   with a health check and the previous version kept, a save backup before migration and a refusal to open a newer save, compatibility gates
   on save schema, content and protocol identity, a reusable update service a game's menu shows (check, notes, progress, cancel, retry,
   apply at a safe point). *Acceptance:* the brief's Windows matrix (two-version update and relaunch with progress kept; interrupted
   download and install, invalid signature, corruption, low disk, startup failure, incompatible save, rollback), each injected on a
   Windows runner.

Also queued: the shooter's move to Killchain (step 6 of ADR 2026-10-14-a-game-plugs-into-the-server-s-match-simulation-the; its tooling is in
place since #162 to #164); the friction notes in STATUS.md.

## The next demonstrated bottleneck

String-keyed id lookups and per-tick allocation: about half of Workshop's tick at 1000 machines (`perf`, above). Roadmap item 1 removes it
without changing what anyone writes.
