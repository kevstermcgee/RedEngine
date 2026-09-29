# 2026-09-29. Develop on a path, ship on a pin
Status: accepted
Summary: A game follows a local engine checkout while it is developed and is pinned to an exact engine commit when a version ships; game pin and game unpin switch between the two.

## Context
`game.json` can name the engine two ways (ADR 0024): `{"path": ...}`, a local checkout, or `{"git": URL, "ref": COMMIT}`. Measured on the 4-core dev/server box
(`benches/history/build-times.json`, run `2026-09-29-phase1-build-speed-experiments`): a path-pinned game shares that checkout's build, so it has no
engine build of its own and an engine edit is 4-5 s in the `fast` profile; a git-pinned game gets its own clone in `.red/engine` and a cold build of about
5 minutes per pinned commit. The other side of the trade: a path-pinned game silently follows `main`, so a wire-protocol bump or schema change can break it
or stop the server matching clients that were already published. RedEngineGames builds a game's Windows client from the cataloged engine commit, so the
shared server for that game has to be built from the same commit. Sharing one `CARGO_TARGET_DIR` across games on different commits is not a fix: the final
binaries overwrite each other and `scripts/red`'s staleness check would trust the wrong one. `sccache` 0.7.7 did not help either (no hits across target dirs).

## Decision
- **Develop on a path, ship on a pin.** While a game is built it follows a local engine checkout (fast loop, newest engine). When a version is released for
  others to install, `red_engine2 game pin` rewrites only the `engine` block of `game.json` to `{git, ref}` with the checkout's HEAD commit and `origin` URL
  (`tools::game::pin_text`, `engine_pin`). `game unpin <path>` goes back (`unpin_text`).
- **Only what a clone can reproduce.** `game pin` refuses a checkout with uncommitted tracked changes (`--allow-dirty` overrides) and a commit that no remote
  branch contains, so the pin can always be fetched. The checkout is `--engine`, else the project's `engine.path`, else the one the binary was built from.
- The command prints the consequence instead of hiding it: the release's clients and its server must both be built from that commit (`scripts/red serve`
  builds the pinned engine's server).

## Consequences
The cold build is paid once per release instead of once per game. A released game no longer changes under its players when `main` moves, and engine fixes
reach it only when it is re-pinned. Several released games on different protocol versions need one server build each. Nothing changes for existing projects
until someone runs `game pin`. Not done: a shared build cache across pinned clones (needs `scripts/red` to copy binaries into a per-project folder first), and
checking that the pinned commit's protocol version matches the clients already published.
