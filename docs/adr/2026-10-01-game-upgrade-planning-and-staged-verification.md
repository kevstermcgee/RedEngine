# 2026-10-01. Game upgrade planning and staged verification
Status: accepted
Summary: A read-only upgrade planner matches known compatibility changes and writes a packet; staged verification builds the target engine in isolation and never writes over the real project

## Context
A game project (ADR 0024) pins its engine either by a local path (fast loop, follows `main`) or by an exact
commit (`game pin`, "develop on a path, ship on a pin"). Moving a published game to a newer pinned commit had no
dedicated workflow: the only tools were `game pin` itself (which only resolves and writes the engine reference, with
no view of what else might need to change) and `game check`/`scripts/red check` (which can only test the engine
already in use, not a candidate one). Nothing answered "is this specific project safe to move to commit X, what
breaks, and what do I need to fix" before someone just ran `game pin` and found out the hard way — especially for
a hand-edited map, where blindly rebuilding from a blueprint can discard an edit nobody meant to lose.

## Decision
- **`game upgrade plan`** (`tools::upgrade::run_plan`) is read-only: it loads `game.json` (`game::load`), probes the
  project's own Cargo manifests for a `red_engine2` dependency (`find_cargo_engine_refs`), classifies the project
  from that evidence alone (`classify`: `StandardBlueprint`, `CustomRustClient`, or `Unknown` with a reason — never
  guessed), resolves the requested target revision to one fixed commit against a reachable local checkout
  (`resolve_target`: refuses a dirty checkout, never fetches, never switches that checkout), and matches
  `docs/upgrade-migrations.json`'s records against real evidence (`applicable`; each migration id has its own small
  detector reading source at a specific commit via `git show`, never a generic text-search "proof"). The result is a
  packet (`Packet`/`packet_json`/`render_packet`) written to an explicit destination — exact baseline/target,
  requested fixes, migration verdicts, classified changes, relevant files, next checks and uncertainties, not the
  engine's changelog.
- **`game upgrade verify`** (`tools::upgrade::run_verify`) is the only part allowed to build or write staged content.
  It obtains the target engine from an identity-stamped cache (`ensure_target_build`/`BuildIdentity`: sha, toolchain,
  profile, features, lockfile hash — reused only when every field matches, so "do not rebuild identical engine
  binaries separately for every map" does not mean "trust any binary in a directory with the right name"), builds it
  (when not already cached) into its own isolated clone, never the engine's or an agent's working checkout, then
  copies the project into a scratch directory and runs the **target** binary's own `game build-all` there
  (`stage_and_build`) — the real project is read, never written. Each map's candidate is compared to the real,
  committed one with the existing semantic differ (`diff::diff`): identical is clean, anything else is a surfaced
  conflict, written only as a `.upgrade-candidate` artifact beside the real map, never over it. Both commands reuse
  `game::blueprint_drift` (extracted from `game::check` for exactly this), `game::check`, and the project's own
  `checks` to decide pass/fail — "blueprint compiled" is never treated as "the game still plays correctly."
- The migration registry (`docs/upgrade-migrations.json`) is a small sidecar, not a `game.json` schema change (the
  schema stays strict). It is seeded with two records already verified against this repo's real history: wire
  protocol version lockstep (`PROTOCOL_VERSION` bumped v8 through v14 across real commits; required for any
  networked game) and the growth of `verify`'s `checks` `GROUPS` (optional adoption). An id with no evidence either
  way stays `Possible`, never silently certified harmless.
- No `apply` command exists yet. Once `verify` is clean, the existing `game pin --engine <checkout>` performs the
  real engine-reference switch, and clean regenerated maps are copied in by hand; a conflicting map stays exactly
  where review requires deliberate attention.

## Consequences
Planning never risks the project or an engine checkout, so it is safe to run speculatively, repeatedly, without
network access. Verification's one real cost (building the target engine) is paid once per distinct identity and
reused across every project and map that upgrades to it. What is not done yet: an `apply` command that performs the
pin and the clean-map copy itself (deliberately deferred — "require deliberate application"); a three-way map
merger for conflicts (the packet surfaces them, a person or a follow-up tool resolves them); migration detectors
beyond the two seeded here (new ones are small, per-id functions in `tools::upgrade::applicable`, not a generic
rule engine). To undo: delete `tools::upgrade`, the `GameCmd::Upgrade`/`GameUpgradeCmd` variants, and
`docs/upgrade-migrations.json`; `game pin`/`game check` and every scene the upgrade touched are unaffected.
