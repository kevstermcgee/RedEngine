# 2026-10-05. A game's progress is filed under its id, not its folder
Status: accepted
Summary: game.json may carry an id: settings and saved variables are keyed by it instead of name plus folder hash, projects without one are unchanged, and a project that gains one has its old files copied forward once.

## Context
Settings and saved variables (`settings::key_for`) were keyed by the project's name plus a hash of its canonical directory (so two same-named games never share files). That makes the folder the game's identity: unpack a new ZIP somewhere else and a player's progress (Marcel's days lived) silently starts over. `RE2_SAVE_DIR` (installed games name their own save folder) fixed the shipped games; a project run from a folder, a copy, a move or a second checkout still loses its saves. Feedback item 4 of `docs/analysis/2026-10-04-feedback-from-the-marcel-split-screen-and-distribution-work.md` asked for an id in `game.json` and one save module for the three mechanisms.

## Decision
`game.json` takes an optional `id` (1 to 40 lowercase letters and digits in groups separated by single hyphens; `game::valid_id`). `key_for` returns it when present, so the same game in any folder is one game. Without an `id` nothing changes: the name-and-folder key is used exactly as before, so no existing project moves. `new-game` writes an `id` made from the name. A project that first gains an `id` keeps what it saved: `settings::adopt_legacy` (called at client start) copies `settings.json` and `vars.json` from the old directory-keyed place to the new one, once, never over an existing file.

Not done, on purpose: a shared module for the other two mechanisms. Killchain's `stats.rs` file is in a fixed per-user folder (`%APPDATA%\Killchain`), not keyed by path, so it never had this problem. The host identity directory holds the server's key pair, whose fingerprint players pin: its path must not move, so it stays where it is.

## Consequences
A game that sets an `id` keeps its progress across ZIP updates, moves and copies without help from the launcher; two different games must not share an id (the author's choice, as with a package name). `id` is rejected by an older engine's strict `game.json` parser, so a project pinned to an engine from before this change cannot carry one. To undo: remove the `id` line; the folder key returns (progress saved under the id stays in its own place).
