# Tidewatch (the reference game for narrative content)

Two night watches on a tidal island. Odile Marr keeps the signal brazier; Bram Teller waits on the dock for his father's boat, the Wren. You
carry peat, one brick at a time, from the shed to the brazier while it burns. A boat comes in at the end of a watch if the light was not dark
too long. Bram wants two bricks for the dock lamp, and what you answer — lend, refuse, promise "after the watch", or ask why he is afraid of
Odile — moves real peat, changes who knows what, and is remembered a watch later. It exists to show and test narrative content on the
engine's public API (ADR 2026-10-15-narrative-is-compiled-content-run-by-one-story-system, `docs/STORY.md`). It is not part of the release.

```bash
cargo run -p tidewatch                                       # play: WASD / left stick to walk, E / Space / A to talk and to go on,
                                                             # W/S, the D-pad or 1-9 to choose, Backspace / B to leave, F5 saves, Esc saves and quits
red_engine2 story check games/tidewatch/content/story        # the story: parse, bind, type-check, lint (no game binary needed)
red_engine2 story build games/tidewatch/content/story        # compile it into content/story/tidewatch.story.json (milliseconds, no Rust)
cargo test -p tidewatch_server -p tidewatch                  # the story under the real match, the client, saves, replay, the pictures
target/debug/tidewatch_server cli sim games/tidewatch/content/tidewatch.json --inspect story          # the scenarios, and the story's log
target/debug/tidewatch_server cli story walk games/tidewatch/content/tidewatch.json games/tidewatch/content/story/walks.json
target/debug/tidewatch_server story-bindings > games/tidewatch/content/story/bindings.json             # after changing the watch's API
```

| path | what it is |
|---|---|
| `content/tidewatch.json` | the island (shed, brazier, bell, dock, lamp), its zones and rules (`at_shed`, `near_bram`, ...), HUD, `checks.sim`, the `tidewatch` block |
| `content/story/people.story` | Odile's and Bram's sheets, the story's facts, what can be known, the setups and their payoffs |
| `content/story/tidewatch.story` | the conversations (Bram's request, his after-the-watch callback, Odile), the barks and notices, the pacing director |
| `content/story/tidewatch.story.json` | the compiled package the game loads (`story build`; a test fails when it is stale) |
| `content/story/bindings.json` | what the watch offers a story: queries, commands (the director's one unattended), events (`story-bindings`; kept fresh by a test) |
| `content/story/walks.json` | the story-walks: Bram's request on every branch, and the promise kept or broken |
| `server/src/watch.rs` | the gameplay system `tw:watch`: peat in ledger stores, the brazier, the watches; typed queries, commands, events |
| `server/src/game.rs` | Tidewatch as the match's game: the watch and the story in one systems host; zone variables into commands; intentions to the story |
| `server/src/content.rs` | the scene block: loads the package and binds it to the watch and the scene's variables (refused, `file:line` first, if it does not fit) |
| `server/tests/story.rs` | every behaviour the brief asks for, through the real match (19 tests) |
| `src/lib.rs`, `tests/play.rs` | the client: third-person camera, the engine's optional presenter, saves that resume where you stood; offscreen pictures in `out/` |

**What it decides:** the world does not pause while you talk (the brazier keeps burning) but you stand still; barks never stop play; a bark
that matters (the light going out) interrupts a talk with Bram, which resumes after it. **What it proves** and how is listed in
`docs/analysis/2026-10-15-narrative-foundation.md`.
