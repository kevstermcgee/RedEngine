# 2026-10-15. A game system's tick runs on interned ids and ledger slots
Status: accepted
Summary: An `Id` is an interned, `Copy` handle to text kept once per process (ordered by its text, so maps, hashes and saves are unchanged); the ledger keeps stores and resources as slots with an in-place transaction and an undo log, and a system resolves its ids to slots once, so a steady-state tick allocates nothing and a refusal is words only when read

## Context
The game-systems milestone (ADR 2026-10-14-game-systems-compose-under-one-match-authority, docs/analysis/2026-10-14-game-systems-milestone.md)
measured Workshop at N = 1000 machines: p95 tick 1.73 ms, about half of it `memcmp` (string-keyed map lookups of ids) and `malloc`/`free` (ids
cloned, store ids formatted and parsed, transactions and their refusals built, every tick). Roadmap item 1 ("indexed runtime data") was pulled
before any second game is written against `System`, while the trait has two users (Workshop and `tests/systems_abilities.rs`). Its acceptance:
p95 at most a third of 1.73 ms, the same systems hash over 3000 ticks of both shipped scenes, zero allocations per steady-state tick, and
`describe systems` and the content files unchanged.

## Decision
- **`Id` is interned.** `Id::parse` validates the text and returns a `Copy` handle to the one copy kept for the process; equality and hashing
  are by that address, ordering is by the text. Maps keyed by ids therefore iterate exactly as when an id was a `String`, which keeps every
  hash, save and wire order. Each distinct id is kept for the life of the process: ids come from content and saves, both bounded. Text that
  may name nothing (an inspection target, anything a peer sends) is looked up with `Id::find`, which never adds one. The handle is the
  text's address rather than a `u32` index (the roadmap's wording): `as_str` and ordering then need no table lookup and no lock.
- **The ledger keeps slots.** Stores and declared resources and reasons are dense slots. A store's slot is stable for the ledger's life
  (through removal, re-adding and `restore`), so a system may resolve once (`Ledger::key`, `res`, `why`) and keep the slots. `apply_ops` runs
  `SlotOp`s in place with an undo log it reuses; `check_ops` reads through a small overlay. A refusal is a `Fail` (which op, what about it);
  `Ledger::explain` writes the same `Diagnostic` as before when someone reads it. The id-keyed `Tx`/`apply`/`check` API stays, resolved to
  slots first (`try_apply` skips the words). Saved state (`LedgerState`) and `Ledger::hash` are unchanged byte for byte; a store is read
  through `StoreRef` (`owner()`, `capacity()`, `items()`, `to_store()`).
- **A system resolves once and keeps scratch.** Workshop's machines resolve each unit to slots on the first tick and again after a command or
  a load, and keep their transaction and drain buffers between ticks; the flow systems keep a `Tx` and use `try_apply`.

## Consequences
- Measured (docs/analysis/2026-10-15-interned-ids-measured.md, Intel N97): N = 1000 p95 1.58 ms on the string version the same day, 18 us
  after; 19,539 allocations per tick to 0 after warm-up; the golden hashes (`games/workshop/server/tests/golden.rs`) and 190 KB of
  `--inspect --json` output identical.
- A system that builds `Tx`s and calls `apply` every tick still works, as fast as before: slots are the path for a hot loop, not a requirement.
- Ids leak by design. A server that interned text from untrusted peers would grow without bound; `Id::find` is the rule for such text.
- Not done here: slot-indexed system state (Workshop's units stay a `BTreeMap<Id, Unit>`; the tick walks it beside the resolved slots),
  replication deltas (roadmap item 2), sleeping blocked machines (item 3).
