# 2026-09-30. red_engine2 servers manages the game servers on this machine
Status: accepted
Summary: A thin, Linux-only layer over per-user systemd units whose ExecStart is red_server: list, status, start, stop, restart, logs, with player counts from the server's own stats line, a --yes gate when players are connected, and no secrets printed.

## Context
Hosted games run as per-user systemd units (`deploy/game-host.sh` writes one per game; on the dev box `great-outdoors.service` has run for over 14 hours with `Restart=on-failure`, a memory cap and start at
boot), and a `red-server.service` template exists. Seeing what is running, and turning servers on and off, meant `systemctl --user` and `journalctl --user` by hand, and nothing showed whether a stop would
disconnect players. The owner wanted a small utility inside this repository, a CLI now and possibly a GUI later, on one machine, Linux only, and left open whether it should wrap `game-host.sh`.

## Decision
- **A subcommand, `red_engine2 servers`**, not a new binary: no `Cargo.toml` change, `--json` and the self-description for free, instant from the prebuilt binary, and it builds in the headless configuration.
- **A thin layer over systemd, not a supervisor.** systemd already restarts, caps and boots servers. A *server* is a user unit whose `ExecStart` is `red_server` (discovered from `systemctl --user show`; no registry file).
  Stray `red_server` processes (no unit) are listed and can be stopped by pid, not restarted.
- **The logic is a pure function of a `Backend` trait** (`tools::servers`: systemd, the process table, the journal), so tests use a fake machine and cannot touch a real server; `cli/servers.rs` only renders structs or JSON, so a GUI can reuse them.
- **Player counts from the server's own `stats:` log line** (`RED_STATS_SECS`), with its age; older than three intervals or absent is *unknown* (`?`), never `0`. No protocol change, no admin endpoint.
- **Safety:** stop/restart of a server that freshly reports players needs `--yes`; an unknown count warns and proceeds (a server without stats could otherwise never be stopped); starting on a UDP port another running server
  serves is refused (both units on the dev box default to 27015); `--pid` only signals a stray `red_server` found by this tool, never a managed server or another program. `--wait N` makes `start`/`restart` return only once the unit is running (a new
  process, for restart) or fail.
- **Secrets:** values of `*KEY*`, `*SECRET*`, `*TOKEN*`, `*PASSWORD*`-named variables and options are masked in everything printed (logs included); only the inline `Environment=` (port, map, stats interval) is read, never `EnvironmentFile`.
- **It does not wrap `game-host.sh`.** That script *provisions*: it builds the binary (minutes), creates the QUIC identity and the private join key, and writes the unit. `servers` *operates* what exists; putting secret generation and long builds
  behind a management command would blur the two and widen what a typo can do. For testing, what matters is `--json`, exit codes, `--yes` and `--wait`, which it has.
- **Tests:** unit tests with a fake backend (discovery, secrets, names, the safety policy, `--wait`, ports, unmanaged pids, rendering), and `tests/servers_cli.rs` through the real binary, read-only and non-destructive (it lists, and tries to act on names
  and pids that do not exist); skipped where there is no systemd user session.

## Consequences
- One command shows every server with state, uptime, players, port, map, memory and start-at-boot, and controls it, on this box. Windows, macOS, Docker and several hosts are out of scope: a second `Backend` is the place to add them.
- The player count is as fresh as the stats interval (60 s on the dev box) and is only there if the unit sets `RED_STATS_SECS`; a real status query would need a server-side endpoint with authentication, deliberately left for later.
- Ephemeral test servers (`systemd-run --user` with a free port and a time limit) are the natural next step for testing and are not built yet.
