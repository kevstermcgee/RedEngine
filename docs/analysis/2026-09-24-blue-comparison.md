# Comparison with BlueEngine, and what was built in response (2026-09-24)

Two outside reviews compared this repository with `kevstermcgee/BlueEngine`. Their summary: Red is stronger on map-design and AI-agent
tooling; Blue is stronger on shipping, hosting and the multiplayer front end. Two defects in Red were named: a stale README sentence
("no online multiplayer yet") and no LICENSE file. This note records what was fixed, what was built to beat Blue's version of each item,
and what was **not** done.

## Defects found in Red

| Finding | Fix |
|---|---|
| README "Known limits" said multiplayer did not exist; also "no 2-D UI" and a link to a sibling repo (`../forge3d`) that only exists on one machine | Rewritten; `tests/repo_hygiene.rs` bans those claims in README/SPEC/AGENTS/CLAUDE and checks every relative markdown link resolves inside the repo |
| No LICENSE | MIT `LICENSE` (same licence as Blue) and `license` in `Cargo.toml`; the hygiene test requires both. The holder is "Red Engine contributors": change it if you want a name |
| Found while building the replacements: remote players froze during a burst of lost snapshots, then snapped forward | Bounded, speed-aware extrapolation and a catch-up limit in `net::interp` (ADR 0034), guarded by `net-test` and unit tests |
| Found: parked (disconnected) players held their ids and could lock newcomers out | Soft reservations (`Server::free_slot`) |

## What Blue has, and what replaced it here

| Blue | Red now | Where it is better (and not) |
|---|---|---|
| Multiplayer template: menu, connect, lobby, ready, HUD with ping, rematch. Its screens are drawn but not wired (Connect flips a screen; Ready jumps to Playing) | The whole loop is real: connect form with masked key, lobby with roster / character / ready, countdown, timed rounds, results, rematch, late joiners, HUD with ping, timer, scoreboard (ADR 0029). Pure `sim::flow` state machine, state-based lobby messages that survive packet loss, screens audited at 9 window sizes, proven over real UDP (`tests/net_flow.rs`) and with a real window (`scripts/lobby_demo.ps1`) | Not built: spectator camera, teams, kick/ban, map voting |
| Join key sent inside a cleartext JSON `Hello`; "no encryption or cryptographic authentication" | Challenge/response (the key is never sent), stateless address cookies (no amplification), HMAC tag on every datagram, replay and forgery tests (ADR 0028) | Authentication only: **traffic is not encrypted**, and a short key can be guessed offline. Blue's template does have QUIC/TLS transport; Red does not |
| `blue_portmap.py`: hard-coded control URL, gateway guessed as `.1`, SSDP port used as the HTTP port | `net::upnp`: SSDP discovery, description parsing, service-version handling, ownership rules, permanent-lease refusal, lease renewal, CGNAT warning, `red_server --upnp` (ADR 0031) | Tested against an in-process fake router and SSDP responder, **not a real router**. No NAT-PMP/PCP/IPv6 |
| `net-test`: 120 ticks of a prediction buffer through a latency model in one process | `net-test`: real server + real clients behind a seeded bursty-loss proxy, five profiles, judged on what a player would notice (ADR 0034) | It found and fixed a real defect (above). Not simulated: bandwidth caps, NAT rebinding, hitscan lag compensation |
| `inspect-performance` / `validate-budget`: one hard-coded 2-player lab, a mean | `perf` / `checks.perf`: any scene, real walking players, p50/p95/p99/worst, best-of windows, bandwidth, datagram size, promoted props, advice, runs inside `verify` (ADR 0030) | Wall-clock budgets are machine-dependent; no GPU or memory measurement |
| `be2.py package`: zip + SHA-256 + commit (Python, cannot verify, wall-clock timestamps) | `package` / `package --verify`: reproducible bytes, dirty-tree refusal, symlink refusal, feature-isolated binaries, graphics-free check re-run at verify time (ADR 0032) | Not signed; the manifest records what was built, not that tests passed |
| `check_headless.py` | Already covered by `tests/headless_boundary.rs` and CI; now also enforced on the packaged binaries | |
| `FEATURES.json` (hand-written feature -> files -> checks) | `docs/features.json` + `features --check` (a test) + `impact` (changed files -> features -> dependents -> exact commands) (ADR 0033) | Dependencies are declared, not inferred |

## Not done (Blue has it, or a reviewer suggested it)

* Rust prototype API (`SceneBuilder` / prelude): Red stays JSON-first.
* `game-describe` / `game-schema` / JSON Schema files for maps and patches: `describe`, `validate` and strict field errors cover the same ground
  for the scene language, but no machine-readable JSON Schema is published.
* Lag compensation for hitscan, QUIC/TLS transport, spectator camera.
* A real-router UPnP run and a Linux/Docker run of the new tools (only Windows was exercised locally; CI has the Linux jobs).
