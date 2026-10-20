#!/usr/bin/env python3
"""Outside-in reachability probe for a machine that hosts Red Engine games.

Everything about hosting has only ever been checked from inside the home network (loopback, the LAN, the router's own port-mapping table).
That says nothing about whether a friend in another state can reach the machine. This script is meant to run on a machine that is *not* on
that network (a GitHub-hosted runner is a real machine on the internet) and answers, per target, "did an outside packet get an answer".

Targets (each one is a separate JSON entry, none needs a secret):

  dns     the host name resolves (and to what: the last octet is masked, this output ends up in public CI logs).
  relay   the shared rendezvous relay (default UDP 28016): sends the relay's real Resolve request for a code that cannot exist (`0` is not in
          the code alphabet) and requires the well-formed CodeNotFound reply. Wire format: crates/red_core/src/net/relay.rs (tag 3 + 6 code
          bytes asks, tag 5 answers). No key, no state is created on the relay.
  game    a game server (default UDP 27015, QUIC + TLS 1.3, ALPN `red/8`, crates/red_core/src/net/quic.rs). Two steps: (1) a stateless QUIC
          Version Negotiation probe (an Initial with a reserved version: a live QUIC listener answers with the versions it speaks), then (2) a full
          QUIC/TLS handshake. The game's join key is checked later, inside the encrypted Hello, and is NOT needed for the handshake, so nothing
          here can join a match. The SHA-256 of the presented certificate (the same `sha256:<hex>` clients pin) is recorded: it is public.
  silent  a plain UDP datagram to a port nothing should listen on (default 27016). It must stay unanswered. It is a control, not a pass/fail
          check: silence looks the same whether a firewall dropped the packet or the port is closed, so it only documents "nothing answers".

For every UDP target the detail says which of three things happened, because they mean different things on the way in:
  reply    something answered.
  refused  an ICMP port-unreachable came back: the packet reached a machine (or the router) that has no listener on that port.
  silent   nothing came back within the timeout: dropped on the way (router with no mapping, a firewall) or no reply sent.

Output: one JSON object on stdout (progress goes to stderr). Exit 0 when every required target is ok, 1 when one is not, 2 on bad arguments.
Python standard library only; the QUIC handshake additionally needs `pip install aioquic==1.2.0` (the workflow does that), and without it the
`game` target reports that and still does its Version Negotiation step.

    python3 scripts/net_probe.py --host red-engine.duckdns.org
    python3 scripts/net_probe.py --host 127.0.0.1 --no-mask      # against the machine itself, to see the probe work
"""

import argparse
import hashlib
import json
import os
import platform
import select
import socket
import ssl
import sys
import time
import urllib.request

DEFAULT_HOST = "red-engine.duckdns.org"
RELAY_PORT = 28016
GAME_PORT = 27015
SILENT_PORT = 27016
ALPN = "red/8"

# Relay control messages (crates/red_core/src/net/relay.rs): tag byte, then the payload.
TAG_RESOLVE = 3
TAG_CODE_NOT_FOUND = 5
CODE_LEN = 6
# '0' is not in the code alphabet (ABCDEFGHJKMNPQRSTUVWXYZ23456789), so this can never be a live code: the answer is always CodeNotFound.
IMPOSSIBLE_CODE = b"000000"

DEFAULT_REQUIRED = ("dns", "relay", "game")


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def mask_ip(ip, octets_kept, enabled=True):
    """`203.0.113.45` -> `203.0.113.x` (keep 3) or `203.0.x.x` (keep 2); anything that is not a dotted IPv4 is returned as is."""
    parts = ip.split(".")
    if not enabled or len(parts) != 4:
        return ip
    return ".".join(parts[:octets_kept] + ["x"] * (4 - octets_kept))


def ms(seconds):
    return round(seconds * 1000.0, 1)


def result(ok, detail, rtt=None, **extra):
    out = {"ok": bool(ok), "rtt_ms": None if rtt is None else ms(rtt), "detail": detail}
    out.update(extra)
    return out


