# Lantern Walk

A tiny dusk game on an endless procedural world: three lanterns lie in the meadow, carry each one to the shrine before night falls. It combines a generated world (`procgen`),
a day/night `clock`, a generated score and the nature ambience (`audio`), rule-driven play with a carried prop (`rules`), a declared `ui` with a start and end cards, and a hosted
match (`re2 examples/lantern_walk/lantern_walk.json --host`).

It was written as a fresh author would write it, and every mistake made on the way is a test (`tests/lantern_walk.rs`): a guessed prop height (lint names the real ground height), a
target on a lantern or inside a bush (`reach` says what covers it and where to stand), a zone at the wrong height, a misspelled audio assertion, a missing score file, a
two-player game built on carrying (split-screen guests cannot carry props). `red_engine2 verify examples/lantern_walk/lantern_walk.json` runs eleven checks: lint, four standing
places, three scripted playthroughs (one lantern, all three, night falling), the score's loudness and the night's soundscape.

Play with `--players 2` and the client tells you what a second player cannot do. Not published to RedEngineGames (add it to `games-publish.json` to ship it; `cargo test` then checks
that the download contains the score).
