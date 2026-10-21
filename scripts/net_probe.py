#!/usr/bin/env python3
"""Outside-in reachability probe for a machine that hosts Red Engine games.

Everything about hosting has only ever been checked from inside the home network (loopback, the LAN, the router's own port-mapping table).
That says nothing about whether a friend in another state can reach the machine. This script runs on a machine that is *not* on that
network and answers, per link, "did an outside packet get an answer, and if not, which link broke". Two people run it:

  * `.github/workflows/reachability.yml`, every hour from a GitHub-hosted runner (a real machine on the internet). It opens one issue when
    the relay or the game port is still unreachable on a second probe a few minutes after the first. docs/HOSTING.md, "Reachability watch".
    (The box's own self-check is the inside view: units, router mappings, protocol drift. This is only what an outside packet gets.)
  * a player who cannot join, on their own machine:  python3 scripts/net_probe.py --no-json
    The defaults are the owner's server (OWNER_HOST below: a public name, no key, no secret). The last lines say in plain words which link
    failed: the name lookup, the player's own network (outgoing UDP), the relay, or the game port.

Targets (each one is a separate JSON entry, none needs a secret):

  dns     the host name resolves, and to what kind of address (public, private, carrier-grade NAT). The address itself is hidden: this output
          ends up in public CI logs and issues, and the name is public while the home address is not. When this machine's own resolver fails,
          the public resolvers below are asked directly: if one finds the address, only this machine's DNS is broken, the other targets are
          probed at that address, and the failure is not a verdict about the server.
  udp     a control: a DNS query over UDP to public resolvers (1.1.1.1, 8.8.8.8, 9.9.9.9). An answer means this machine can send UDP out and
          receive the answer, so a silent relay or game port is the server side's doing and not this machine's network. It is not a
          required target; it exists to tell "your network blocks UDP" apart from "the server is unreachable".
  relay   the shared rendezvous relay (default UDP 28016): sends the relay's real Resolve request for a code that cannot exist (`0` is not in
          the code alphabet) and requires the well-formed CodeNotFound reply. Wire format: crates/red_core/src/net/relay.rs (tag 3 + 6 code
          bytes asks, tag 5 answers). No key, no state is created on the relay.
  game    a game server (default UDP 27015, QUIC + TLS 1.3, ALPN `red/8`, crates/red_core/src/net/quic.rs). Two steps: (1) a stateless QUIC
          Version Negotiation probe (an Initial with a reserved version: a live QUIC listener answers with the versions it speaks), then (2) a full
          QUIC/TLS handshake. The game's join key is checked later, inside the encrypted Hello, and is NOT needed for the handshake, so nothing
          here can join a match. The SHA-256 of the presented certificate (the same `sha256:<hex>` clients pin) is recorded: it is public, and
          `--fingerprint` compares it with the one you were given. Neither the game version nor the key is checked: this is the network, not the game.
  silent  a plain UDP datagram to a port nothing should listen on (default 27016). It must stay unanswered. It is a control, not a pass/fail
          check: silence looks the same whether a firewall dropped the packet or the port is closed, so it only documents "nothing answers".

For every UDP target the detail says which of three things happened, because they mean different things on the way in:
  reply    something answered.
  refused  an ICMP port-unreachable came back: the packet reached a machine (or the router) that has no listener on that port.
  silent   nothing came back within the timeout: dropped on the way (router with no mapping, a firewall) or no reply sent.

Output: one JSON object on stdout (`--no-json` leaves it out), progress and then the plain-words result on stderr. Every address is hidden
(`x.x.x.x`) in everything printed, the public resolvers of the UDP control excepted; `--no-mask` shows them for a run against your own
machine, and `--markdown` hides them whatever it is given. Exit codes: 0 every required target is ok, 1 one is not, 2 bad arguments, 3 incomplete
(everything that could be tested passed but the QUIC handshake needs aioquic, which is not installed), 4 inconclusive (a required target
failed while this machine could not use UDP at all, or the only failure is this machine's own resolver while a public one found the name,
so the failure says nothing about the server), 70 the probe itself crashed (a bug in the probe, not a verdict about the server; its
traceback is hidden like every other line). Only 1 is a verdict that the server is unreachable.
Python standard library only; the QUIC handshake additionally needs `python3 -m pip install aioquic==1.2.0` (the workflow does that), and
without it the `game` target says so and still does its Version Negotiation step.

    python3 scripts/net_probe.py --no-json                            # the owner's server, plain words only
    python3 scripts/net_probe.py --host HOST --relay-port 28016 --game-port 27015
    python3 scripts/net_probe.py --host 127.0.0.1 --no-mask           # against the machine itself, to see the probe work
    python3 scripts/net_probe.py --markdown probe.json --style issue  # the Markdown the workflow posts, from a saved result
"""

import argparse
import errno
import hashlib
import ipaddress
import json
import os
import platform
import re
import select
import socket
import ssl
import struct
import sys
import textwrap
import time
import traceback
import urllib.request

# The owner's server: public facts only. The name is the one the game itself resolves (crates/red_core/src/net/relay.rs DEFAULT_RELAY), the ports are
# the deployed relay and game server (docs/HOSTING.md). There is deliberately NO join key here and no default fingerprint: the fingerprint is public but
# changes whenever the server's identity is remade, and a stale constant would turn a harmless redeploy into a false alarm (pass `--fingerprint`).
OWNER_HOST = "red-engine.duckdns.org"
DEFAULT_HOST = OWNER_HOST
RELAY_PORT = 28016
GAME_PORT = 27015
SILENT_PORT = 27016
OWNER_FINGERPRINT = None
ALPN = "red/8"
AIOQUIC_PIN = "aioquic==1.2.0"
AIOQUIC_INSTALL = f"python3 -m pip install {AIOQUIC_PIN}   (on Windows: py -m pip install {AIOQUIC_PIN})"

