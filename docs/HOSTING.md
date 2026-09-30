# Hosting a Red server

The dedicated server (`red_server`) is one small headless binary: no window, GPU or audio libraries (`--no-default-features`, ADR 0017).
It speaks QUIC (UDP) on one port (default **27015**) and is configured by flags or environment variables (`RED_MAP`, `RED_PORT`, `RED_BIND`,
`RED_TLS_CERT`, `RED_TLS_KEY`, `RED_MAX_CONNECTIONS`,
`RED_SPAWN_GROUP`, `RED_TIMEOUT_MS`, `RED_STATS_SECS`, `RED_RUN_FOR`, `RED_KEY`, `RED_LOBBY`, `RED_UPNP`, `RED_MIN_PLAYERS`,
`RED_COUNTDOWN_SECS`, `RED_ROUND_SECS`, `RED_RESULTS_SECS`, `RED_SCORE_TO_WIN`; a flag beats a variable). It stops cleanly on Ctrl-C and on
SIGTERM (`docker stop`, systemd): clients are told, and a `--record` trace is written.

**Loopback by default.** With no `--bind`/`RED_BIND`, `--public` or `--upnp`, `red_server` listens on `127.0.0.1` only. That is what tests, bots, `play-local`, `net-test`, `perf` and `scripts/red serve` need, and a loopback socket never triggers the OS firewall prompt (a Windows "allow access?" dialog blocks an unattended run until someone clicks it). To host for other machines say so: `--public` (= `--bind 0.0.0.0`), `--bind IP`, or `--upnp` (implies public). The container image and the systemd unit set `RED_BIND=0.0.0.0` themselves. A client connecting to a loopback server also binds loopback (`NetClient`).

**Encrypted when it faces the network (ADR 0044).** A server reachable from other machines speaks QUIC + TLS 1.3 with the deployment's own
identity. Make it once, keep `key.pem` private and out of version control:

```bash
red_engine2 net-identity --out /etc/red/identity        # writes cert.pem + key.pem, prints the fingerprint
red_server --public --tls-cert /etc/red/identity/cert.pem --tls-key /etc/red/identity/key.pem --key auto
re2 --connect HOST:27015 --server-fingerprint sha256:... map.json      # players pin the printed fingerprint (or RE2_SERVER_FINGERPRINT)
```

The server prints its fingerprint at every start; give it to players with the join key. The fingerprint says *which server* (a client
refuses any other, and never falls back to plain UDP); the join key says *who may play*: they are separate. A CA-issued certificate works
too (`--server-ca ca.pem --server-name host`). Without an identity the server speaks *development UDP* (authenticated, not encrypted), which it
does only on loopback, or on another address with the explicit `--insecure-public-udp` (a trusted LAN); a public server with neither refuses
to start. Threat model, budgets and measurements: `docs/analysis/2026-09-27-transport-threat-model.md`.

Not sure your machine can do it? `red_engine2 doctor` probes UDP loopback, the default port, and the output directory.

## Pick one

| Where | How |
|---|---|
| Any machine with Docker | make the identity once: `docker compose run --rm --entrypoint red_engine2 red-server net-identity --out /identity`, then `docker compose up --build` (maps from `./maps`, `RED_MAP=/maps/main.json`; identity from `./identity`) |
| A Linux box or VPS | build headless: `cargo build --profile fast --no-default-features --bin red_server` (about half the rebuild time of `--release`; for up to 8 players the server tick is under 1 ms either way, see `benches/history/build-times.json`; `--release` is still what `package` ships); install `target/fast/red_server` to `/usr/local/bin`; copy `deploy/red-server.service` to `/etc/systemd/system/`, `deploy/server.env.example` to `/etc/red/server.env`, make `/etc/red/identity` with `red_engine2 net-identity` (key readable by the `red` user only), then `systemctl enable --now red-server` |
| Your own PC (Windows/macOS/Linux) | `scripts/dev server maps/main.json` (or `scripts/red serve` in a game project) |
| A game project | `scripts/red serve`: the map, port and spawn group come from `game.json` |

The server needs only the map JSON: ship it next to the binary (or bake it into your image).
Clients must have the same map: their hello carries a hash of it and the server rejects a mismatch (`WrongMap`). The hash ignores
line endings, so a Windows and a Linux checkout of the same map agree.

## One command on a Linux box

Every project made by `new-game` has an `install.sh` in its deploy folder (it reads `game.json`): it builds the headless server from the engine `game.json` names (profile `fast`), makes the QUIC
identity and a join key once, installs a per-user systemd service (no root; `loginctl enable-linger` keeps it up when logged out) and starts it.
`install.sh --info` prints how friends connect (address, fingerprint, join key), `--uninstall` removes the service and keeps the identity. The fingerprint is
public; the join key is private (it lives in `~/.config/<name>/server.env`, never in the project). Forward UDP 27015 for friends outside your network. Great Outdoors
was hosted this way and a QUIC client with the pinned fingerprint and key raced on it.

## Keys, lobby and rounds

* `--key SECRET` (or `RED_KEY`) makes joining need a key. Clients prove they know it (an HMAC challenge/response: the key is never sent) and every
  datagram after the handshake carries an authentication tag, so a stranger cannot join, inject input, kick a player or replay a
  captured packet (ADR 0028). `--key auto` invents a random 128-bit key and prints it: prefer it to a memorable word (a short key can be
  guessed offline by someone who recorded the handshake). Players type it into the connect form or pass `re2 --connect HOST:PORT --key K`.
* `--lobby` (or a `"match"` block in the map, see `red_engine2 describe scene`) turns on the lobby: players press Ready,
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
* **Encrypted and authenticated on QUIC.** Traffic is confidential, the server is verified by its fingerprint, and with a `--key` strangers
  cannot join (the proof is bound to the TLS connection). Connection limits, a bounded inbound queue, rate limits, size limits and
  hostile-packet tests (`tests/net_quic.rs`, `tests/net_abuse.rs`, `tests/net_auth.rs`) protect the server. On *development UDP* the traffic is
  readable and an open server authenticates only against blind attackers. Tested, not independently audited. No lag compensation for hitscan yet.
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
