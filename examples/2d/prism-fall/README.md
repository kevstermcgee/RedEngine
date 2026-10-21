# Prism Fall

A focused Tetris-style game built entirely in RedEngine's native 2D format. The 10-by-20 charcoal well fills the window edge to edge, with no outer frame or side gutters. A 44-pixel strip above it holds score, hold and two tiny next-piece previews; instructions stay in the menus. Play Marathon for score, race to forty lines in Sprint, or practise in Zen, which keeps a steady speed and gives you a fresh well after a top-out.

The game uses a shuffled seven-bag, full clockwise and counter-clockwise SRS kick tables (including the I piece's distinct kicks), a ghost landing indicator, immediate movement with delayed auto repeat, soft and hard drops, a half-second lock delay with at most fifteen movement resets, and one hold per piece. Scores include singles, doubles, triples, four-line clears, three-corner T-spins, combos, back-to-back difficult clears and perfect clears. T-spin mini classification is not implemented. Marathon and Zen score records and the fastest completed Sprint are saved locally.

| Input | Action |
| --- | --- |
| Left / Right or A / D | Move; hold to repeat |
| Down or S | Soft drop |
| Up, W or E | Rotate clockwise |
| Q | Rotate counter-clockwise |
| Space, Z or J | Hard drop |
| C, Shift, X or K | Hold |
| Escape or P | Pause / resume |
| F or F11 | Fullscreen |
| Up / Down, Enter | Select a mode and start |
| 1 / 2 / 3 | Start Marathon / Sprint / Zen |
| Gamepad | D-pad/stick move, up rotates, A drops/selects, X/B/RT holds, Start pauses |

The optional quick keys appear in the pause menu. Press R there to restart the same mode, or M to choose another. Sound effects are synthesized inside the engine; no downloaded assets or external runtime are needed.

From a RedEngine checkout:

```sh
scripts/dev red play2d examples/2d/prism-fall.game2d.json
scripts/dev red verify examples/2d/prism-fall.game2d.json
node examples/2d/prism-fall/verify.js /path/to/red_engine2
node examples/2d/prism-fall/verify-controller.js /path/to/engine/build/debug/deps
```

To edit the game, change `build.js`, then run `node examples/2d/prism-fall/build.js`. The script writes the adjacent `.game2d.json` deterministically. `showcase.scenario.json` is a real input sequence that places forty pieces and clears thirteen lines; the generator includes it in the shipped checks. The separate regression runner creates temporary board fixtures and drives the real engine, checking all clear counts, compaction, SRS kicks, blocked rotation, hold limits, lock timing, scoring, top-out, Zen rescue, Sprint completion and 912 bag draws across twelve seeds.

Created against RedEngine main `91beedf852ecff044d21fe525a619eca0373d316` on 2026-10-21. The source-matched prebuilt has fingerprint `816dc4d644bc`. Native Linux keyboard input, pause/resume, fullscreen toggles and persistence were exercised under Xvfb. The controller regression runner compiles the engine's current pad_actions mapping and controller module directly and checks all four D-pad directions, diagonals, release and focus gating; the shipped scenarios also check a menu-to-gameplay controller-action session. The Windows installer is built by RedEngineGames' release workflow; no physical controller or Windows installation was tested in the development session.

Prism Fall is an independent falling-block game, without affiliation to the Tetris brand.