# Relay control messages (crates/red_core/src/net/relay.rs): tag byte, then the payload.
TAG_RESOLVE = 3
TAG_CODE_NOT_FOUND = 5
CODE_LEN = 6
# '0' is not in the code alphabet (ABCDEFGHJKMNPQRSTUVWXYZ23456789), so this can never be a live code: the answer is always CodeNotFound.
IMPOSSIBLE_CODE = b"000000"

# The control for "can this machine use UDP at all": public DNS resolvers (their addresses are public and exempt from IP masking).
UDP_CONTROL_RESOLVERS = (("1.1.1.1", 53), ("8.8.8.8", 53), ("9.9.9.9", 53))
UDP_CONTROL_NAME = "example.com"

KNOWN_TARGETS = ("dns", "udp", "relay", "game", "silent")
DEFAULT_REQUIRED = ("dns", "relay", "game")
PAUSE_BETWEEN_TRIES = 0.5  # seconds; tests set it to 0

EXIT_OK, EXIT_FAILED, EXIT_USAGE, EXIT_INCOMPLETE, EXIT_INCONCLUSIVE, EXIT_INTERNAL = 0, 1, 2, 3, 4, 70

# Whether addresses are hidden in everything this process prints. `main` sets it from --no-mask; the default is the safe one, because every line
# the probe writes can end up in a public CI log or an issue.
MASKING = True
HIDDEN_IPV4 = "x.x.x.x"
HIDDEN_IPV6 = "x:x::x"
_IPV4 = re.compile(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b")
# Hex groups joined by colons. Only a run with a `::` or at least five colons is taken for an IPv6 address, so a time of day (17:55:39) or a
# `sha256:` prefix is left alone. The probe asks for IPv4 only; this is for an address a library's error text might carry.
_IPV6 = re.compile(r"(?<![\w:.])(?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}(?![\w:])")


# Fixed when the module loads: whatever the UDP control is later pointed at (a test's fake resolver on loopback), only these well-known resolvers are shown.
PUBLIC_RESOLVERS = frozenset(ip for ip, _port in UDP_CONTROL_RESOLVERS)


def keep_unmasked():
    return PUBLIC_RESOLVERS


def redact_text(text, enabled=None):
    """Every dotted IPv4 in `text` becomes `x.x.x.x` and every IPv6 `x:x::x`, except the public resolvers the UDP control talks to.
    A safety net over the explicit hiding below: an exception message or a library's error text can carry an address the probe did not think of.
    Nothing of the address is kept, not even a prefix: the first three octets of a home address leave only a few hundred candidates."""
    if (MASKING if enabled is None else enabled) is False:
        return text
    keep = keep_unmasked()
    text = _IPV4.sub(lambda m: m.group(0) if m.group(0) in keep else HIDDEN_IPV4, text)
    return _IPV6.sub(lambda m: HIDDEN_IPV6 if "::" in m.group(0) or m.group(0).count(":") >= 5 else m.group(0), text)


def redact(obj, enabled=None):
    """`redact_text` over every string in a JSON-like structure (keys are left alone)."""
    if isinstance(obj, str):
        return redact_text(obj, enabled)
    if isinstance(obj, list):
        return [redact(v, enabled) for v in obj]
    if isinstance(obj, dict):
        return {k: redact(v, enabled) for k, v in obj.items()}
    return obj


def log(msg):
    print(redact_text(msg), file=sys.stderr, flush=True)


def mask_ip(ip, octets_kept, enabled=True):
    """`203.0.113.45` -> `x.x.x.x` (keep 0: the host's address) or `203.0.x.x` (keep 2: a GitHub-hosted runner's egress, a cloud address
    that is not anybody's home); anything that is not a dotted IPv4 is returned as is."""
    parts = ip.split(".")
    if not enabled or len(parts) != 4:
        return ip
    return ".".join(parts[:octets_kept] + ["x"] * (4 - octets_kept))


SHARED_ADDRESS_SPACE = ipaddress.ip_network("100.64.0.0/10")  # RFC 6598: carrier-grade NAT, the ISP shares one public address between customers


def address_kind(ip):
    """What kind of address `ip` is, which is safe to publish where the address is not: `public`, `private` (a home or office network,
    including the documentation ranges), `shared` (carrier-grade NAT), `loopback` or `other`."""
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return "other"
    if a.is_loopback:
        return "loopback"
    if a in SHARED_ADDRESS_SPACE:
        return "shared"
    if a.is_private:
        return "private"
    return "public" if a.is_global else "other"


# A name that points at one of these cannot be reached from the internet whatever the router does.
UNREACHABLE_KINDS = {"private": "a private (home network) address", "shared": "a carrier-grade NAT address (100.64.0.0/10)"}


def is_address(host):
    try:
        ipaddress.ip_address(host)
        return True
    except ValueError:
        return False


def wrong_record(report_or_host, dns):
    """The kind of address a NAME points at when nobody on the internet can reach it (None otherwise; an address given as --host is what the user meant)."""
    host = report_or_host if isinstance(report_or_host, str) else report_or_host.get("host", "")
    kind = (dns or {}).get("address_kind")
    return kind if kind in UNREACHABLE_KINDS and not is_address(host) else None


def ms(seconds):
    return round(seconds * 1000.0, 1)


def result(ok, detail, rtt=None, **extra):
    out = {"ok": bool(ok), "rtt_ms": None if rtt is None else ms(rtt), "detail": detail}
    out.update(extra)
    return out


# ---------------------------------------------------------------------------------------------------------------------------------------
# UDP request/reply with the outcomes spelled out

# A connected UDP socket learns of an ICMP port-unreachable as ECONNREFUSED on Linux and macOS, and as WSAECONNRESET (ConnectionResetError) on Windows.
REFUSED_ERRORS = (ConnectionRefusedError, ConnectionResetError)


def os_error_name(e):
    return errno.errorcode.get(e.errno or 0) or e.__class__.__name__


def udp_exchange(ip, port, payload, timeout, accept=None):
    """Sends `payload` from a fresh socket (a fresh source port every call) and waits for one reply.

    Returns (outcome, reply_bytes, seconds): outcome is "reply", "refused" (ICMP port unreachable), "silent", or "unreachable:<ERRNO>" (this machine has
    no route: no network, no default gateway). `accept(reply)` may reject a stray datagram, in which case waiting continues until the timeout."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    t0 = time.monotonic()
    try:
        try:
            sock.connect((ip, port))  # connected: only this peer's replies arrive, and an ICMP error surfaces on the next call
            t0 = time.monotonic()
            sock.send(payload)
        except REFUSED_ERRORS:
            return "refused", b"", time.monotonic() - t0
        except OSError as e:
            return "unreachable:" + os_error_name(e), b"", time.monotonic() - t0
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
            except REFUSED_ERRORS:
                return "refused", b"", time.monotonic() - t0
            except OSError as e:  # EHOSTUNREACH / ENETUNREACH arrive the same way
                return "unreachable:" + os_error_name(e), b"", time.monotonic() - t0
            if accept is None or accept(data):
                return "reply", data, time.monotonic() - t0
    finally:
        sock.close()


# ---------------------------------------------------------------------------------------------------------------------------------------
# targets


def probe_dns(host, mask, timeout=2.0):
    """The name through this machine's own resolver (what the game does). When that fails, the public resolvers of the UDP control are asked
    directly: if one of them finds an address, the name is fine and only this machine's resolver failed (`local_resolver_failed`), so the
    other targets are still probed at that address and the failure is not a verdict about the server (exit 4 when it is the only one)."""
    t0 = time.monotonic()
    try:
        infos = socket.getaddrinfo(host, None, socket.AF_INET, socket.SOCK_DGRAM)
    except OSError as e:
        ips, resolver, notes = public_dns_lookup(host, timeout)
        if ips:
            shown = [mask_ip(i, 0, mask) for i in ips]
            kind = address_kind(ips[0])
            return result(False, f"this machine's resolver could not look up {host} ({e}), but the public resolver {resolver} found it: {', '.join(shown)} ({kind})",
                          None, addresses=shown, address_kind=kind, local_resolver_failed=True, public_resolver=resolver), ips[0]
        public = f"; the public resolvers found no address either ({'; '.join(notes)})" if notes else ""
        return result(False, f"{host} did not resolve to an IPv4 address: {e}{public}", public_lookup=notes), None
    took = time.monotonic() - t0
    ips = sorted({i[4][0] for i in infos})
    shown = [mask_ip(i, 0, mask) for i in ips]
    kind = address_kind(ips[0])  # the one the other targets are probed at
    return result(True, f"{host} -> {', '.join(shown)} ({kind})", took, addresses=shown, address_kind=kind), ips[0]


def dns_query(name, qid):
    """A standard recursive A query for `name` (ValueError for a name a query cannot carry)."""
    labels = name.rstrip(".").split(".")
    if not all(0 < len(label) < 64 for label in labels):
        raise ValueError(f"not a DNS name: {name!r}")
    question = b"".join(bytes([len(label)]) + label.encode("ascii") for label in labels) + b"\0" + struct.pack(">HH", 1, 1)
    return struct.pack(">HHHHHH", qid, 0x0100, 1, 0, 0, 0) + question


DNS_RCODES = {0: "no address", 1: "FORMERR", 2: "SERVFAIL", 3: "NXDOMAIN (no such name)", 5: "REFUSED"}


def parse_a_records(data, qid):
    """(rcode, [IPv4, ...]) from a DNS response to query `qid` (the A records of its answer section), or None when `data` is not one."""
    if len(data) < 12:
        return None
    rid, flags, qdcount, ancount, _ns, _ar = struct.unpack(">HHHHHH", data[:12])
    if rid != qid or not flags & 0x8000:
        return None
    pos, ips = 12, []

    def skip_name(pos):
        while True:
            n = data[pos]  # IndexError on a truncated name
            if n & 0xC0 == 0xC0:
                return pos + 2  # a compression pointer ends the name
            if n == 0:
                return pos + 1
            pos += 1 + n

    try:
        for _ in range(qdcount):
            pos = skip_name(pos) + 4
        for _ in range(ancount):
            pos = skip_name(pos)
            rtype, rclass, _ttl, rdlen = struct.unpack(">HHIH", data[pos:pos + 10])
            rdata = data[pos + 10:pos + 10 + rdlen]
            pos += 10 + rdlen
            if rtype == 1 and rclass == 1 and len(rdata) == 4:
                ips.append(socket.inet_ntoa(rdata))
    except (IndexError, struct.error):
        pass  # a truncated answer: keep what was read
    return flags & 0xF, ips


def public_dns_lookup(host, timeout):
    """Asks the public resolvers of the UDP control for `host`'s A record directly, past this machine's own resolver. Returns (addresses, the
    resolver that gave them, notes): the first resolver with an address wins; otherwise the notes say what each one answered."""
    try:
        dns_query(host, 0)
    except (ValueError, UnicodeError):
        return [], None, []
    notes = []
    for ip, port in UDP_CONTROL_RESOLVERS:
        qid = struct.unpack(">H", os.urandom(2))[0]
        outcome, data, _took = udp_exchange(ip, port, dns_query(host, qid), timeout, accept=lambda d, q=qid: parse_a_records(d, q) is not None)
        if outcome != "reply":
            notes.append(f"{ip}: {outcome}")
            continue
        rcode, ips = parse_a_records(data, qid)
        if ips:
            log(f"  public resolver {ip}: {len(ips)} address(es) for {host}")
            return ips, ip, notes
        notes.append(f"{ip}: {DNS_RCODES.get(rcode, f'rcode {rcode}')}")
    return [], None, notes


def probe_udp_control(timeout):
    """Can this machine send a UDP datagram out and get the answer back? One query to each public resolver in turn; the first answer is enough."""
    attempts = []
    for ip, port in UDP_CONTROL_RESOLVERS:
        qid = struct.unpack(">H", os.urandom(2))[0]
        outcome, data, took = udp_exchange(ip, port, dns_query(UDP_CONTROL_NAME, qid), timeout,
                                           accept=lambda d, q=qid: len(d) >= 12 and struct.unpack(">H", d[:2])[0] == q and d[2] & 0x80 != 0)
        attempts.append(f"{ip}: {outcome}")
        log(f"  udp control {ip}:{port}: {outcome}")
        if outcome == "reply":
            return result(True, f"UDP works from this machine: {ip} answered a DNS query in {ms(took)} ms", took, resolver=ip, attempts=attempts)
    kinds = [a.split(": ", 1)[1] for a in attempts]
    if all(k.startswith("unreachable") for k in kinds):
        detail = f"this machine has no route to the internet ({kinds[0]})"
    else:
        detail = "no public DNS resolver answered over UDP: this network, a VPN or a firewall seems to block outgoing UDP"
    return result(False, detail, None, attempts=attempts, no_route=all(k.startswith("unreachable") for k in kinds))


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
            return result(False, f"something answered on UDP {port} but not with CodeNotFound: {data[:16].hex()} ({len(data)} bytes)", took, port=port, attempts=attempts,
                          wrong_reply=True)
        if outcome.startswith("unreachable"):
            break  # no route: asking again changes nothing
        time.sleep(PAUSE_BETWEEN_TRIES)
    return result(False, explain_no_reply(port, attempts, "relay"), None, port=port, attempts=attempts)


def explain_no_reply(port, attempts, what):
    refused = [a for a in attempts if a == "refused"]
    unreachable = [a for a in attempts if a.startswith("unreachable")]
    if unreachable:
        return f"no {what} reply on UDP {port}: this machine could not send the packet ({unreachable[0].split(':', 1)[1]}), it has no route to the host"
    if refused:
        return (f"no {what} reply on UDP {port}: ICMP unreachable came back, so the packet reached a machine or router "
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
        if outcome.startswith("unreachable"):
            break
        time.sleep(PAUSE_BETWEEN_TRIES)
    return result(False, explain_no_reply(port, attempts, "QUIC"), None, attempts=attempts)


# TLS alert numbers a server can answer a ClientHello with (a QUIC CONNECTION_CLOSE carries 0x100 + the alert).
TLS_ALERTS = {40: "handshake_failure", 70: "protocol_version", 80: "internal_error", 120: "no_application_protocol (the server speaks a different ALPN than this probe)"}


def describe_close(error_code, reason_phrase, timeout):
    if reason_phrase == "Idle timeout":
        # aioquic's own idle timer: the server stopped answering mid-handshake. (Not a close frame the server sent.)
        return f"the handshake stalled: the server stopped answering and nothing completed within {timeout:g} s"
    if 0x100 <= error_code < 0x200:
        alert = error_code - 0x100
        why = f" ({reason_phrase})" if reason_phrase else ""
        return f"the server refused the handshake with TLS alert {alert} {TLS_ALERTS.get(alert, '')}".strip() + why
    return f"the server closed the connection during the handshake: error 0x{error_code:x} {reason_phrase!r}".strip()


def quic_handshake_once(ip, port, server_name, alpn, timeout):
    """One QUIC/TLS 1.3 handshake with no join key, driven by aioquic's sans-IO QuicConnection on a plain IPv4 UDP socket (so the runner's
    IPv6 support cannot matter). Returns a dict: ok, error, cert_sha256, alpn, handshake_ms, icmp_refused."""
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
            except REFUSED_ERRORS:
                out["icmp_refused"] = True
                out["error"] = "ICMP port unreachable: nothing listens on that UDP port (or a router rejected the packet)"
                return False
            except OSError as e:
                out["error"] = f"send failed: {os_error_name(e)}"
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
                except REFUSED_ERRORS:
                    out["icmp_refused"] = True
                    out["error"] = "ICMP port unreachable: nothing listens on that UDP port (or a router rejected the packet)"
                    return out
                except OSError as e:
                    out["error"] = f"receive failed: {os_error_name(e)}"
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
                    out["error"] = describe_close(event.error_code, event.reason_phrase, timeout)
                    return out
        # Say goodbye properly so the server does not wait out its idle timeout for a connection that is already finished.
        conn.close(error_code=0, reason_phrase="net-probe")
        flush()
        return out
    finally:
        sock.close()


def aioquic_installed():
    try:
        import aioquic  # noqa: F401
        return True
    except ImportError:
        return False


def same_fingerprint(a, b):
    norm = lambda s: s.strip().lower().replace(":", "")
    return norm(a or "").removeprefix("sha256") == norm(b or "").removeprefix("sha256")


def probe_game(ip, port, server_name, tries, timeout, handshake_timeout, alpn, fingerprint=None):
    listener = probe_quic_listener(ip, port, tries, timeout)
    if not aioquic_installed():
        return result(False, f"the QUIC handshake was not tried: aioquic is not installed ({AIOQUIC_INSTALL}); Version Negotiation: " + listener["detail"],
                      None, port=port, listener=listener, handshake=None, needs_aioquic=True)
    attempts = []
    last = None
    for n in range(1, tries + 1):
        try:
            last = quic_handshake_once(ip, port, server_name, alpn, handshake_timeout)
        except Exception as e:  # a probe must report, not crash
            last = {"ok": False, "error": f"{e.__class__.__name__}: {e}", "cert_sha256": None, "alpn": None, "handshake_ms": None}
        attempts.append("handshake" if last["ok"] else (last["error"] or "failed"))
        log(f"  quic handshake try {n}/{tries}: " + (f"completed in {last['handshake_ms']} ms, alpn={last['alpn']}" if last["ok"] else str(last["error"])))
        if last["ok"] or last.get("icmp_refused"):
            break
        time.sleep(PAUSE_BETWEEN_TRIES)
    if last and last["ok"]:
        extra = {}
        ok, detail = True, f"QUIC/TLS 1.3 handshake completed (ALPN {last['alpn']}, no join key sent)"
        if fingerprint:
            match = same_fingerprint(fingerprint, last["cert_sha256"])
            extra["fingerprint_match"] = match
            if not match:
                ok, detail = False, f"the handshake completed but the server's certificate is {last['cert_sha256']}, not the fingerprint you gave"
        return result(ok, detail, (last["handshake_ms"] or 0) / 1000.0, port=port, cert_sha256=last["cert_sha256"], alpn=last["alpn"], listener=listener,
                      attempts=attempts, **extra)
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


def github_hosted():
    """True on a runner GitHub provides (a cloud machine). A self-hosted runner, the box or a player's machine is somebody's network."""
    return os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"


def runner_egress_ip(mask):
    """Best effort: the address this machine's traffic leaves from. On a GitHub-hosted runner it keeps two octets (a cloud network, so a host's
    firewall log can be matched by network and time); anywhere else it is hidden whole, because run from home the first octets are the home
    address's. Opt-in (`--egress`): it asks two third-party services. Never fails the probe."""
    for url in ("https://api.ipify.org", "https://ifconfig.me/ip"):
        try:
            with urllib.request.urlopen(url, timeout=5) as r:
                ip = r.read(64).decode().strip()
            if ip.count(".") == 3:
                return mask_ip(ip, 2 if github_hosted() else 0, mask)
        except Exception:
            continue
    return None


