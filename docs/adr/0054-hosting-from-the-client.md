# 0054. Hosting from inside the client: `re2 --host`
Status: accepted
Summary: Hosting from inside the client: `re2 --host` serves the map on a thread of the game and joins it, so bots need no second process

## Context
A game built on the engine with bots for opponents is single-player in spirit but multiplayer in machinery: to play it you started `red_server`, waited for it, then started `re2 --connect`,
and remembered to stop the server afterwards. Two processes, a console window and a port are a lot to ask before the first shot, and a shortcut that does it for you has to manage a child
process it did not start (find it, hide it, kill it, survive the window being closed). The server is a library type that already runs on a thread in every network test.

## Decision
`net::host::LocalHost::start(map, options)` is the core of `red_server` for one map: the same `Server`, the match flow and bots that the map's own `match` and `bots` blocks ask for, interest
management, on the loopback (or any address the options give), with a free port unless one is named. It runs on a thread of the current process and stops, telling its clients, when it is
dropped. `re2 --host [--fill N] [--bot-skill LEVEL] MAP` starts one and joins it; the host is dropped after the client, so closing the game closes the server and nothing is left running.

`red_server` is unchanged (it has recording, UPnP, keys and signal handling that a game does not need); the two share the server, not the start-up.

**Pausing.** A match that belongs to one person can wait for them. `Server::set_pause_flag` gives another thread a switch that freezes the simulation and the match clock while the server keeps ticking, answering and sending snapshots, so clients stay connected (`LocalHost::pause_flag`). The pause menu (Escape) sets it, and so does losing focus in a hosted game; resuming clears it. The world, the bots, health regeneration and the round timer all stand still.

## Consequences
- One executable and one click start a game against bots: `re2.exe arena.json --host`. A game's launcher is a shortcut to that command.
- The map alone decides the fight (`bots`, `match`, `combat`, `weapons` blocks), and `--fill` / `--bot-skill` are the player's difficulty and crowd knobs.
- The server shares the process with the renderer: a slow frame does not slow the server thread (it sleeps and ticks on its own), and the loopback has no packet loss to hide bugs, so
  network behaviour is still proved by the dedicated-server tests, not by playing this way.
- Limits: loopback only unless `HostOptions::bind` says otherwise; no join key, recording or UPnP (use `red_server` to host for other machines, docs/HOSTING.md).
