# 2026-10-13. The next phase
Status: proposed
Summary: Sequence for scale without absorbing games: docs at runtime and prebuilts first, then red_core, CI, shippable game crates; then per-entity rules to Prop Hunt; then own shapes as the cheap path; each with a measured done

## Context
Red's bet is that a game is data and that the engine describes itself. The owner's brief of 2026-10-13 sets the target state:

1. A game is data while the rules language carries it, and a shippable game crate on the public API when it cannot. No more game code in the engine crate
   "because a crate of its own could not ship".
2. The engine crate holds reusable systems only. A headless `red_core` builds and tests in a fraction of the full tree's time.
3. An original-looking game is the cheap path.
4. Every entry document an agent pays for per session has a byte budget.
5. Long horizon: rules plus the custom-client layer carry a game at the scale of Factorio, RimWorld or Dota 2. Nothing here builds that; nothing may move away from it.

An outside review of b5b458f was re-checked item by item at 4c45d86 (`docs/analysis/2026-10-13-next-phase-review-verified.md`): almost everything holds.
Corrections: the museum's `include_str!` of a game file is test-only; 10 shared `#[path]` files, not 9; PR shards compile for 229-258 s each, ~27 job-minutes
per PR; 28 files over 1,000 lines (two grown by #115); 69 ADRs that no feature cites. Fixed already: `start` routes plain game prompts to game-create.
Not yet answered in the repo: the prebuilt question in STATUS.md (no release exists, so `prebuilt.py` has never matched).

**Where the brief and the owner's feedback disagree, the feedback wins:**
- *Order of the rules gaps.* `docs/RULES_LANGUAGE_GAPS.md` ranks a fall-out height second (`player_below`, `world.ground: none`), and the Idea Forge backlog has
  it (`implicit-ground-plane-hides-voids`, `no-void-or-kill-floor`); the brief leaves it out. No run reported a missing 3D spawn/destroy; three did report rules
  that change scale or movement, and one a body swap (`no-body-swap-or-camera-attach`), which Prop Hunt needs. Phase 2 below follows the evidence.
- *Strings and the story system.* #99 is merged and is primitives, not a system (ADR 2026-10-10-story-is-rules-and-tables-not-a-story-system): 2D has string
  tables, `pick`, `wrap` and a dialogue widget; 3D has none. "Strings" here means bringing that design to 3D; "hold the story system" means no story layer above
  these primitives until per-entity state exists.
- *Cold start comes first.* The backlog's top key is the cold build (recurring, 12 runs, 100 minutes). The prebuilt answer and docs read at runtime (which keeps
  doc edits from changing the build fingerprint) move to the front of Phase 1.
- *What a game ships.* The Marcel feedback (2026-10-04, finding 13) wants the engine installed once and games as content packs. A game crate ships its own
  binary only because it has its own code; a data game must not need one.
- *The shape of the seams.* The Frameless feedback (2026-10-11, findings 1-2) names them: one generated-world trait both renderers draw (`models`,
  `colliders_near`, `lamps`, `shader_params`) and a `GameMode` hook in the client (eight entry points). The museum moves out behind those seams.

## Decision
Rules for every step: one pull request per item; never a structural move and a behaviour change together; a parity run for every move (the S5/S6 pattern);
every claimed improvement measured in `docs/analysis` with its method; hosted CI green before the owner is asked to merge; `status --note` at each checkpoint.