# ---------------------------------------------------------------------------------------------------------------------------------------
# plain words: which link failed


LINK_LABELS = {"dns": "Name lookup", "udp": "Your network (UDP out)", "relay": "Relay (short codes)", "game": "Game server", "silent": "Control port"}
TELL_THE_OWNER = "Tell the owner."


def attempt_kinds(target):
    return [a.split(": ", 1)[-1] for a in (target or {}).get("attempts", [])]


def udp_blocked(targets):
    """True when the control ran and failed (this machine's own UDP is the problem); None when it did not run."""
    control = targets.get("udp")
    return None if control is None else not control["ok"]


def local_resolver_failed(targets):
    """True when the name lookup failed only on this machine: its resolver did not answer but a public resolver found the address."""
    dns = targets.get("dns") or {}
    return not dns.get("ok", True) and bool(dns.get("local_resolver_failed"))


def words_dns(t, host):
    if t.get("local_resolver_failed"):
        shown = ", ".join(t.get("addresses") or ["an address"])
        return (f"this machine's own DNS resolver could not look up {host}, but a public DNS server found it ({shown}, a {t.get('address_kind') or 'known'} "
                f"address): the name is fine and this machine's DNS is the problem (its settings, a VPN, a filter or the router). The game looks the name up the "
                f"same way, so it fails here too until that is fixed or another network is used.")
    if t["ok"]:
        kind = t.get("address_kind") or "known"
        shown = ", ".join(t.get("addresses") or ["resolved"])
        if wrong_record(host, t):
            return (f"{host} resolves, but to {UNREACHABLE_KINDS[kind]} ({shown}), which nobody on the internet can reach: whatever keeps the name up to "
                    f"date sent the wrong address. {TELL_THE_OWNER}")
        return f"{host} resolves to a {kind} address ({shown})."
    return (f"{host} did not resolve to an address from this machine. If other websites work for you, the name has stopped pointing at the server "
            f"(it is a DuckDNS name the owner's box keeps up to date) and the owner has to fix it; if nothing works for you, check your internet connection.")


