# 2026-10-18. A game started by a player opens in fullscreen and plays the RedEngine Games title card
Status: accepted
Summary: When `RE2_PLAYER=1` is set, as a game's `Play-<slug>.exe` and the RedEngine Games app set it, both players (`re2d`/`play2d` and the 3D client) open in borderless fullscreen and first play a 2.6-second "RedEngine Games" title card (`ui::title_card`); `--fullscreen` and `--intro` ask for the same by hand.

## Context
The owner wants every published game to open in full screen by default and to begin with a short title screen that says "RedEngine Games". The same engine binaries also serve development: people run scenes and games by hand, and scripts and tests capture pictures and step games headless. A fullscreen window or a title card in those runs gets in the way and shifts timings.

A launch argument was considered. Published games pass their arguments through `launch.args`, but those are built against whatever engine revision the release catalog pins, and an older `re2d` would read an unknown `--fullscreen` as the game's path. Unknown environment variables, by contrast, are ignored by every engine version.

## Decision
`ui::title_card::for_player` reads `RE2_PLAYER=1`. RedEngineGames' `Play-<slug>.exe` (`distribution/play_launcher.rs`) and the app (`launcher/core/src/launch.rs`) set it when they start a game. When it is set:
- `re2d` and `red_engine2 play2d` open borderless fullscreen and play the card before the first tick. The game waits, and a key or click skips the card.
- The 3D client (`re2`, and any game binary on `client::run`) implies `--fullscreen --intro`. `App::start_intro` begins the card after the scene loads. `intro_frame` draws it with an `Overlay` instead of the frame, and nothing updates until it ends. Any key except F11, or a click, skips it. `end_intro` makes every overlay painter repaint.

`TitleCard` paints the card on the CPU: "Red**Engine** Games" in the site's palette and the system's bold sans (Segoe UI Bold on Windows, `Serif::sans_bold`), with a soft glow behind it. The name fades in and rises, a red line grows under it, and the card fades to black. It is at most 1920 px wide, and bigger windows scale it up. Headless runs never open a window, so they never show the card.

## Consequences
- Players see the same start in every game. Development and automated runs are unchanged unless they set the variable or pass the flags.
- Existing installs get the card only when a game is rebuilt on an engine that has it. RedEngineGames releases follow the engine revision its catalog pins, so the games need a new release after the pin moves.
- A player who wants a window presses F11.
- To undo, stop setting the variable in the two launchers, or drop `for_player` from the defaults in `re2d`, `play2d` and `client::parse_args`.