**Phase 1, structural** (in this order; the brief's items 1 and 2 swap because docs at runtime makes every later PR cheaper and the prebuilt possible):
1. *Docs at runtime.* Dev builds read SPEC, AGENTS, REFERENCE, PLAY_2D, GLOSSARY, the ADRs and the analysis notes from the checkout (as `docs/features.json` already
   is); release builds embed them. **Done:** a doc-only PR compiles no crate in CI (its log has no `Compiling red_engine2`), and `search` finds an ADR created
   after the build, by test.
2. *Prebuilts.* Answer to STATUS.md: yes, `release.yml` publishes a prebuilt (Linux x86_64, then Windows) for every `main` commit that changes a build input,
   keeping the last N. Publishing releases is an outward action: the owner enables it. **Done:** a fresh clone of main reaches `describe --brief` with no
   compile, measured.
3. *`red_core`.* A crate with schema, content, sim, physics, net, collide and player (the headless server's tree) that `red_engine2`, red2d, `re2` and game crates
   depend on; red2d's `#[path]` sharing ends. First cuts: `net` -> `tools::portmap`, `net::session` -> `ui::race`. **Done:** the headless build, clippy and tests
   compile only `red_core` and its dependencies; no `#[path]` across crates; a one-line edit in `sim/` goes through `iterate` in under 20 s warm on the 4-core box,
   measured against today's ~50 s.
4. *CI.* Build the nextest archive once and fan it out; on PRs run headless as one shard of the net_*, server_env and sim_replay suites, with the full headless matrix
   on main; `save-if` keyed on Cargo.lock; Docker on main only. **Done:** before/after in `docs/analysis` with a method, against #115's 59.0 job-minutes and ~27
   job-minutes of compilation.
5. *A shippable game crate.* `package`, `game play` and `publish` know a game binary (Killchain first). Add the two engine seams above, then move Frameless to a crate
   behind them, its saves and a seeded visit unchanged (parity: the same rooms, plaques and keys for the same seed). Then audit the ~30K lines: a reusable system
   stays and its `//!` line names the games that use it; one game's code moves to its crate or an example; Killchain's wire structs go behind a game-message
   extension (a protocol version bump). The rule gets its own ADR. **Done:** `src/museum` is gone from the engine crate, Frameless ships from its crate, and the
   audit table is in `docs/analysis`. The audit re-checks earlier calls rather than presuming them: the loadout shooter (arsenal, ordnance, killcam,
   `sim::shooter`, the objective modes) is engine by design per ADR 2026-09-30-killchain-loadout-shooter, and stays unless no second game would use it.

**Alongside Phase 1,** each a small PR once item 1 lands: an AGENTS.md byte budget enforced by a test like `describe --brief`'s, split into a game-maker page and an
engine-maintainer page, without the SPEC.md contradiction; a `preflight` file-length check (an allowlist of today's 28, a plan to split red2d `game.rs`, red2d
`sim.rs` and `net/protocol.rs`); the ADR bar back to "a decision a future reader would otherwise re-derive", the rest in `status --note`; the stale claims
(ENGINE_OVERVIEW, `symbols.rs`, unused `thiserror`, `macros.rs`'s name).

**Phase 2, the rules language** (the evidence's order):
0. A lint for more than 16 replicated vars (or a deliberately raised cap, with a protocol bump).
1. Per-entity variables, then restartable timers (`no-per-entity-timers`, 5 runs).
2. The fall-out height.
3. Rules that change movement, gravity and scale, and a body swap (Prop Hunt's hider becomes a prop).
4. 3D string tables (the 2D design).
5. 3D spawn/destroy, only if items 1-3 or the acceptance games show the need.

**Done:** Gentle Carousel rewritten in well under 100 rules with its board intact (rules, bytes, verify seconds, before and after), and a Prop Hunt that needs
no engine Rust: the PR that adds it touches no file under `src/` or `crates/`.

**Phase 3, uniqueness as the cheap path:** shape ops (lathe, extrude, array along a path), procedural materials and palette roles for the hex colours; `catalog
--template <name>`; the `own_prefab` recipe and the `style-own` lint; scaffold text that says "make three prefabs of your own first; the catalogue is filler";
and an ADR deciding whether a checked, licensed, size-capped mesh/texture import supersedes ADR 0008 (an idea only: nothing is imported from another
engine). **Done:** in a re-run of the agent-loop trace an own prefab costs at most twice a library prop, and two Idea Forge games in a row each have three or
more own prefabs, each uses the shared catalogue for under 30% of what it places, and the owner judges the published screenshots unlike each other.

## Consequences
Phase 1 is the expensive part, and it makes everything after it cheaper: a doc PR stops recompiling, a fresh checkout stops waiting minutes, a sim edit stops
relinking 79 test binaries. Existing games keep working through every move (parity runs; Frameless saves and Killchain's protocol are explicit gates).
S7 (the offline client through `MatchSim`) stays parked; the `GameMode` seam is designed so it can follow. Phases 2 and 3 are behaviour changes and can
interleave with Phase 1 only where they touch no crate boundary. To change the order, edit this ADR and STATUS.md Next together. Status becomes accepted
when the owner approves the plan.