def words_udp(t):
    if t["ok"]:
        return "UDP packets leave this machine and answers come back, so this machine's own network is not the problem."
    if t.get("no_route"):
        return "this machine has no route to the internet at all (no network, or no gateway). Connect to a network and run this again."
    return ("no public DNS server (1.1.1.1, 8.8.8.8, 9.9.9.9) answered over UDP. This network, a VPN or a firewall blocks outgoing UDP (or lets through "
            "only its own DNS server). The game needs UDP, so it cannot work from here whatever state the server is in. Try another network such as a "
            "phone hotspot, turn off any VPN, and allow Python (and the game) through the firewall.")


def words_relay(t, blocked, port):
    if t["ok"]:
        return f"the relay answered on UDP {port}: short join codes can be looked up."
    if t.get("wrong_reply"):
        return f"something answers on UDP {port} but it is not the relay's reply. {TELL_THE_OWNER}"
    if blocked:
        return "no answer, but this machine cannot use UDP at all (see above), so this says nothing about the relay."
    kinds = attempt_kinds(t)
    if any(k.startswith("unreachable") for k in kinds):
        return "this machine could not send the packet (no route to the host): connect to a network and run this again."
    if "refused" in kinds:
        return (f"the packet arrived but the owner's machine said nothing listens on UDP {port}: the relay is stopped. {TELL_THE_OWNER} Direct join codes "
                f"(the long ones) need no relay and can still work.")
    return (f"no answer on UDP {port} although UDP works from here: the relay is stopped, or the owner's router or firewall is not passing that port, or the "
            f"home connection is down. {TELL_THE_OWNER} Direct join codes (the long ones) need no relay and can still work.")