# ---------------------------------------------------------------------------------------------------------------------------------------
# UDP request/reply with the three outcomes spelled out


def udp_exchange(ip, port, payload, timeout, accept=None):
    """Sends `payload` from a fresh socket (a fresh source port every call) and waits for one reply.

    Returns (outcome, reply_bytes, seconds): outcome is "reply", "refused" (ICMP port unreachable) or "silent". `accept(reply)` may reject a stray
    datagram, in which case waiting continues until the timeout."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        sock.connect((ip, port))  # connected: only this peer's replies arrive, and an ICMP error surfaces as ConnectionRefusedError
        t0 = time.monotonic()
        sock.send(payload)
        deadline = t0 + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                return "silent", b"", timeout
            ready, _, _ = select.select([sock], [], [], left)
            if not ready:
                return "silent", b"", timeout
            try:
                data = sock.recv(4096)
            except ConnectionRefusedError:
                return "refused", b"", time.monotonic() - t0
            except OSError as e:  # EHOSTUNREACH / ENETUNREACH arrive the same way
                return "refused:" + e.__class__.__name__, b"", time.monotonic() - t0
            if accept is None or accept(data):
                return "reply", data, time.monotonic() - t0
    finally:
        sock.close()


# ---------------------------------------------------------------------------------------------------------------------------------------
# targets


def probe_dns(host, mask):
    t0 = time.monotonic()
    try:
        infos = socket.getaddrinfo(host, None, socket.AF_INET, socket.SOCK_DGRAM)
    except OSError as e:
        return result(False, f"{host} did not resolve to an IPv4 address: {e}"), None
    took = time.monotonic() - t0
    ips = sorted({i[4][0] for i in infos})
    shown = [mask_ip(i, 3, mask) for i in ips]
    return result(True, f"{host} -> {', '.join(shown)}", took, addresses=shown), ips[0]


def probe_relay(ip, port, tries, timeout):
    request = bytes([TAG_RESOLVE]) + IMPOSSIBLE_CODE
    attempts = []
    for n in range(1, tries + 1):
        outcome, data, took = udp_exchange(ip, port, request, timeout)
        attempts.append(outcome)
        log(f"  relay try {n}/{tries}: {outcome}" + (f" {data.hex()} in {ms(took)} ms" if data else ""))
        if outcome == "reply":
            if data == bytes([TAG_CODE_NOT_FOUND]):
                return result(True, f"relay answered a Resolve for an impossible code with CodeNotFound (try {n}/{tries})", took, port=port, attempts=attempts)
            return result(False, f"something answered on UDP {port} but not with CodeNotFound: {data[:16].hex()} ({len(data)} bytes)", took, port=port, attempts=attempts)
        time.sleep(0.5)
    return result(False, explain_no_reply(port, attempts, "relay"), None, port=port, attempts=attempts)


def explain_no_reply(port, attempts, what):
    refused = [a for a in attempts if a.startswith("refused")]
    if refused:
        return (f"no {what} reply on UDP {port}: ICMP unreachable came back ({refused[0]}), so the packet reached a machine or router "
                f"with no listener/mapping for that port")
    return (f"no {what} reply on UDP {port} after {len(attempts)} tries: the packet was dropped on the way in (no router mapping, a firewall) "
            f"or nothing answered; the host's firewall log shows which")


def version_negotiation_request():
    """A QUIC long-header packet with a reserved ('greased') version, padded to 1200 bytes. Returns (datagram, our_dcid, our_scid)."""
    dcid, scid = os.urandom(8), os.urandom(8)
    first = 0xC0 | (os.urandom(1)[0] & 0x3F)
    head = bytes([first]) + (0x0A1A2A3A).to_bytes(4, "big") + bytes([len(dcid)]) + dcid + bytes([len(scid)]) + scid
    return head + os.urandom(1200 - len(head)), dcid, scid


def parse_version_negotiation(data, our_dcid, our_scid):
    """The versions listed in a Version Negotiation packet addressed to us, or None when `data` is not one."""
    if len(data) < 7 or not data[0] & 0x80 or data[1:5] != b"\0\0\0\0":
        return None
    dl = data[5]
    dcid = data[6:6 + dl]
    if len(data) < 7 + dl:
        return None
    sl = data[6 + dl]
    scid = data[7 + dl:7 + dl + sl]
    if dcid != our_scid or scid != our_dcid:
        return None
    rest = data[7 + dl + sl:]
    return [int.from_bytes(rest[i:i + 4], "big") for i in range(0, len(rest) - 3, 4)]


def probe_quic_listener(ip, port, tries, timeout):
    """Step 1 of `game`: is a QUIC listener answering at all? Stateless on the server (it creates no connection)."""
    attempts = []
    for n in range(1, tries + 1):
        packet, dcid, scid = version_negotiation_request()
        outcome, data, took = udp_exchange(ip, port, packet, timeout, accept=lambda d: parse_version_negotiation(d, dcid, scid) is not None)
        attempts.append(outcome)
        log(f"  quic version-negotiation try {n}/{tries}: {outcome}")
        if outcome == "reply":
            versions = parse_version_negotiation(data, dcid, scid)
            return result(True, "a QUIC listener answered the Version Negotiation probe", took, versions=[f"0x{v:08x}" for v in versions], attempts=attempts)
        time.sleep(0.5)
    return result(False, explain_no_reply(port, attempts, "QUIC"), None, attempts=attempts)


def quic_handshake_once(ip, port, server_name, alpn, timeout):
    """One QUIC/TLS 1.3 handshake with no join key, driven by aioquic's sans-IO QuicConnection on a plain IPv4 UDP socket (so the runner's
    IPv6 support cannot matter). Returns a dict: ok, error, cert_sha256, alpn, handshake_ms, retry_seen."""
    from aioquic.quic.configuration import QuicConfiguration
    from aioquic.quic.connection import QuicConnection
    from aioquic.quic import events
    from cryptography.hazmat.primitives.serialization import Encoding

    config = QuicConfiguration(is_client=True, alpn_protocols=[alpn], verify_mode=ssl.CERT_NONE, server_name=server_name, idle_timeout=timeout)
    conn = QuicConnection(configuration=config)
    peer = (ip, port)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.connect(peer)
    out = {"ok": False, "error": None, "cert_sha256": None, "alpn": None, "handshake_ms": None, "icmp_refused": False}
    t0 = time.monotonic()
    deadline = t0 + timeout
    conn.connect(peer, now=t0)

    def flush():
        for data, _addr in conn.datagrams_to_send(now=time.monotonic()):
            try:
                sock.send(data)
            except OSError as e:
                out["error"] = f"send failed: {e}"
                return False
        return True

    try:
        done = False
        while not done:
            if not flush():
                return out
            now = time.monotonic()
            if now >= deadline:
                out["error"] = f"no completed handshake within {timeout:g} s"
                return out
            timer = conn.get_timer()
            wait = deadline - now if timer is None else max(0.0, min(deadline, timer) - now)
            ready, _, _ = select.select([sock], [], [], wait)
            if ready:
                try:
                    data = sock.recv(65535)
                except ConnectionRefusedError:
                    out["icmp_refused"] = True
                    out["error"] = "ICMP port unreachable: nothing listens on that UDP port (or a router rejected the packet)"
                    return out
                except OSError as e:
                    out["error"] = f"receive failed: {e}"
                    return out
                conn.receive_datagram(data, peer, now=time.monotonic())
            elif timer is not None and time.monotonic() >= timer:
                conn.handle_timer(now=time.monotonic())
            while True:
                event = conn.next_event()
                if event is None:
                    break
                if isinstance(event, events.HandshakeCompleted):
                    out["ok"] = True
                    out["handshake_ms"] = ms(time.monotonic() - t0)
                    out["alpn"] = event.alpn_protocol
                    cert = getattr(conn.tls, "_peer_certificate", None)
                    if cert is not None:
                        out["cert_sha256"] = "sha256:" + hashlib.sha256(cert.public_bytes(Encoding.DER)).hexdigest()
                    done = True
                elif isinstance(event, events.ConnectionTerminated):
                    out["error"] = f"the server closed the connection during the handshake: error 0x{event.error_code:x} {event.reason_phrase!r}".strip()
                    return out
        # Say goodbye properly so the server does not wait out its idle timeout for a connection that is already finished.
        conn.close(error_code=0, reason_phrase="net-probe")
        flush()
        return out
    finally:
        sock.close()


def probe_game(ip, port, server_name, tries, timeout, handshake_timeout, alpn):
    listener = probe_quic_listener(ip, port, tries, timeout)
    try:
        import aioquic  # noqa: F401
    except ImportError:
        return result(False, "aioquic is not installed (pip install aioquic==1.2.0), so no handshake was attempted; "
                      "Version Negotiation: " + listener["detail"], listener["rtt_ms"] and listener["rtt_ms"] / 1000.0, port=port, listener=listener, handshake=None)
    attempts = []
    last = None
    for n in range(1, tries + 1):
        try:
            last = quic_handshake_once(ip, port, server_name, alpn, handshake_timeout)
        except Exception as e:  # a probe must report, not crash
            last = {"ok": False, "error": f"{e.__class__.__name__}: {e}", "cert_sha256": None, "alpn": None, "handshake_ms": None}
        attempts.append("handshake" if last["ok"] else (last["error"] or "failed"))
        log(f"  quic handshake try {n}/{tries}: " + (f"completed in {last['handshake_ms']} ms, alpn={last['alpn']}" if last["ok"] else str(last["error"])))
        if last["ok"]:
            break
        if last.get("icmp_refused"):
            break
        time.sleep(0.5)
    if last and last["ok"]:
        return result(True, f"QUIC/TLS 1.3 handshake completed (ALPN {last['alpn']}, no join key sent)", (last["handshake_ms"] or 0) / 1000.0, port=port,
                      cert_sha256=last["cert_sha256"], alpn=last["alpn"], listener=listener, attempts=attempts)
    why = (last or {}).get("error") or "unknown"
    if listener["ok"]:
        detail = f"a QUIC listener answers but the handshake did not complete: {why}"
    else:
        detail = f"the handshake did not complete: {why}"
    return result(False, detail, None, port=port, cert_sha256=None, listener=listener, attempts=attempts)


def probe_silent(ip, port, timeout, tries=2):
    attempts = []
    for n in range(1, tries + 1):
        outcome, data, took = udp_exchange(ip, port, b"red-net-probe: nothing should answer this", timeout)
        attempts.append(outcome)
        log(f"  silent-port try {n}/{tries}: {outcome}")
        if outcome == "reply":
            return result(False, f"UNEXPECTED: something answers on UDP {port} ({len(data)} bytes)", took, port=port, attempts=attempts)
    if any(a.startswith("refused") for a in attempts):
        detail = f"no answer on UDP {port}; an ICMP port-unreachable came back, so the packet reached a machine with no listener there"
    else:
        detail = f"no answer on UDP {port} (silence: either nothing listens or the packet was dropped; this check cannot tell which)"
    return result(True, detail, None, port=port, attempts=attempts)


def runner_egress_ip(mask):
    """Best effort: the address this machine's traffic leaves from, masked to two octets (so a host's firewall log can be matched by
    network and time without publishing the runner's address). Never fails the probe."""
    for url in ("https://api.ipify.org", "https://ifconfig.me/ip"):
        try:
            with urllib.request.urlopen(url, timeout=5) as r:
                ip = r.read(64).decode().strip()
            if ip.count(".") == 3:
                return mask_ip(ip, 2, mask)
        except Exception:
            continue
    return None


