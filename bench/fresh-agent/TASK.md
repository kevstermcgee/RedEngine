# Task for a fresh agent (give this text, and nothing else about the engine)

You are in an empty working directory. The engine's command line is `red_engine2` (its path is in `$RED_ENGINE`). You may run it, read its output, and edit files in the game directory you
create. Use only what the command line tells you: do not open the engine's source.

Build a small **native game** called `star-dash` in `./star-dash/`:

1. A top-down game. The player (scene id `p`) collects 6 stars and wins when all 6 are collected. A countdown variable named `timeleft` starts at **25** seconds; when it reaches 0 before the stars
   are all collected the player loses. A drifting hazard that ends the game in a loss when touched is welcome but optional.
2. It remembers the player's best result between runs (a saved variable, shown on screen).
3. It plays with the keyboard and the mouse, has a sound effect and a restart button.
4. Prove it: a scripted playthrough that **wins** and one that **loses**.
5. Validate it and verify it.
6. Before changing anything else, copy the game file to `before.game2d.json`.
7. Then make **this one gameplay change**: the countdown starts at **15** seconds instead of 25 (and every check still passes).
8. Re-verify.

Run every command from inside `star-dash/`. Finish by printing the output of `red_engine2 verify star-dash.game2d.json`.
