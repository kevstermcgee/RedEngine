# Ledger property tests (2026-10-14): what they cover and what they cost

Review follow-up item 2 (roadmap item 6, pulled forward). `crates/red_core/src/sim/ledger_props.rs` runs seeded random sequences against a plain
model of what `sim::ledger` promises, through the ledger's public API only.

## What one run does

300 seeds x 200 steps = 60,000 steps. Each step is one of: a transaction (70%; one op, or two to four a quarter of the time) that is `check`ed,
then `apply`d; `add_store` (10%, duplicates included); `remove_stores` of one to three stores into another (20%: unknown, repeated, self,
missing `into`, not enough room). Ops are moves, consumptions and productions over six store names (three exist at the start; capacities 12, 6
and 0), two declared resources and one never declared, three declared reasons and four undeclared (an unknown name, or a declared name of the
wrong kind: `Source(ship)`, `Sink(mine)`), amounts 0-3 mostly and up to 13 (past every capacity) one time in six.

After every step: the result and the error code equal the model's; the stores and totals equal the model's; `audit()` passes; a refused
`apply`, `add_store` or `remove_stores` leaves `state()` and `hash()` unchanged; `check` changes nothing and agrees with `apply` (so `apply`
after `check` Ok always succeeds). After each sequence, `state()` restores exactly into a fresh ledger with the same declarations. A failure
names its seed and step. The generator is splitmix64 written in the test (no crate; same sequences on every platform).

## Measured (ubuntu-pc, Intel N97, 4 cores; dev profile; `cargo test -p red_core --lib ledger_props`)

| | |
|---|---|
| steps accepted / refused | 12,666 / 47,334 |
| `apply` after a successful `check` | 9,235 (all succeeded) |
| test time | 0.47 s (three runs: 0.47, 0.47, 0.46) |

## Does it catch bugs

Two deliberate mutations of `ledger.rs`, each run once and reverted:
- a store accepts one unit over its capacity (`free() + 1 < n`): fails at seed 0 step 0 (`check` disagrees with the model);
- `remove_stores` applies one transaction per store (item 1's dismantle bug, at the ledger level): fails at seed 0 step 52.

## Not covered

Concurrency (the ledger is single-threaded by design); `restore` of tampered saves (unit test `a_save_restores_exactly_and_a_tampered_one_is_refused_without_change`);
the systems host (its command rollback has its own unit test, item 1).
