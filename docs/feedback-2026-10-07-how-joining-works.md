# Feedback: how a Killchain join actually travels, and what is and is not proven (2026-10-07)

> **Correction, 2026-10-20 (the text below is left as it was written).** Two statements in it are wrong.
> 1. *"UPnP is not answered."* The router (a TP-Link Archer A7 at 192.168.0.1) does answer UPnP. `deploy/red-portmap.py` keeps UDP 28016 (relay) and 27015 (game) mapped to
>    the box with **permanent** mappings. Its 2026-10-08 first version asked for one-hour leases, but this router ignores an identical re-add, so a lease was never extended
>    and each mapping lapsed for about 40 s every hour (measured and fixed 2026-10-20; `docs/HOSTING.md`, "Operating the box").
> 2. *"The relay answers on its public name from inside: No (no NAT loopback on the router)."* **The router does hairpin.** A Resolve sent to `red-engine.duckdns.org:28016`
>    from the box itself reaches the relay and is answered, and a QUIC Version Negotiation probe to the same name on UDP 27015 is answered by the game server
>    (`deploy/red-selfcheck.py`, checks `public` and `game`, run every five minutes since 2026-10-20). A missing hairpin therefore cannot explain the 2026-10-07 "never answered
>    Register"; no UPnP mapping had been opened yet then (`red-portmap` dates from 2026-10-08). So a test from the box to its own public name does exercise DNS, the router mapping and
>    the relay or the server. What it cannot show is a friend's own ISP path: that is what an outside probe is for (the one on branch `net-probe`, run from a GitHub runner).
>
> The cousin joins the official server with the code on line 2 of `~/.config/killchain/join-code.txt`.

Filed for the review later tonight, beside `docs/feedback-2026-10-07-stabilization-brief.md` (read that first; this note is the evidence behind its multiplayer items).
Written by the Killchain session after the user asked "how does all of this work technically?" and said they once served a game from a different engine off the same mini PC
over UDP, flawlessly. It is observation, not instruction, and it separates what was tested from what was assumed.

## The road a join takes

1. **Name.** The joiner's game resolves `red-engine.duckdns.org`, which the box keeps pointed at the home's public address (`duckdns-update.timer`). Checked: the name resolves to the
   address `api.ipify.org` reports, so the home has a public IPv4 address (not carrier-grade NAT).
2. **Router.** The router must send the chosen UDP port to the box. UPnP is not answered (`red_engine2 portmap status`: "no router answered UPnP discovery"), so this is a manual
   forward or it does not exist. The engine has no way to tell the owner whether it exists.
3. **Firewall on the box.** `ufw` is active (`systemctl is-active ufw`). The rules file is root-only, but the owner's shell history shows `sudo ufw allow 27015/udp`, so UDP 27015 is allowed.
   UDP 27016 and 28016 are not known to be.
4. **The server.** `red_server` (here the user unit `red-game@killchain`) listens on `0.0.0.0:<port>`. Routers and firewalls see only "UDP, this port, this machine"; they cannot tell one
   engine's traffic from another's. That is why the owner's earlier game worked: nothing about the road depends on the game.
5. **The handshake (RedEngine specific).** QUIC (UDP with TLS 1.3). The long join code is `HOST:PORT#sha256:<fingerprint>#<key>`: the fingerprint pins the server's identity, the key is
   the game-level admission (an HMAC proof in the `Hello`), and the map hash and protocol version must match or the server refuses (`WrongMap`, `Version`).
6. **The game.** The server ticks at 60 Hz and is authoritative; each client predicts its own movement and corrects from snapshots.

The **relay** (`red_relay`, UDP 28016 on the box) is the alternative road for hosts that cannot be reached directly: host and joiner both connect out to it, and it forwards ciphertext. It
needs its own port reachable and is not a free lunch (brief item 1: the short-code JOIN supplies no game key to a keyed HOST).

## What was verified on the box, and what was not

| Claim | Evidence |
|---|---|
| The Killchain server runs, is sandboxed, restarts cleanly, enabled at boot | `systemctl --user show red-game@killchain`, 0 restarts, `default.target.wants` link |
| A QUIC client with the key joins on loopback and on the LAN address (192.168.0.133) | bot joins, round started, clean leave, in the unit's journal |
| A join with no key or a wrong key is refused | `NeedsKey`, `BadKey` |
| The real client launched on a different map joins and is matched to the right map | scripted client on Quarry joined the Ironworks server by long code; screenshot of Ironworks |
| The relay process answers on loopback | `red_server --relay 127.0.0.1:28016` printed a code |
| The relay answers on its public name from inside | **No**: "never answered Register" (no NAT loopback on the router, or the port is not forwarded; indistinguishable from inside) |
| UDP 27015 is open on the router | **Not verified.** Assumed from the owner's history of serving a game on this box and the `ufw` rule |
| A friend in another state can join | **Not verified.** Nobody outside the home network has tried |

Things that would have settled it cheaply and that the engine does not offer: a reachability check run from outside (see the stabilization brief, item 1); the router's forward target
address (the box has two DHCP addresses, 192.168.0.133 on ethernet and .211 on wifi, so a forward can silently point at the wrong one).

## State left on the box (so a later session is not surprised)

- `red-game@killchain` on UDP 27015 (moved from 27016 at the owner's choice, because they will not change the router and 27015 is the port that was already open).
- `great-outdoors.service` is **stopped and disabled on purpose** to free 27015. Files and identity are untouched. `killchain-port 27016` reverses it.
- `ufw` is active; Killchain's join codes (with the key) are in `~/.config/killchain/join-code.txt`, mode 600.

## For the engine

- A **port-forward-agnostic path that does not need the owner to trust a router** would remove the whole class of "it works here and not for the cousin": either the relay made reliable for
  keyed hosts (brief item 1), or a documented, tested overlay (Tailscale was offered to the owner as the no-router option).
- **`net-check`**: a command that, given a server or relay address, reports from a hosted probe whether UDP reaches it, and says what to fix (forward, firewall, CGNAT). Today the only test
  is asking a friend.
- When a host starts, the log should say which of **loopback, LAN, and public** addresses it believes are reachable and why, instead of printing a join line that may not work.
- The documentation said the short code needs no setup; the code shows otherwise for the default HOST. Documentation and capability claims should be tested like code.