def words_game(t, blocked, port):
    listener = t.get("listener") or {}
    if t.get("needs_aioquic"):
        seen = f"A QUIC listener does answer on UDP {port}. " if listener.get("ok") else ""
        return f"NOT fully tested: the encrypted handshake needs the aioquic library. {seen}Install it and run this again: {AIOQUIC_INSTALL}"
    if t["ok"]:
        return f"a QUIC/TLS handshake completed on UDP {port} (no join key was sent): the game server is up and reachable."
    if t.get("fingerprint_match") is False:
        return (f"the server answers but its certificate is not the one you were given: the server's identity was remade, so your join code is out of date. "
                f"Ask the owner for the current one.")
    if blocked:
        return "no answer, but this machine cannot use UDP at all (see above), so this says nothing about the game server."
    kinds = attempt_kinds(listener)
    if listener.get("ok"):
        return (f"a QUIC listener answers on UDP {port} but the encrypted handshake did not finish ({t['detail'].split(': ', 1)[-1]}). The server is running "
                f"but not accepting connections. {TELL_THE_OWNER}")
    if any(k.startswith("unreachable") for k in kinds):
        return "this machine could not send the packet (no route to the host): connect to a network and run this again."
    if "refused" in kinds:
        return f"the packet arrived but the owner's machine said nothing listens on UDP {port}: the game server is stopped. {TELL_THE_OWNER}"
    return (f"no answer on UDP {port} although UDP works from here: the game server is stopped, or the owner's router or firewall is not passing that port, or the "
            f"home connection is down. {TELL_THE_OWNER}")


