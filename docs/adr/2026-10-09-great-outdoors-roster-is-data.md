# 2026-10-09. The Great Outdoors driver roster is data
Status: accepted
Summary: The eight drivers' names, abilities and kart numbers live in assets/data/drivers.json, parsed once and checked by a test that they equal the table they replaced bit for bit; the Driver enum remains the wire id.

## Context
The simplification pass (STATUS.md, stage S5) wanted the Great Outdoors roster as data. `Driver::spec()` and `Driver::name()` in `src/sim/kart.rs` were two `match`es over eight variants: the numbers a game designer tunes (top speed, grip, mass, drift rate, surface multipliers) lived in Rust, so tuning a kart or adding a driver's numbers meant a rebuild and reading the engine.

## Decision
- `assets/data/drivers.json` holds the roster: a `base` kart and, per driver in **wire order**, a `name`, an `ability` word and only the numbers that differ from the base. `Roster::parse` reads it (every key checked, an unknown one is an error, names unique, the speeds and mass above 0, exactly one entry per `Driver`) with errors naming the entry and field; it is parsed once, embedded with `include_str!`, and a unit test parses it, so a broken file fails the build's tests, never a running server (the simulation has no panics: an unreadable file would yield the base kart named `?`).
- `Driver` stays an enum: it is the wire id (`Driver::wire`, `from_wire`, `ALL`), and protocol v15 is untouched. `Ability` stays code (what bumping, drafting and building do is behaviour) and is chosen by name in the data.
- Parity, measured: a test keeps the old table as the expected values and compares every field of every driver **bit for bit**; and an 8-bot `race-test` of the generated Great Outdoors circuit (all eight animals, 113 s of game time, 24 pickups, 7 hits) prints a **byte-identical** report before and after.

## Consequences
Tuning a driver is a JSON edit plus `cargo test`. What it does not do yet: a game cannot bring its own roster (the count and the abilities are still the engine's eight) and `MatchSim` still assigns drivers by wire id as before; both would be a protocol decision (the roster size is a wire constant) and are left out.
