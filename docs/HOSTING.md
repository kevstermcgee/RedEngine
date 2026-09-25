# Hosting a Red server

The dedicated server (`red_server`) is one small headless binary: no window, GPU or audio libraries (`--no-default-features`, ADR 0017).
It speaks UDP on one port (default **27015**) and is configured by flags or environment variables (`RED_MAP`, `RED_PORT`, `RED_BIND`,
`RED_SPAWN_GROUP`, `RED_TIMEOUT_MS`, `RED_STATS_SECS`, `RED_RUN_FOR`; a flag beats a variable). It stops cleanly on Ctrl-C and on
SIGTERM (`docker stop`, systemd): clients are told, and a `--record` trace is written.

Not sure your machine can do it? `red_engine2 doctor` probes UDP loopback, the default port, and the output directory.

## Pick one

| Where | How |
|---|---|
| Any machine with Docker | `docker compose up --build` (maps from `./maps`, `RED_MAP=/maps/main.json`), or `docker build -t red-server . && docker run --rm -p 27015:27015/udp red-server` |
| A Linux box or VPS | build headless: `cargo build --release --no-default-features --bin red_server`; install to `/usr/local/bin`; copy `deploy/red-server.service` to `/etc/systemd/system/`, `deploy/server.env.example` to `/etc/red/server.env`, then `systemctl enable --now red-server` |
| Your own PC (Windows/macOS/Linux) | `scripts/dev server maps/main.json` (or `scripts/red serve` in a game project) |
| A game project | `scripts/red serve`: the map, port and spawn group come from `game.json` |

The server needs only the map JSON: ship it next to the binary (or bake it into your image).
Clients must have the same map: their hello carries a hash of it and the server rejects a mismatch (`WrongMap`). The hash ignores
line endings, so a Windows and a Linux checkout of the same map agree.

## Networking

* Allow **UDP** 27015 in the machine's firewall (`ufw allow 27015/udp`) and, in the cloud, the provider's security group.
* At home, either forward UDP 27015 on the router to the server, or skip port forwarding entirely with a private network
  (Tailscale/WireGuard/ZeroTier): players join `re2 --connect <tailscale-ip>:27015`. **This is the recommended way today** because
  of the next point.
* **The wire protocol is not encrypted or authenticated yet** (rate limits, size limits, per-address token checks and hostile-packet
  tests exist, `tests/net_abuse.rs`, but there is no key exchange). Do not expose a server to strangers on the open internet expecting
  privacy; use a private network for friends. Adding authenticated encryption is the top item in
  `docs/analysis/2026-09-24-cheddar-feedback.md` (the Cheddar fork already prototyped a passkey-derived scheme).
* Test reachability from *outside* your network (a phone hotspot is enough): a connection from the server machine to its own public
  address proves nothing about the router. `red_bot --server HOST:PORT --behavior forward:0 --duration 3` is a fine probe.

## Watching it

`RED_STATS_SECS=30` prints a line per 30 s: players, tick time, props promoted, in/out KB/s, snapshots, bad packets. A healthy small
match is a few KB/s and tens of microseconds per tick (`tests/net_budget.rs` and `tests/alloc_budget.rs` hold the budgets).