def words_silent(t, port):
    return f"something answers on UDP {port}, which nothing should: {t['detail']}"


def verdict_words(report):
    targets = report["targets"]
    ok = lambda name: targets.get(name, {}).get("ok", True)
    if report.get("incomplete"):
        return (f"INCOMPLETE: everything that could be tested passed, but the encrypted handshake with the game server needs aioquic, which is not "
                f"installed. Install it and run this again: {AIOQUIC_INSTALL}")
    if report["ok"]:
        return ("OK: from this machine the relay and the game server can both be reached. If joining still fails, the network between you and the server is "
                "not the cause: check that you both run the same release of the game and use a current join code (this probe sends no key and checks neither).")
    if udp_blocked(targets):
        return ("INCONCLUSIVE: no outgoing UDP works from this machine, so nothing can reach the server from here whatever state it is in. The cause is "
                "this machine's own network; the results below say nothing about the server.")
    local_dns = local_resolver_failed(targets)
    if report.get("inconclusive") and local_dns:
        return (f"INCONCLUSIVE: this machine's own DNS resolver could not look up {report['host']}, but a public DNS server found its address and every other "
                f"check passed there: the server is reachable from the internet. The cause is this machine's DNS (its settings, a VPN, a filter or the router).")
    if not ok("dns") and not local_dns:
        return "FAILED at the first step: this machine cannot find the server's address."
    kind = wrong_record(report, targets.get("dns"))
    if not ok("relay") and not ok("game") and kind:
        return (f"FAILED: neither the relay nor the game server answers, and {report['host']} points at {UNREACHABLE_KINDS[kind]}, which cannot be reached "
                f"from the internet: the name's record is wrong. {TELL_THE_OWNER}")
    if not ok("relay") and not ok("game"):
        return (f"FAILED: neither the relay nor the game server answers although UDP works from here: the owner's home connection is not reachable from the "
                f"internet (router port mapping, a changed address, the machine off). {TELL_THE_OWNER}")
    if not ok("relay"):
        return f"FAILED: the game server answers but the relay does not: joining with a short code will fail. {TELL_THE_OWNER}"
    if not ok("game"):
        return f"FAILED: the relay answers but the game server does not: a short code resolves, then the join fails. {TELL_THE_OWNER}"
    return "FAILED: see the lines above."


def diagnose(report):
    """The links in the order a packet needs them, each with a verdict in plain words. Returns [{link, label, ok, words}]."""
    targets = report["targets"]
    host = report["host"]
    ports = report.get("ports", {})
    blocked = udp_blocked(targets)
    out = []

    def add(name, words):
        if targets[name].get("not_tried"):
            words = "not tried: the name lookup failed first."
        out.append({"link": name, "label": LINK_LABELS[name], "ok": targets[name]["ok"], "words": words})

    if "dns" in targets:
        add("dns", words_dns(targets["dns"], host))
    if "udp" in targets:
        add("udp", words_udp(targets["udp"]))
    if "relay" in targets:
        add("relay", words_relay(targets["relay"], blocked, ports.get("relay", RELAY_PORT)))
    if "game" in targets:
        add("game", words_game(targets["game"], blocked, ports.get("game", GAME_PORT)))
    if "silent" in targets and not targets["silent"]["ok"]:
        add("silent", words_silent(targets["silent"], ports.get("silent", SILENT_PORT)))
    return out


def render_plain(report, width=100):
    """The plain-words result for a terminal (ASCII only: a Windows console may not print anything else)."""
    lines = [f"RESULT for {report['host']}", ""]
    for d in report.get("diagnosis") or diagnose(report):
        status = "ok  " if d["ok"] else "FAIL"
        first = f"  [{status}] {d['label']}: "
        lines.append(textwrap.fill(d["words"], width=width, initial_indent=first, subsequent_indent=" " * len(first)))
    lines += ["", textwrap.fill(report.get("verdict") or verdict_words(report), width=width, initial_indent="  ", subsequent_indent="  ")]
    return "\n".join(lines)


# ---------------------------------------------------------------------------------------------------------------------------------------
# Markdown (the job summary and the issue the workflow opens): rendered from a saved JSON result


def cell(text):
    return str(text).replace("|", "\\|").replace("\r", " ").replace("\n", " ")


GITHUB_LOGIN = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$")


