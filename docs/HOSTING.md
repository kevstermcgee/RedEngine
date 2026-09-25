# Hosting a Red server

The dedicated server (`red_server`) is one small headless binary: no window, GPU or audio libraries (`--no-default-features`, ADR 0017).
It speaks UDP on one port (default **27015**) and is configured by flags or environment variables (`RED_MAP`, `RED_PORT`, `RED_BIND`,
`RED_SPAWN_GROUP`, `RED_TIMEOUT_MS`, `RED_STATS_SECS`, `RED_RUN_FOR`, `RED_KEY`, `RED_LOBBY`, `RED_UPNP`, `RED_MIN_PLAYERS`,
`RED_COUNTDOWN_SECS`, `RED_ROUND_SECS`, `RED_RESULTS_SECS`, `RED_SCORE_TO_WIN`; a flag beats a variable). It stops cleanly on Ctrl-C and on
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

## Keys, lobby and rounds

* `--key SECRET` (or `RED_KEY`) makes joining need a key. Clients prove they know it (an HMAC challenge/response: the key is never sent) and every
  datagram after the handshake carries an authentication tag, so a stranger cannot join, inject input, kick a player or replay a
  captured packet (ADR 0028). `--key auto` invents a random 128-bit key and prints it: prefer it to a memorable word (a short key can be
  guessed offline by someone who recorded the handshake). Players type it into the connect form or pass `re2 --connect HOST:PORT --key K`.
* `--lobby` (or a `"match"` block in the map, see `red_engine2 describe scene`) turns on the lobby: players choose a character and press Ready,
  a countdown starts when everyone is ready, the round runs for `round_secs` (or until a rule ends it or `score_to_win` is reached), the
  results show, and pressing Ready again is a rematch (ADR 0029). Without it the server is in open play: join = play.
* Try it: `powershell -File scripts/lobby_demo.ps1` (Windows) starts a keyed lobby server, a bot and a real graphical client and screenshots each stage.

## Networking

* Allow **UDP** 27015 in the machine's firewall (`ufw allow 27015/udp`) and, in the cloud, the provider's security group.
* At home, three ways, best first: a private network (Tailscale/WireGuard/ZeroTier: players join `re2 --connect <tailscale-ip>:27015`, nothing
  is opened to the internet); **UPnP** (`red_server --upnp`, or `red_engine2 portmap status|enable|remove|keep`) which asks your router to
  open UDP 27015 for this machine only, renews the lease while the server runs and removes it on exit, and prints the address to give a
  friend (ADR 0031: it refuses to touch a mapping that is not its own and warns when your ISP gives you a carrier-grade NAT address, which
  no mapping can fix; tested against a fake router, not a real one); or forwarding UDP 27015 by hand.
* **Authenticated, not encrypted.** With a `--key`, strangers cannot join and nobody can forge, inject or replay datagrams (ADR 0028), and
  rate limits, size limits and hostile-packet tests (`tests/net_abuse.rs`, `tests/net_auth.rs`) protect the server. The traffic itself is
  readable by anyone on the path, and an *open* server (no key) authenticates only against blind attackers. Do not send anything through it
  you would not say on a shared Wi-Fi. There is no lag compensation for hitscan yet.
* Test reachability from *outside* your network (a phone hotspot is enough): a connection from the server machine to its own public
  address proves nothing about the router. `red_bot --server HOST:PORT --behavior forward:0 --duration 3` is a fine probe.

## Proving it works and shipping it

* `red_engine2 net-test maps/main.json --profile bad` (or `all`) puts a real server and clients behind a lossy, laggy proxy and says whether the
  game is still playable (ADR 0034). `red_engine2 perf maps/main.json` measures tick time and bandwidth with N walking players against the map's
  `checks.perf` budget (ADR 0030).
* `red_engine2 package out/release.zip` builds the client and the headless server (in separate target directories, so no graphics crates can leak
  into the server), and writes one reproducible zip with a SHA-256 manifest and the exact commit; `package --verify out/release.zip` re-checks
  it anywhere (ADR 0032). Run `scripts/ci.sh` first: the manifest records what was built, not that tests passed.

## Watching it

`RED_STATS_SECS=30` prints a line per 30 s: players, tick time, props promoted, in/out KB/s, snapshots, bad packets. A healthy small
match is a few KB/s and tens of microseconds per tick (`tests/net_budget.rs` and `tests/alloc_budget.rs` hold the budgets).