# ---------------------------------------------------------------------------------------------------------------------------------------


def main(argv=None):
    ap = argparse.ArgumentParser(description="Outside-in reachability probe for Red Engine game hosting (see the module docstring).")
    ap.add_argument("--host", default=DEFAULT_HOST, help=f"host name or IPv4 address (default {DEFAULT_HOST})")
    ap.add_argument("--relay-port", type=int, default=RELAY_PORT)
    ap.add_argument("--game-port", type=int, default=GAME_PORT)
    ap.add_argument("--silent-port", type=int, default=SILENT_PORT, help="a UDP port nothing should answer on")
    ap.add_argument("--alpn", default=ALPN, help="QUIC ALPN id (crates/red_core/src/net/quic.rs)")
    ap.add_argument("--tries", type=int, default=4, help="attempts per target (default 4)")
    ap.add_argument("--timeout", type=float, default=3.0, help="seconds to wait for a UDP reply per attempt (default 3)")
    ap.add_argument("--handshake-timeout", type=float, default=6.0, help="seconds per QUIC handshake attempt (default 6)")
    ap.add_argument("--require", default=",".join(DEFAULT_REQUIRED), help="comma list of targets that must be ok (dns, relay, game, silent; default dns,relay,game)")
    ap.add_argument("--no-mask", action="store_true", help="print addresses in full (for a run against your own machine)")
    ap.add_argument("--skip", default="", help="comma list of targets not to run")
    args = ap.parse_args(argv)

    known = ("dns", "relay", "game", "silent")
    required = [t for t in args.require.split(",") if t]
    skip = {t for t in args.skip.split(",") if t}
    bad = [t for t in required + sorted(skip) if t not in known]
    if bad or args.tries < 1:
        ap.error(f"unknown target(s) {bad}" if bad else "--tries must be at least 1")
    mask = not args.no_mask

    started = time.time()
    report = {
        "probe": "net_probe",
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
        "host": args.host,
        "runner": {"egress_ip_masked": None, "platform": platform.platform(), "python": platform.python_version(), "github_run": os.environ.get("GITHUB_RUN_ID")},
        "targets": {},
    }
    if "GITHUB_ACTIONS" in os.environ or args.host not in ("127.0.0.1", "localhost"):
        report["runner"]["egress_ip_masked"] = runner_egress_ip(mask)
    log(f"runner egress (masked): {report['runner']['egress_ip_masked']}")

    log(f"dns {args.host}")
    dns, ip = probe_dns(args.host, mask)
    if "dns" not in skip:
        report["targets"]["dns"] = dns
    if ip is None:
        for name in known[1:]:
            if name not in skip:
                report["targets"][name] = result(False, "not tried: the host name did not resolve")
    else:
        if "relay" not in skip:
            log(f"relay {mask_ip(ip, 3, mask)}:{args.relay_port}")
            report["targets"]["relay"] = probe_relay(ip, args.relay_port, args.tries, args.timeout)
        if "game" not in skip:
            log(f"game (QUIC) {mask_ip(ip, 3, mask)}:{args.game_port}")
            report["targets"]["game"] = probe_game(ip, args.game_port, args.host, min(args.tries, 3), args.timeout, args.handshake_timeout, args.alpn)
        if "silent" not in skip:
            log(f"silent port {mask_ip(ip, 3, mask)}:{args.silent_port}")
            report["targets"]["silent"] = probe_silent(ip, args.silent_port, args.timeout)

    failed = [t for t in required if t in report["targets"] and not report["targets"][t]["ok"]]
    report["required"] = required
    report["failed_required"] = failed
    report["ok"] = not failed
    report["seconds"] = round(time.time() - started, 1)
    print(json.dumps(report, indent=2, sort_keys=False))
    for name, t in report["targets"].items():
        log(f"{'ok  ' if t['ok'] else 'FAIL'} {name}: {t['detail']}")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