def mentions(logins):
    """`@login` for each valid GitHub login in `logins` (a comma list or a list); anything else is dropped with a note on stderr, never an error:
    a bad name in the list must not stop the issue that reports an outage."""
    names = [n.strip() for n in (logins.split(",") if isinstance(logins, str) else logins or []) if n.strip()]
    good = [n for n in names if GITHUB_LOGIN.match(n)]
    for bad in sorted(set(names) - set(good)):
        print(f"net_probe: not a GitHub login, not mentioned: {bad!r}", file=sys.stderr)
    return [f"@{n}" for n in good]


def render_markdown(report, style="summary", run_url=None, mention=None):
    """`summary`: the job summary. `issue`: the body of the issue opened when the targets stay unreachable (`mention`: the logins to notify).
    `comment`: a follow-up comment on it. `reopen`: the comment on the watch's issue when an outage comes back within a day of its close.
    Every string goes through `redact_text` once more, always (whatever --no-mask said), so a result file edited or produced elsewhere cannot
    put an address in a public page."""
    host = report.get("host", "?")
    if report.get("ok"):
        status = "ok"
    else:
        status = "INCONCLUSIVE" if report.get("inconclusive") else ("INCOMPLETE" if report.get("incomplete") else "FAILED")
    verdict = report.get("verdict") or verdict_words(report)
    failed = [LINK_LABELS.get(n, n) for n in report.get("failed_required") or []]
    out = []
    if style == "issue":
        out += [f"The hourly outside-in probe could not reach `{host}` from a GitHub runner, on two probes a few minutes apart"
                + (f" (failed: {', '.join(failed)})." if failed else ".")
                + " A friend trying to join sees the same. The box's own checks look from the inside and can miss it: the server's journal and the router's "
                "port table can both look fine while nothing gets in.", ""]
        tagged = mentions(mention)
        if tagged:
            out += ["cc " + " ".join(tagged), ""]
    elif style == "comment":
        out += [f"Still unreachable at {report.get('started_utc', '?')}.", ""]
    elif style == "reopen":
        out += [f"Unreachable again at {report.get('started_utc', '?')}, on two probes a few minutes apart"
                + (f" (failed: {', '.join(failed)})" if failed else "")
                + ". This issue was closed less than a day ago, so the watch reopens it instead of opening a new one.", ""]
    else:
        out += [f"### Reachability of {host}: {status}", ""]
    if run_url:
        out += [f"Run: {run_url}", ""]
    out += [f"**{verdict}**", ""]
    out += ["| link | ok | rtt ms | what it means |", "|---|---|---|---|"]
    diag = {d["link"]: d for d in report.get("diagnosis") or diagnose(report)}
    for name, t in report.get("targets", {}).items():
        words = diag[name]["words"] if name in diag else t.get("detail", "")
        rtt = "" if t.get("rtt_ms") is None else t["rtt_ms"]
        out.append(f"| {LINK_LABELS.get(name, name)} | {'yes' if t.get('ok') else 'NO'} | {rtt} | {cell(words)} |")
    cert = report.get("targets", {}).get("game", {}).get("cert_sha256")
    if cert:
        out += ["", f"Server certificate: `{cert}`"]
    egress = (report.get("runner") or {}).get("egress_ip_masked")
    out += ["", f"Probed at {report.get('started_utc', '?')} from a GitHub runner" + (f" (egress, masked: `{egress}`)." if egress else ".")]
    if style == "issue":
        out += ["", "To check by hand from another network (no key is needed): `python3 scripts/net_probe.py --no-json`; see `docs/HOSTING.md`, \"Reachability watch\".",
                "This issue is opened by `.github/workflows/reachability.yml` and closed by it on the next clean pass; while it stays open, the watch adds a comment "
                "at most every six hours, and an outage that comes back within a day of the close reopens it. To silence the watch during a known outage: "
                "`gh workflow disable reachability.yml` (and `gh workflow enable reachability.yml` afterwards)."]
    out += ["", "<details><summary>The probe's JSON</summary>", "", "```json", json.dumps(report, indent=2), "```", "", "</details>"]
    return redact_text("\n".join(out) + "\n", enabled=True)


# ---------------------------------------------------------------------------------------------------------------------------------------


def build_parser():
    ap = argparse.ArgumentParser(description="Outside-in reachability probe for Red Engine game hosting (see the module docstring). "
                                 "With no options it checks the owner's server and says in plain words which link fails.")
    ap.add_argument("--host", default=DEFAULT_HOST, help=f"host name or IPv4 address (default {DEFAULT_HOST})")
    ap.add_argument("--relay-port", type=int, default=RELAY_PORT, help=f"relay UDP port (default {RELAY_PORT})")
    ap.add_argument("--game-port", type=int, default=GAME_PORT, help=f"game server UDP port (default {GAME_PORT})")
    ap.add_argument("--silent-port", type=int, default=SILENT_PORT, help=f"a UDP port nothing should answer on (default {SILENT_PORT})")
    ap.add_argument("--fingerprint", default=OWNER_FINGERPRINT, help="optional sha256:<hex> certificate fingerprint to compare with the server's (public; not a key)")
    ap.add_argument("--alpn", default=ALPN, help="QUIC ALPN id (crates/red_core/src/net/quic.rs)")
    ap.add_argument("--tries", type=int, default=4, help="attempts per target (default 4)")
    ap.add_argument("--timeout", type=float, default=3.0, help="seconds to wait for a UDP reply per attempt (default 3)")
    ap.add_argument("--handshake-timeout", type=float, default=6.0, help="seconds per QUIC handshake attempt (default 6)")
    ap.add_argument("--require", default=",".join(DEFAULT_REQUIRED), help="comma list of targets that must be ok (dns, udp, relay, game, silent; default dns,relay,game)")
    ap.add_argument("--skip", default="", help="comma list of targets not to run")
    ap.add_argument("--no-mask", action="store_true", help="print addresses in full (for a run against your own machine; --markdown hides them regardless)")
    ap.add_argument("--no-json", action="store_true", help="print only the progress and the plain-words result, not the JSON")
    ap.add_argument("--egress", action="store_true", help="also look up this machine's public address (two third-party web services) and record it masked: "
                    "two octets kept on a GitHub-hosted runner, none anywhere else")
    ap.add_argument("--markdown", metavar="RESULT.json", help="do not probe: print Markdown for a saved JSON result")
    ap.add_argument("--style", choices=("summary", "issue", "comment", "reopen"), default="summary",
                    help="with --markdown: the job summary, an issue body, a follow-up comment, or the comment on a reopened issue")
    ap.add_argument("--run-url", default=None, help="with --markdown: the workflow run's link")
    ap.add_argument("--mention", default="", help="with --markdown --style issue: comma list of GitHub logins to notify (the workflow passes the maintainers "
                    "in .github/contributors.json); a name that is not a login is left out")
    return ap


def markdown_mode(args):
    try:
        with open(args.markdown, encoding="utf-8") as f:
            report = json.load(f)
        text = render_markdown(report, args.style, args.run_url, args.mention)
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as e:
        print(f"net_probe: cannot render {args.markdown}: {e.__class__.__name__}: {e}", file=sys.stderr)
        return EXIT_USAGE
    sys.stdout.write(text)
    return EXIT_OK


def main(argv=None):
    global MASKING
    ap = build_parser()
    args = ap.parse_args(argv)
    if args.markdown:
        return markdown_mode(args)

    required = [t for t in args.require.split(",") if t]
    skip = {t for t in args.skip.split(",") if t}
    bad = [t for t in required + sorted(skip) if t not in KNOWN_TARGETS]
    if bad or args.tries < 1:
        ap.error(f"unknown target(s) {bad}" if bad else "--tries must be at least 1")
    mask = not args.no_mask
    MASKING = mask

    started = time.time()
    report = {
        "probe": "net_probe",
        "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
        "host": args.host,
        "ports": {"relay": args.relay_port, "game": args.game_port, "silent": args.silent_port},
        "runner": {"egress_ip_masked": None, "platform": platform.platform(), "python": platform.python_version(), "github_run": os.environ.get("GITHUB_RUN_ID")},
        "targets": {},
    }
    if args.egress:
        report["runner"]["egress_ip_masked"] = runner_egress_ip(mask)
        log(f"runner egress (masked): {report['runner']['egress_ip_masked']}")
    if "game" not in skip and not aioquic_installed():
        log(f"note: aioquic is not installed, so the game server's encrypted handshake cannot be tested. To test it too: {AIOQUIC_INSTALL}")

    log(f"dns {args.host}")
    dns, ip = probe_dns(args.host, mask, min(args.timeout, 2.0))
    if "dns" not in skip:
        report["targets"]["dns"] = dns
    if "udp" not in skip:
        log("udp control (can this machine use UDP at all?)")
        report["targets"]["udp"] = probe_udp_control(min(args.timeout, 2.0))
    if ip is None:
        for name in ("relay", "game", "silent"):
            if name not in skip:
                report["targets"][name] = result(False, "not tried: the host name did not resolve", not_tried=True)
    else:
        if "relay" not in skip:
            log(f"relay {mask_ip(ip, 0, mask)}:{args.relay_port}")
            report["targets"]["relay"] = probe_relay(ip, args.relay_port, args.tries, args.timeout)
        if "game" not in skip:
            log(f"game (QUIC) {mask_ip(ip, 0, mask)}:{args.game_port}")
            report["targets"]["game"] = probe_game(ip, args.game_port, args.host, min(args.tries, 3), args.timeout, args.handshake_timeout, args.alpn,
                                                   args.fingerprint)
        if "silent" not in skip:
            log(f"silent port {mask_ip(ip, 0, mask)}:{args.silent_port}")
            report["targets"]["silent"] = probe_silent(ip, args.silent_port, args.timeout)

    failed = [t for t in required if t in report["targets"] and not report["targets"][t]["ok"]]
    # A failure while this machine cannot use UDP at all says nothing about the server: the watch must not open an issue for its own runner's network.
    # Nor does a name lookup that failed only in this machine's resolver (a public resolver found the address, and the other targets were probed there).
    about_server = [t for t in failed if not (t == "dns" and local_resolver_failed(report["targets"]))]
    inconclusive = bool(failed) and (bool(udp_blocked(report["targets"])) or not about_server)
    incomplete = bool(about_server) and not inconclusive and all(report["targets"][t].get("needs_aioquic") for t in about_server)
    report["required"] = required
    report["failed_required"] = failed
    report["ok"] = not failed
    report["inconclusive"] = inconclusive
    report["incomplete"] = incomplete
    report["seconds"] = round(time.time() - started, 1)
    report["diagnosis"] = diagnose(report)
    report["verdict"] = verdict_words(report)
    report = redact(report, mask)
    if not args.no_json:
        print(json.dumps(report, indent=2, sort_keys=False), flush=True)
    for name, t in report["targets"].items():
        log(f"{'ok  ' if t['ok'] else 'FAIL'} {name}: {t['detail']}")
    print("", file=sys.stderr)
    print(redact_text(render_plain(report)), file=sys.stderr, flush=True)
    if report["ok"]:
        return EXIT_OK
    if inconclusive:
        return EXIT_INCONCLUSIVE
    return EXIT_INCOMPLETE if incomplete else EXIT_FAILED


def run(argv=None):
    """`main`, but a crash in the probe itself is exit 70 with a traceback, never exit 1: 1 means "a required target is unreachable", and a
    caller (the workflow) must not open an issue claiming that because of a bug here."""
    try:
        return main(argv)
    except KeyboardInterrupt:
        return 130
    except Exception:
        log(traceback.format_exc().rstrip())  # through log: an exception's text can carry the probed address, and stderr is a public CI log
        return EXIT_INTERNAL


if __name__ == "__main__":
    sys.exit(run())
