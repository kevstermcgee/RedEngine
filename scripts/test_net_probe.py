#!/usr/bin/env python3
"""Tests for scripts/net_probe.py (the outside-in reachability probe) and for .github/workflows/reachability.yml (the hourly watch that runs it).

Run by `scripts/ci.sh pytools` (every scripts/test_*.py, on Linux and on Windows) or directly: python3 scripts/test_net_probe.py

Offline: nothing here touches the internet or the owner's server, and `ProbeCase` fails any test that tries (a connect, a sendto or a name lookup that is
not loopback, or a web request). The relay, the QUIC listener and the DNS resolver are small UDP servers on 127.0.0.1 in a thread; the QUIC handshake is a
canned answer (one class runs the real handshake against an aioquic server on loopback when aioquic is installed); the public resolvers of the "can this
machine use UDP" control are replaced by the fake. The workflow's own step scripts run under bash with a fake `gh` and a fake probe (Linux and macOS).

What is pinned, per the links a player cannot see:
  * the relay datagram (layout, the reply that counts, a late reply, silence, an ICMP refusal, no route) and that the constants match crates/red_core/src/net,
  * the UDP control, the QUIC Version Negotiation step, the handshake wording (an idle timeout is not "the server closed the connection"),
  * the plain-words line for each link and the verdict that combines them (DNS and the kind of address, the player's own UDP, the relay, the game port),
  * hiding: no address and no part of one in the JSON, the log, the plain words, or the Markdown a public issue is made of,
  * exit codes 0 / 1 / 2 / 3 / 4 / 70, and that only 1 means "unreachable" (a crash, a runner without UDP or a missing aioquic never does),
  * the workflow: hourly at an odd minute, dispatch, no push/pull_request trigger, least privilege, pinned actions; a failure confirmed by a second probe
    minutes later; one issue opened, commented at most every six hours, closed by a clean pass; an experiment never touches it.
"""
import contextlib
import io
import json
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import textwrap
import threading
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import net_probe as np  # noqa: E402

WORKFLOW = os.path.join(ROOT, ".github", "workflows", "reachability.yml")
CONTRIBUTORS = os.path.join(ROOT, ".github", "contributors.json")
RELAY_RS = os.path.join(ROOT, "crates", "red_core", "src", "net", "relay.rs")
QUIC_RS = os.path.join(ROOT, "crates", "red_core", "src", "net", "quic.rs")
WINDOWS = sys.platform == "win32"
FAKE_CERT = "sha256:" + "ab" * 32
ISSUE_TITLE = "Killchain and the relay are not reachable from the internet"
LOOPBACK = {"127.0.0.1", "localhost", "::1"}


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


# ---------------------------------------------------------------------------------------------------------------------------------------
# fakes


class FakeUdp:
    """A UDP server on 127.0.0.1 (a free port) in a thread. `handler(datagram, addr)` returns the reply bytes or None; every datagram is recorded."""

    def __init__(self, handler=None):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.05)
        self.port = self.sock.getsockname()[1]
        self.received = []
        self.handler = handler or (lambda data, addr: None)
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stop.is_set():
            try:
                data, addr = self.sock.recvfrom(65535)
            except socket.timeout:
                continue
            except OSError:  # Windows raises ConnectionResetError here after a reply went to a client that had already gone
                continue
            self.received.append(data)
            reply = self.handler(data, addr)
            if reply is not None:
                try:
                    self.sock.sendto(reply, addr)
                except OSError:
                    pass

    def close(self):
        self.stop.set()
        self.thread.join(2)
        self.sock.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def relay_says(reply):
    return lambda data, addr: reply if data == bytes([np.TAG_RESOLVE]) + np.IMPOSSIBLE_CODE else None


def make_version_negotiation(request):
    """What a QUIC listener answers an Initial with an unknown version: a Version Negotiation packet with the connection ids swapped."""
    dl = request[5]
    dcid = request[6:6 + dl]
    sl = request[6 + dl]
    scid = request[7 + dl:7 + dl + sl]
    return bytes([0x8A]) + b"\0\0\0\0" + bytes([len(scid)]) + scid + bytes([len(dcid)]) + dcid + struct.pack(">II", 1, 0xFF00001D)


def quic_listener(data, addr):
    return make_version_negotiation(data) if len(data) >= 1200 and data[0] & 0x80 else None


def dns_answers(data, addr):
    """A resolver: echoes the query id with the response bit set."""
    return data[:2] + bytes([data[2] | 0x80]) + data[3:] if len(data) >= 12 else None


def closed_udp_port():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def handshake_ok(*args, **kw):
    return {"ok": True, "error": None, "cert_sha256": FAKE_CERT, "alpn": "red/8", "handshake_ms": 12.0, "icmp_refused": False}


def handshake_fails(error):
    return lambda *args, **kw: {"ok": False, "error": error, "cert_sha256": None, "alpn": None, "handshake_ms": None, "icmp_refused": False}


def offline_only(addr):
    host = addr[0] if isinstance(addr, tuple) else addr
    if host not in LOOPBACK:
        raise AssertionError(f"a test tried to reach {host!r}: these tests run offline, against fakes on 127.0.0.1")


class ProbeCase(unittest.TestCase):
    """No pauses between tries, hiding back on after a test that turned it off, the probe's progress lines kept off the test output, and nothing that
    reaches past the loopback: a connect, a sendto or a name lookup elsewhere, or a web request, fails the test."""

    def setUp(self):
        real_connect, real_sendto, real_getaddrinfo = socket.socket.connect, socket.socket.sendto, socket.getaddrinfo

        def connect(sock, addr):
            offline_only(addr)
            return real_connect(sock, addr)

        def sendto(sock, data, *rest):
            offline_only(rest[-1])
            return real_sendto(sock, data, *rest)

        def getaddrinfo(host, *args, **kw):
            offline_only(host)
            return real_getaddrinfo(host, *args, **kw)

        for patch in (mock.patch.object(np, "PAUSE_BETWEEN_TRIES", 0),
                      mock.patch.object(np, "runner_egress_ip", side_effect=AssertionError("looked up the egress address")),
                      mock.patch.object(np.urllib.request, "urlopen", side_effect=AssertionError("a web request")),
                      mock.patch.object(socket.socket, "connect", connect), mock.patch.object(socket.socket, "sendto", sendto),
                      mock.patch.object(socket, "getaddrinfo", getaddrinfo)):
            patch.start()
            self.addCleanup(patch.stop)
        self.addCleanup(setattr, np, "MASKING", True)
        quiet = contextlib.redirect_stderr(io.StringIO())
        quiet.__enter__()
        self.addCleanup(quiet.__exit__, None, None, None)

    def run_main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                rc = np.main(argv)
            except SystemExit as e:
                rc = e.code
        return rc, out.getvalue(), err.getvalue()

    def world(self, *, relay=b"\x05", quic=True, dns=True, handshake=None, silent=None, aioquic=True):
        """Fake relay, QUIC listener, resolver and a control port on loopback, entered into an ExitStack. Returns (argv, servers)."""
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        if handshake is None:  # a dead listener cannot complete a handshake either
            handshake = handshake_ok if quic else handshake_fails("no completed handshake within 6 s")
        servers = {
            "relay": stack.enter_context(FakeUdp(relay_says(relay) if relay is not None else None)),
            "game": stack.enter_context(FakeUdp(quic_listener if quic else None)),
            "dns": stack.enter_context(FakeUdp(dns_answers if dns else None)),
            "silent": stack.enter_context(FakeUdp(silent)),
        }
        for patch in (mock.patch.object(np, "UDP_CONTROL_RESOLVERS", (("127.0.0.1", servers["dns"].port),)), mock.patch.object(np, "aioquic_installed", return_value=aioquic),
                      mock.patch.object(np, "quic_handshake_once", side_effect=handshake)):
            patch.start()
            self.addCleanup(patch.stop)
        argv = ["--host", "127.0.0.1", "--relay-port", str(servers["relay"].port), "--game-port", str(servers["game"].port), "--silent-port", str(servers["silent"].port),
                "--tries", "2", "--timeout", "0.25", "--no-mask"]
        return argv, servers


class Offline(ProbeCase):
    def test_a_test_that_reaches_past_the_loopback_fails(self):
        for attempt in (lambda: np.udp_exchange("192.0.2.1", 28016, b"x", 0.1), lambda: socket.getaddrinfo("example.org", None),
                        lambda: np.urllib.request.urlopen("https://example.org")):
            with self.subTest(attempt=attempt), self.assertRaises(AssertionError):
                attempt()


# ---------------------------------------------------------------------------------------------------------------------------------------
# the relay


class Relay(ProbeCase):
    def test_the_resolve_datagram_is_tag_3_then_a_code_that_cannot_exist_and_code_not_found_is_the_answer_that_counts(self):
        with FakeUdp(relay_says(b"\x05")) as relay:
            r = np.probe_relay("127.0.0.1", relay.port, 3, 2.0)
        self.assertEqual(relay.received, [b"\x03" + b"000000"])
        self.assertTrue(r["ok"], r)
        self.assertIn("CodeNotFound", r["detail"])
        self.assertEqual(r["attempts"], ["reply"])

    def test_the_constants_are_the_ones_the_rust_relay_and_the_game_use(self):
        if not (os.path.exists(RELAY_RS) and os.path.exists(QUIC_RS)):
            self.skipTest("the Rust sources are not beside this script")
        relay, quic = read(RELAY_RS), read(QUIC_RS)
        self.assertEqual(int(re.search(r"const TAG_RESOLVE: u8 = (\d+);", relay).group(1)), np.TAG_RESOLVE)
        self.assertEqual(int(re.search(r"const TAG_CODE_NOT_FOUND: u8 = (\d+);", relay).group(1)), np.TAG_CODE_NOT_FOUND)
        self.assertEqual(int(re.search(r"pub const CODE_LEN: usize = (\d+);", relay).group(1)), np.CODE_LEN)
        self.assertEqual(len(np.IMPOSSIBLE_CODE), np.CODE_LEN)
        alphabet = re.search(r'const ALPHABET: &\[u8\] = b"([A-Z0-9]+)";', relay).group(1)
        self.assertTrue(all(chr(c) not in alphabet for c in np.IMPOSSIBLE_CODE), "the probe's code must not be one the relay can hand out")
        self.assertEqual(re.search(r'pub const DEFAULT_RELAY: &str = "([^"]+)";', relay).group(1), f"{np.OWNER_HOST}:{np.RELAY_PORT}")
        self.assertEqual(re.search(r'pub const ALPN: &\[u8\] = b"([^"]+)";', quic).group(1), np.ALPN)

    def test_a_reply_that_is_not_code_not_found_fails_and_says_what_arrived(self):
        for wrong in (b"\x04abc", b"\x05\x00", b"hello"):
            with self.subTest(wrong=wrong), FakeUdp(relay_says(wrong)) as relay:
                r = np.probe_relay("127.0.0.1", relay.port, 2, 2.0)
                self.assertFalse(r["ok"])
                self.assertIn("not with CodeNotFound", r["detail"])
                self.assertIn(wrong[:16].hex(), r["detail"])

    def test_silence_is_retried_and_reported_as_dropped_on_the_way_in(self):
        with FakeUdp() as relay:
            t0 = time.monotonic()
            r = np.probe_relay("127.0.0.1", relay.port, 2, 0.2)
            took = time.monotonic() - t0
            self.assertEqual(len(relay.received), 2, "one datagram per try")
        self.assertFalse(r["ok"])
        self.assertEqual(r["attempts"], ["silent", "silent"])
        self.assertIn("dropped on the way in", r["detail"])
        self.assertLess(took, 5, "bounded by tries x timeout")

    def test_a_late_reply_on_a_later_try_still_passes(self):
        seen = []

        def second_time(data, addr):
            seen.append(data)
            return b"\x05" if len(seen) >= 2 else None

        with FakeUdp(second_time) as relay:
            r = np.probe_relay("127.0.0.1", relay.port, 3, 0.4)
        self.assertTrue(r["ok"], r)
        self.assertIn("try 2/3", r["detail"])
        self.assertEqual(r["attempts"], ["silent", "reply"])

    def test_a_port_with_no_listener_is_reported_as_refused_where_the_os_reports_it(self):
        r = np.probe_relay("127.0.0.1", closed_udp_port(), 2, 0.4)
        self.assertFalse(r["ok"])
        # Linux and macOS surface the ICMP port-unreachable as ECONNREFUSED, Windows as WSAECONNRESET (both are "refused"); an OS that drops it says silent.
        self.assertIn(r["attempts"][0], ("refused", "silent") if WINDOWS else ("refused",))
        if r["attempts"][0] == "refused":
            self.assertIn("ICMP unreachable came back", r["detail"])

    def test_no_route_stops_after_one_try_and_says_so(self):
        with mock.patch.object(np, "udp_exchange", return_value=("unreachable:ENETUNREACH", b"", 0.0)) as ex:
            r = np.probe_relay("192.0.2.1", np.RELAY_PORT, 4, 1.0)
        self.assertEqual(ex.call_count, 1)
        self.assertIn("no route to the host", r["detail"])
        self.assertIn("ENETUNREACH", r["detail"])

    def test_a_send_error_is_an_outcome_not_a_crash(self):
        with mock.patch.object(socket.socket, "send", side_effect=OSError(101, "Network is unreachable")):
            outcome, data, _ = np.udp_exchange("127.0.0.1", 9, b"x", 0.2)
        self.assertTrue(outcome.startswith("unreachable:"), outcome)
        self.assertEqual(data, b"")

    def test_windows_reports_an_icmp_refusal_as_a_connection_reset(self):
        # recv raising ConnectionResetError (what a connected UDP socket does on Windows) must read as "refused", like ConnectionRefusedError on Linux.
        with mock.patch.object(socket.socket, "recv", side_effect=ConnectionResetError(10054, "forcibly closed")), \
                mock.patch.object(np.select, "select", return_value=([object()], [], [])), mock.patch.object(socket.socket, "send", return_value=1), \
                mock.patch.object(socket.socket, "connect", return_value=None):
            outcome, _, _ = np.udp_exchange("127.0.0.1", 9, b"x", 0.5)
        self.assertEqual(outcome, "refused")


# ---------------------------------------------------------------------------------------------------------------------------------------
# the UDP control and the QUIC steps


class Control(ProbeCase):
    def test_a_resolver_that_answers_proves_this_machine_can_use_udp(self):
        with FakeUdp(dns_answers) as dns, mock.patch.object(np, "UDP_CONTROL_RESOLVERS", (("127.0.0.1", dns.port),)):
            r = np.probe_udp_control(2.0)
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["resolver"], "127.0.0.1")
        query = dns.received[0]
        self.assertEqual(struct.unpack(">HHHHHH", query[:12])[1:], (0x0100, 1, 0, 0, 0), "a standard recursive query with one question")
        self.assertIn(b"\x07example\x03com\x00", query)

    def test_an_answer_for_another_query_does_not_count(self):
        wrong_id = lambda data, addr: bytes([data[0] ^ 0xFF, data[1]]) + bytes([data[2] | 0x80]) + data[3:]
        with FakeUdp(wrong_id) as dns, mock.patch.object(np, "UDP_CONTROL_RESOLVERS", (("127.0.0.1", dns.port),)):
            r = np.probe_udp_control(0.2)
        self.assertFalse(r["ok"])

    def test_every_resolver_silent_means_udp_is_blocked_and_the_next_resolver_is_tried_first(self):
        with FakeUdp() as one, FakeUdp(dns_answers) as two, mock.patch.object(np, "UDP_CONTROL_RESOLVERS", (("127.0.0.1", one.port), ("127.0.0.1", two.port))):
            r = np.probe_udp_control(0.2)
            self.assertTrue(r["ok"], "the second resolver answered")
        with FakeUdp() as one, FakeUdp() as two, mock.patch.object(np, "UDP_CONTROL_RESOLVERS", (("127.0.0.1", one.port), ("127.0.0.1", two.port))):
            r = np.probe_udp_control(0.2)
        self.assertFalse(r["ok"])
        self.assertIn("seems to block outgoing UDP", r["detail"])
        self.assertFalse(r["no_route"])


class Quic(ProbeCase):
    def test_the_version_negotiation_probe_is_a_1200_byte_long_header_with_a_reserved_version(self):
        packet, dcid, scid = np.version_negotiation_request()
        self.assertEqual(len(packet), 1200)
        self.assertTrue(packet[0] & 0x80, "long header")
        self.assertEqual(packet[1:5], (0x0A1A2A3A).to_bytes(4, "big"), "a reserved ('greased') version no server speaks")
        self.assertEqual(packet[5], 8)
        self.assertEqual(packet[6:14], dcid)

    def test_a_live_listener_answers_with_its_versions_and_a_stray_datagram_is_ignored(self):
        with FakeUdp(quic_listener) as game:
            r = np.probe_quic_listener("127.0.0.1", game.port, 2, 2.0)
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["versions"], ["0x00000001", "0xff00001d"])
        stray = lambda data, addr: b"\x80\0\0\0\0\x01a\x01b" + struct.pack(">I", 1)  # a Version Negotiation for somebody else
        with FakeUdp(stray) as game:
            r = np.probe_quic_listener("127.0.0.1", game.port, 1, 0.2)
        self.assertFalse(r["ok"])
        self.assertEqual(r["attempts"], ["silent"])

    def test_the_handshake_wording_does_not_blame_the_server_for_the_local_idle_timer(self):
        stalled = np.describe_close(1, "Idle timeout", 6.0)
        self.assertIn("stalled", stalled)
        self.assertNotIn("server closed", stalled)
        self.assertIn("TLS alert 120", np.describe_close(0x178, "", 6.0))
        self.assertIn("different ALPN", np.describe_close(0x178, "", 6.0))
        self.assertIn("(No common ALPN protocols)", np.describe_close(0x128, "No common ALPN protocols", 6.0))
        self.assertIn("error 0xa", np.describe_close(0xA, "bad", 6.0))

    def test_fingerprints_compare_ignoring_case_colons_and_the_prefix(self):
        self.assertTrue(np.same_fingerprint(FAKE_CERT, FAKE_CERT.upper()))
        self.assertTrue(np.same_fingerprint("ab" * 32, FAKE_CERT))
        self.assertFalse(np.same_fingerprint("sha256:" + "cd" * 32, FAKE_CERT))
        self.assertFalse(np.same_fingerprint("sha256:" + "cd" * 32, None))

    def test_a_handshake_with_the_wrong_certificate_fails_the_game_target_when_a_fingerprint_was_given(self):
        with FakeUdp(quic_listener) as game, mock.patch.object(np, "aioquic_installed", return_value=True), mock.patch.object(np, "quic_handshake_once", side_effect=handshake_ok):
            right = np.probe_game("127.0.0.1", game.port, "h", 1, 1.0, 1.0, np.ALPN, FAKE_CERT)
            wrong = np.probe_game("127.0.0.1", game.port, "h", 1, 1.0, 1.0, np.ALPN, "sha256:" + "cd" * 32)
            none = np.probe_game("127.0.0.1", game.port, "h", 1, 1.0, 1.0, np.ALPN)
        self.assertTrue(right["ok"] and right["fingerprint_match"])
        self.assertFalse(wrong["ok"])
        self.assertFalse(wrong["fingerprint_match"])
        self.assertIn("not the fingerprint you gave", wrong["detail"])
        self.assertTrue(none["ok"] and "fingerprint_match" not in none)

    def test_without_aioquic_the_game_target_says_how_to_install_it_and_still_does_version_negotiation(self):
        with FakeUdp(quic_listener) as game, mock.patch.object(np, "aioquic_installed", return_value=False):
            r = np.probe_game("127.0.0.1", game.port, "h", 1, 1.0, 1.0, np.ALPN)
        self.assertFalse(r["ok"])
        self.assertTrue(r["needs_aioquic"])
        self.assertIn("pip install aioquic==1.2.0", r["detail"])
        self.assertTrue(r["listener"]["ok"], "step 1 (stdlib only) still ran")


# ---------------------------------------------------------------------------------------------------------------------------------------
# main: exit codes, JSON, plain words


class Main(ProbeCase):
    def parse(self, rc_out_err):
        rc, out, err = rc_out_err
        return rc, json.loads(out), err

    def test_everything_reachable_is_exit_0_one_json_object_and_an_ok_verdict(self):
        argv, _ = self.world()
        rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, 0, err)
        self.assertTrue(report["ok"])
        self.assertEqual(report["failed_required"], [])
        self.assertEqual(list(report["targets"]), ["dns", "udp", "relay", "game", "silent"])
        self.assertEqual(report["targets"]["game"]["cert_sha256"], FAKE_CERT)
        self.assertEqual(report["targets"]["dns"]["address_kind"], "loopback")
        self.assertEqual([d["link"] for d in report["diagnosis"]], ["dns", "udp", "relay", "game"])
        self.assertTrue(report["verdict"].startswith("OK:"))
        self.assertIn("RESULT for 127.0.0.1", err)
        self.assertIn("OK: from this machine the relay and the game server can both be reached", err)

    def test_no_json_leaves_stdout_empty_and_the_words_on_stderr(self):
        argv, _ = self.world()
        rc, out, err = self.run_main(argv + ["--no-json"])
        self.assertEqual((rc, out), (0, ""))
        self.assertIn("RESULT for", err)

    def test_a_dead_relay_is_exit_1_and_named_as_the_relay(self):
        argv, _ = self.world(relay=None)
        rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, 1)
        self.assertEqual(report["failed_required"], ["relay"])
        self.assertTrue(report["verdict"].startswith("FAILED: the game server answers but the relay does not"), report["verdict"])
        relay = next(d for d in report["diagnosis"] if d["link"] == "relay")
        self.assertFalse(relay["ok"])
        self.assertIn("relay is stopped", relay["words"])
        self.assertIn("although UDP works from here", relay["words"])
        self.assertIn("Direct join codes", relay["words"])
        self.assertIn("[FAIL] Relay (short codes)", err)

    def test_a_dead_game_port_is_exit_1_and_named_as_the_game_server(self):
        argv, _ = self.world(quic=False)
        rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, 1)
        self.assertEqual(report["failed_required"], ["game"])
        self.assertTrue(report["verdict"].startswith("FAILED: the relay answers but the game server does not"), report["verdict"])
        game = next(d for d in report["diagnosis"] if d["link"] == "game")
        self.assertIn("game server is stopped", game["words"])
        self.assertTrue(game["words"].endswith("Tell the owner."))

    def test_a_listener_that_will_not_finish_the_handshake_is_not_called_stopped(self):
        argv, _ = self.world(handshake=handshake_fails("the handshake stalled: the server stopped answering and nothing completed within 6 s"))
        rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, 1)
        game = next(d for d in report["diagnosis"] if d["link"] == "game")
        self.assertIn("a QUIC listener answers", game["words"])
        self.assertIn("the handshake stalled", game["words"])
        self.assertIn("not accepting connections", game["words"])

    def test_both_down_while_udp_works_points_at_the_owners_connection(self):
        argv, _ = self.world(relay=None, quic=False)
        rc, report, _ = self.parse(self.run_main(argv))
        self.assertEqual(rc, 1)
        self.assertEqual(sorted(report["failed_required"]), ["game", "relay"])
        self.assertIn("neither the relay nor the game server answers although UDP works from here", report["verdict"])
        self.assertIn("home connection", report["verdict"])

    def test_a_name_that_points_at_a_private_address_is_named_as_the_cause_without_showing_the_address(self):
        argv, _ = self.world()
        argv[1] = "lan.example.invalid"
        argv.remove("--no-mask")
        down = np.result(False, "no reply", None, attempts=["silent", "silent"], listener={"ok": False, "attempts": ["silent"]})
        answer = [(socket.AF_INET, socket.SOCK_DGRAM, 17, "", ("192.168.0.10", 0))]
        with mock.patch.object(socket, "getaddrinfo", return_value=answer), mock.patch.object(np, "probe_relay", return_value=down), \
                mock.patch.object(np, "probe_game", return_value=down), mock.patch.object(np, "probe_silent", return_value=np.result(True, "silence")):
            rc, out, err = self.run_main(argv)
        report = json.loads(out)
        self.assertEqual(rc, 1)
        self.assertEqual(report["targets"]["dns"]["address_kind"], "private")
        self.assertIn("points at a private (home network) address", report["verdict"])
        self.assertIn("the name's record is wrong", report["verdict"])
        words = {d["link"]: d["words"] for d in report["diagnosis"]}
        self.assertIn("nobody on the internet can reach", words["dns"])
        self.assertNotIn("192.168", out + err)
        # Given as an address on purpose (a player probing a LAN server), a private address is what they meant, not a wrong record.
        report = dict(report, host="192.168.0.10")
        self.assertIsNone(np.wrong_record(report, report["targets"]["dns"]))
        self.assertNotIn("record is wrong", np.verdict_words(report))
        self.assertNotIn("record", np.words_dns(report["targets"]["dns"], "192.168.0.10"))

    def test_address_kinds(self):
        for ip, kind in (("8.8.8.8", "public"), ("192.168.0.10", "private"), ("10.1.2.3", "private"), ("100.64.1.2", "shared"), ("127.0.0.1", "loopback"),
                         ("not an address", "other")):
            with self.subTest(ip=ip):
                self.assertEqual(np.address_kind(ip), kind)

    def test_when_this_machines_own_udp_is_blocked_the_result_is_inconclusive_exit_4_not_unreachable(self):
        # The watch opens an issue only for exit 1: a runner whose own network drops UDP must not report the owner's server as down.
        argv, _ = self.world(relay=None, quic=False, dns=False)
        rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, np.EXIT_INCONCLUSIVE)
        self.assertNotEqual(rc, np.EXIT_FAILED)
        self.assertTrue(report["inconclusive"] and not report["ok"] and not report["incomplete"])
        self.assertFalse(report["targets"]["udp"]["ok"])
        self.assertTrue(report["verdict"].startswith("INCONCLUSIVE"), report["verdict"])
        self.assertIn("this machine's own network", report["verdict"])
        words = {d["link"]: d["words"] for d in report["diagnosis"]}
        self.assertIn("blocks outgoing UDP", words["udp"])
        self.assertIn("another network such as a phone hotspot", words["udp"])
        self.assertIn("says nothing about the relay", words["relay"])
        self.assertIn("says nothing about the game server", words["game"])
        self.assertNotIn("Tell the owner", words["relay"] + words["game"])
        self.assertTrue(np.render_markdown(report, "summary").startswith("### Reachability of x.x.x.x: INCONCLUSIVE"))

    def test_a_blocked_udp_control_does_not_matter_when_the_server_answers(self):
        argv, _ = self.world(dns=False)
        rc, report, _ = self.parse(self.run_main(argv))
        self.assertEqual(rc, 0, "the control is not required: the server answered, so it is reachable")
        self.assertFalse(report["inconclusive"])

    def test_a_name_that_does_not_resolve_stops_there_and_says_so(self):
        argv, _ = self.world()
        argv[1] = "no-such-host.example.invalid"
        with mock.patch.object(np.socket, "getaddrinfo", side_effect=socket.gaierror(-2, "Name or service not known")):
            rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, 1)
        self.assertEqual(report["failed_required"], ["dns", "relay", "game"])
        self.assertTrue(report["verdict"].startswith("FAILED at the first step"))
        words = {d["link"]: d["words"] for d in report["diagnosis"]}
        self.assertIn("did not resolve", words["dns"])
        self.assertIn("DuckDNS", words["dns"])
        self.assertEqual(words["relay"], "not tried: the name lookup failed first.")
        self.assertTrue(report["targets"]["udp"]["ok"], "the UDP control does not need the name")

    def test_a_missing_aioquic_is_exit_3_incomplete_with_the_install_command(self):
        argv, _ = self.world(aioquic=False)
        rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, 3)
        self.assertTrue(report["incomplete"] and not report["ok"])
        self.assertTrue(report["verdict"].startswith("INCOMPLETE"))
        self.assertIn("python3 -m pip install aioquic==1.2.0", err)
        self.assertIn("py -m pip install aioquic==1.2.0", err, "the Windows spelling")
        self.assertIn("NOT fully tested", err)
        self.assertIn("note: aioquic is not installed", err, "said before the probe starts, not only at the end")

    def test_missing_aioquic_does_not_hide_a_real_failure(self):
        argv, _ = self.world(aioquic=False, relay=None)
        rc, report, err = self.parse(self.run_main(argv))
        self.assertEqual(rc, 1, "a dead relay is a failure, not an incomplete run")
        self.assertFalse(report["incomplete"])

    def test_bad_arguments_are_exit_2(self):
        for argv in (["--require", "relay,telepathy"], ["--skip", "nonsense"], ["--tries", "0"], ["--no-such-option"], ["--relay-port", "x"]):
            with self.subTest(argv=argv):
                rc, out, err = self.run_main(argv)
                self.assertEqual(rc, 2)
                self.assertEqual(out, "")

    def test_a_crash_in_the_probe_is_exit_70_never_exit_1(self):
        err = io.StringIO()
        with mock.patch.object(np, "main", side_effect=RuntimeError("bug")), contextlib.redirect_stderr(err):
            rc = np.run([])
        self.assertEqual(rc, np.EXIT_INTERNAL)
        self.assertNotEqual(rc, np.EXIT_FAILED)
        self.assertIn("RuntimeError: bug", err.getvalue())

    def test_only_the_required_targets_decide_the_exit_code(self):
        argv, _ = self.world(silent=lambda data, addr: b"who goes there")
        rc, report, _ = self.parse(self.run_main(argv))
        self.assertEqual(rc, 0, "an answering control port is reported but not required")
        self.assertFalse(report["targets"]["silent"]["ok"])
        self.assertIn("UNEXPECTED", report["targets"]["silent"]["detail"])
        self.assertEqual([d["link"] for d in report["diagnosis"]], ["dns", "udp", "relay", "game", "silent"], "an unexpected answer is shown")
        argv, _ = self.world(silent=lambda data, addr: b"who goes there")
        rc, report, _ = self.parse(self.run_main(argv + ["--require", "dns,relay,game,silent"]))
        self.assertEqual((rc, report["failed_required"]), (1, ["silent"]))

    def test_skip_leaves_a_target_out_entirely(self):
        argv, _ = self.world()
        rc, report, _ = self.parse(self.run_main(argv + ["--skip", "udp,silent"]))
        self.assertEqual(rc, 0)
        self.assertEqual(list(report["targets"]), ["dns", "relay", "game"])

    def test_the_egress_lookup_asks_third_parties_only_when_told_to(self):
        argv, _ = self.world()
        rc, report, _ = self.parse(self.run_main(argv))  # setUp's patch raises if it is called
        self.assertEqual(rc, 0)
        self.assertIsNone(report["runner"]["egress_ip_masked"])
        with mock.patch.object(np, "runner_egress_ip", return_value="9.9.x.x") as lookup:
            rc, report, _ = self.parse(self.run_main(argv + ["--egress"]))
        self.assertEqual(lookup.call_count, 1)
        self.assertEqual(report["runner"]["egress_ip_masked"], "9.9.x.x")

    def test_the_defaults_are_the_owners_public_server_and_no_secret(self):
        args = np.build_parser().parse_args([])
        self.assertEqual((args.host, args.relay_port, args.game_port), (np.OWNER_HOST, 28016, 27015))
        self.assertEqual(args.host, "red-engine.duckdns.org")
        self.assertIsNone(args.fingerprint, "a fingerprint changes when the server's identity is remade: no default")
        for path in (os.path.join(HERE, "net_probe.py"), WORKFLOW):
            source = read(path)
            with self.subTest(path=os.path.basename(path)):
                self.assertNotRegex(source, r"(?i)\b(key|secret|token|password)\s*=\s*[\"'][^\"']+[\"']")
                addresses = set(re.findall(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b", source))
                self.assertLessEqual(addresses, {"1.1.1.1", "8.8.8.8", "9.9.9.9", "127.0.0.1", "203.0.113.45", "100.64.0.0"},
                                     "no address is committed (the resolvers of the UDP control, loopback, the documentation address in examples and the CGNAT range)")


# ---------------------------------------------------------------------------------------------------------------------------------------
# hiding addresses


class Hiding(ProbeCase):
    def test_mask_ip(self):
        self.assertEqual(np.mask_ip("203.0.113.45", 0), "x.x.x.x")
        self.assertEqual(np.mask_ip("203.0.113.45", 2), "203.0.x.x")
        self.assertEqual(np.mask_ip("203.0.113.45", 0, enabled=False), "203.0.113.45")
        self.assertEqual(np.mask_ip("example.org", 0), "example.org")

    def test_redact_text_hides_every_address_whole_but_not_versions_times_or_the_public_resolvers(self):
        self.assertEqual(np.redact_text("boom 203.0.113.45 and 198.51.100.7:27015"), "boom x.x.x.x and x.x.x.x:27015")
        for untouched in ("aioquic 1.2.0, Python 3.12.3, Linux-6.8.0-1017-azure", "1.1.1.1 answered, 9.9.9.9 too", "2026-10-20T17:55:39.8362848Z",
                          "red-engine.duckdns.org", FAKE_CERT, '"rtt_ms": 59.1, "ok": true', "red_core::net::relay"):
            with self.subTest(untouched=untouched):
                self.assertEqual(np.redact_text(untouched), untouched)
        self.assertEqual(np.redact_text("203.0.113.45", enabled=False), "203.0.113.45")
        self.assertEqual(np.redact({"a": ["10.1.2.3", {"b": "10.1.2.3"}], "n": 5}), {"a": ["x.x.x.x", {"b": "x.x.x.x"}], "n": 5})

    def test_an_ipv6_address_in_an_error_text_is_hidden_too(self):
        for text in ("connect to 2001:db8::1 failed", "peer 2001:0db8:85a3:0000:0000:8a2e:0370:7334 port 27015", "bound to ::1"):
            with self.subTest(text=text):
                hidden = np.redact_text(text)
                self.assertIn(np.HIDDEN_IPV6, hidden)
                self.assertNotRegex(hidden, r"2001|7334|::1")

    def test_the_default_run_prints_no_address_and_no_part_of_one(self):
        argv, _ = self.world()
        argv.remove("--no-mask")
        rc, out, err = self.run_main(argv)
        self.assertEqual(rc, 0, err)
        self.assertNotIn("127.0.0", out + err, "JSON, progress and plain words: not even the first octets (the fake resolver of the UDP control included)")
        self.assertEqual(np.keep_unmasked(), {"1.1.1.1", "8.8.8.8", "9.9.9.9"}, "only the well-known public resolvers are shown, whatever the control is pointed at")
        self.assertEqual(json.loads(out)["targets"]["dns"]["detail"], "x.x.x.x -> x.x.x.x (loopback)", "the host itself too, when it is given as an address")
        rc, out, err = self.run_main(argv + ["--no-mask"])
        self.assertIn("127.0.0.1", out, "--no-mask is the way to see it")

    def test_an_address_inside_an_error_text_is_hidden_too(self):
        leaky = np.result(False, "OSError: [Errno 113] No route to host (203.0.113.45:27015)", None, port=1, attempts=["silent"])
        argv, _ = self.world()
        argv.remove("--no-mask")
        with mock.patch.object(np, "probe_relay", return_value=leaky):
            rc, out, err = self.run_main(argv)
        self.assertEqual(rc, 1)
        self.assertNotIn("203.0.113", out + err)
        self.assertIn("x.x.x.x:27015", out)
        report = json.loads(out)
        self.assertNotIn("203.0.113", np.render_markdown(report, "issue", "https://example.invalid/run/1"))

    def test_markdown_hides_a_result_file_that_was_not_hidden_even_after_a_no_mask_run(self):
        report = {"host": "h", "ok": False, "started_utc": "2026-10-20T10:17:00Z", "ports": {}, "runner": {"egress_ip_masked": "198.51.100.77"},
                  "targets": {"relay": {"ok": False, "rtt_ms": None, "detail": "answer from 203.0.113.45 was wrong"}}}
        np.MASKING = False  # what a --no-mask run in the same process leaves behind; the cleanup turns it back on
        for style in ("summary", "issue", "comment"):
            with self.subTest(style=style):
                text = np.render_markdown(report, style)
                self.assertNotIn("203.0.113", text)
                self.assertNotIn("198.51.100", text)
                self.assertIn("x.x.x.x", text)


# ---------------------------------------------------------------------------------------------------------------------------------------
# the Markdown of the job summary and the issue


class MarkdownText(ProbeCase):
    def failed_report(self):
        """A dead relay, probed as the workflow probes: addresses hidden."""
        argv, _ = self.world(relay=None)
        argv.remove("--no-mask")
        rc, out, _ = self.run_main(argv)
        self.assertEqual(rc, 1)
        return json.loads(out)

    def test_issue_body_has_the_run_link_the_verdict_the_table_the_json_and_the_mentions(self):
        report = self.failed_report()
        body = np.render_markdown(report, "issue", "https://github.com/o/r/actions/runs/42", "kevstermcgee,meastwood13")
        self.assertIn("on two probes a few minutes apart (failed: Relay (short codes)).", body)
        self.assertIn("\ncc @kevstermcgee @meastwood13\n", body)
        self.assertIn("Run: https://github.com/o/r/actions/runs/42", body)
        self.assertIn(f"**{report['verdict']}**", body)
        self.assertIn("| Relay (short codes) | NO |", body)
        self.assertIn("| Game server | yes |", body)
        self.assertIn("docs/HOSTING.md", body)
        self.assertIn("python3 scripts/net_probe.py --no-json", body)
        embedded = re.search(r"```json\n(.*?)\n```", body, re.S).group(1)
        self.assertEqual(json.loads(embedded), report, "the JSON in the issue is the probe's result")

    def test_a_name_that_is_not_a_github_login_is_left_out_not_an_error(self):
        report = self.failed_report()
        body = np.render_markdown(report, "issue", None, "good-name, bad name,@x,-dash, " + "y" * 40)
        self.assertIn("cc @good-name\n", body)
        self.assertNotIn("cc @kevstermcgee", np.render_markdown(report, "issue", None, ""))
        self.assertNotIn("\ncc ", np.render_markdown(report, "issue", None, None))
        self.assertNotIn("\ncc ", np.render_markdown(report, "comment", None, "kevstermcgee"), "only the issue mentions: participants hear of comments anyway")

    def test_summary_and_comment_styles(self):
        report = self.failed_report()
        summary = np.render_markdown(report, "summary")
        self.assertTrue(summary.startswith("### Reachability of x.x.x.x: FAILED"), summary[:80])
        comment = np.render_markdown(report, "comment", "https://x.invalid/run/9")
        self.assertTrue(comment.startswith("Still unreachable at "))
        self.assertIn("https://x.invalid/run/9", comment)
        self.assertNotIn("two probes", comment)
        ok = np.render_markdown(dict(report, ok=True, failed_required=[]), "summary")
        self.assertTrue(ok.startswith("### Reachability of x.x.x.x: ok"))

    def test_a_table_cell_cannot_break_the_table(self):
        self.assertEqual(np.cell("a | b\nc"), "a \\| b c")

    def test_markdown_mode_renders_a_saved_result_and_refuses_a_missing_or_bad_one(self):
        report = self.failed_report()
        with tempfile.TemporaryDirectory() as d:
            good = os.path.join(d, "probe.json")
            with open(good, "w", encoding="utf-8") as f:
                json.dump(report, f)
            rc, out, _ = self.run_main(["--markdown", good, "--style", "issue", "--run-url", "https://x.invalid/run/1", "--mention", "meastwood13"])
            self.assertEqual(rc, 0)
            self.assertIn("Run: https://x.invalid/run/1", out)
            self.assertIn("cc @meastwood13", out)
            bad = os.path.join(d, "bad.json")
            with open(bad, "w", encoding="utf-8") as f:
                f.write("{not json")
            for path in (bad, os.path.join(d, "missing.json")):
                rc, out, err = self.run_main(["--markdown", path])
                self.assertEqual((rc, out), (2, ""))
                self.assertIn("cannot render", err)


# ---------------------------------------------------------------------------------------------------------------------------------------
# a real handshake, when aioquic is installed (the CI runners do not have it: this runs on a developer machine)


def aioquic_available():
    try:
        import aioquic  # noqa: F401
        import cryptography  # noqa: F401
        return True
    except ImportError:
        return False


@unittest.skipUnless(aioquic_available(), "aioquic is not installed (python3 -m pip install aioquic==1.2.0)")
class RealHandshake(ProbeCase):
    def serve(self, alpn):
        """An aioquic server on loopback with a throwaway self-signed certificate. Returns (port, fingerprint)."""
        import asyncio
        import datetime
        import hashlib
        from aioquic.asyncio import serve
        from aioquic.asyncio.protocol import QuicConnectionProtocol
        from aioquic.quic.configuration import QuicConfiguration
        from cryptography import x509
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import ec
        from cryptography.x509.oid import NameOID

        key = ec.generate_private_key(ec.SECP256R1())
        name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "net-probe-test")])
        now = datetime.datetime.now(datetime.timezone.utc)
        cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key()).serial_number(x509.random_serial_number())
                .not_valid_before(now - datetime.timedelta(days=1)).not_valid_after(now + datetime.timedelta(days=1)).sign(key, hashes.SHA256()))
        tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmp, True)
        cert_path, key_path = os.path.join(tmp, "cert.pem"), os.path.join(tmp, "key.pem")
        with open(cert_path, "wb") as f:
            f.write(cert.public_bytes(serialization.Encoding.PEM))
        with open(key_path, "wb") as f:
            f.write(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
        config = QuicConfiguration(is_client=False, alpn_protocols=[alpn])
        config.load_cert_chain(cert_path, key_path)
        fingerprint = "sha256:" + hashlib.sha256(cert.public_bytes(serialization.Encoding.DER)).hexdigest()
        started = threading.Event()
        box = {}

        def thread():
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            server = loop.run_until_complete(serve("127.0.0.1", 0, configuration=config, create_protocol=QuicConnectionProtocol))
            box["port"] = server._transport.get_extra_info("sockname")[1]
            box["loop"] = loop
            started.set()
            loop.run_forever()
            server.close()
            loop.close()

        t = threading.Thread(target=thread, daemon=True)
        t.start()
        self.assertTrue(started.wait(10), "the aioquic test server did not start")

        def stop():
            box["loop"].call_soon_threadsafe(box["loop"].stop)
            t.join(5)

        self.addCleanup(stop)
        return box["port"], fingerprint

    def test_the_handshake_completes_and_reports_the_certificate_the_clients_pin(self):
        port, fingerprint = self.serve(np.ALPN)
        r = np.quic_handshake_once("127.0.0.1", port, "net-probe-test", np.ALPN, 8.0)
        self.assertTrue(r["ok"], r)
        self.assertEqual(r["alpn"], np.ALPN)
        self.assertEqual(r["cert_sha256"], fingerprint)
        game = np.probe_game("127.0.0.1", port, "net-probe-test", 2, 3.0, 8.0, np.ALPN, fingerprint)
        self.assertTrue(game["ok"], game)
        self.assertTrue(game["fingerprint_match"])

    def test_a_server_that_speaks_another_alpn_is_refused_with_the_alert_that_says_so(self):
        port, _ = self.serve("something-else/1")
        r = np.quic_handshake_once("127.0.0.1", port, "net-probe-test", np.ALPN, 8.0)
        self.assertFalse(r["ok"])
        # aioquic answers a missing common ALPN with handshake_failure (40) and says why in the reason phrase; a Rust server (rustls) uses alert 120.
        self.assertIn("TLS alert", r["error"], r)
        self.assertIn("No common ALPN", r["error"])

    def test_nothing_listening_is_an_icmp_refusal_or_a_timeout_never_a_crash(self):
        r = np.quic_handshake_once("127.0.0.1", closed_udp_port(), "x", np.ALPN, 1.0)
        self.assertFalse(r["ok"])
        self.assertTrue(r["icmp_refused"] or "within" in r["error"], r)


# ---------------------------------------------------------------------------------------------------------------------------------------
# the workflow: what must stay true of its shape (text checks that run everywhere, structure checks when PyYAML is installed)


def workflow_text():
    return read(WORKFLOW)


def top_level_block(text, key):
    """The indented lines under a top-level `key:` (comments and blank lines dropped)."""
    lines = text.splitlines()
    start = next(i for i, l in enumerate(lines) if re.match(rf"^{re.escape(key)}:\s*$", l))
    block = []
    for l in lines[start + 1:]:
        if l and not l[0].isspace() and not l.startswith("#"):
            break
        if l.strip() and not l.strip().startswith("#"):
            block.append(l)
    return block


def steps(text):
    """Each step of the job as its text (a step starts at `      - ` under `    steps:`)."""
    lines = text.splitlines()
    first = next(i for i, l in enumerate(lines) if l == "    steps:")
    out, cur = [], None
    for l in lines[first + 1:]:
        if re.match(r"^      - ", l):
            cur = [l]
            out.append(cur)
        elif cur is not None:
            cur.append(l)
    return ["\n".join(s) for s in out]


def run_script(step):
    """The `run:` body of a step, de-indented ('' when it has none)."""
    lines = step.splitlines()
    for i, l in enumerate(lines):
        if re.match(r"^\s+run: \|\s*$", l):
            body = []
            indent = None
            for b in lines[i + 1:]:
                if b.strip():
                    width = len(b) - len(b.lstrip())
                    indent = width if indent is None else indent
                    if width < indent:
                        break
                body.append(b)
            return textwrap.dedent("\n".join(body)) + "\n"
    m = re.search(r"^\s+run: (.+)$", step, re.M)
    return m.group(1) if m else ""


def step_with(needle):
    return next(s for s in steps(workflow_text()) if needle in s)


def condition(step):
    m = re.search(r"^\s+if: (.+)$", step, re.M)
    return m.group(1) if m else ""


def job_env():
    """The job-level `env:` of the workflow as a dict of its literal values (expressions kept as written)."""
    lines = workflow_text().splitlines()
    start = lines.index("    env:")
    env = {}
    for l in lines[start + 1:]:
        if not l.startswith("      ") or l.startswith("    steps:"):
            break
        m = re.match(r"^      ([A-Z_]+): (.*)$", l)
        if m:
            env[m.group(1)] = m.group(2)
    return env


class Workflow(unittest.TestCase):
    def test_it_runs_hourly_at_an_odd_minute_and_on_demand_and_on_nothing_else(self):
        on = "\n".join(top_level_block(workflow_text(), "on"))
        crons = re.findall(r'cron:\s*"([^"]+)"', on)
        self.assertEqual(len(crons), 1, crons)
        minute, rest = crons[0].split(" ", 1)
        self.assertEqual(rest, "* * * *", "hourly")
        self.assertTrue(minute.isdigit() and int(minute) % 2 == 1, f"minute {minute}: an odd minute, off the hour, where everyone's schedules pile up")
        self.assertIn("workflow_dispatch:", on)
        triggers = re.findall(r"^  ([a-z_]+):", on, re.M)
        self.assertEqual(sorted(triggers), ["schedule", "workflow_dispatch"], "no push, no pull_request, no pull_request_target")

    def test_least_privilege(self):
        permissions = [l.strip() for l in top_level_block(workflow_text(), "permissions")]
        self.assertEqual(sorted(permissions), ["contents: read", "issues: write"])
        self.assertNotRegex(workflow_text(), r"(?m)^[ \t]+permissions:", "no job widens it")

    def test_every_action_is_pinned_by_version(self):
        uses = re.findall(r"^\s+- uses: (\S+)|^\s+uses: (\S+)", workflow_text(), re.M)
        found = [a or b for a, b in uses]
        self.assertTrue(found)
        for use in found:
            self.assertRegex(use, r"^[\w.-]+/[\w.-]+@(v\d+(\.\d+){0,2}|[0-9a-f]{40})$", f"{use} is not pinned by version")

    def test_runs_do_not_stack(self):
        block = "\n".join(top_level_block(workflow_text(), "concurrency"))
        self.assertIn("group: reachability", block)
        self.assertIn("cancel-in-progress: false", block)

    def test_only_the_project_repository_probes_and_opens_issues(self):
        self.assertIn("if: github.repository == 'kevstermcgee/RedEngine'", workflow_text())

    def test_the_issue_steps_run_only_for_the_watch_itself(self):
        issue_steps = [s for s in steps(workflow_text()) if "gh issue" in run_script(s) or "gh label" in run_script(s)]
        self.assertEqual(len(issue_steps), 2, "open-or-comment, and close")
        for s in issue_steps:
            cond = condition(s)
            self.assertTrue(cond, "an issue step without a condition:\n" + s)
            self.assertIn("github.event_name == 'schedule'", cond)
            self.assertIn("github.event_name == 'workflow_dispatch'", cond)
            self.assertIn("steps.probe.outputs.watch == 'true'", cond, "a run against another target never touches the issue")
            self.assertNotIn("pull_request", cond)
        self.assertIn("steps.probe.outputs.status == 'unreachable'", condition(step_with("gh issue create")))
        self.assertIn("steps.probe.outputs.status == 'ok'", condition(step_with("gh issue close")), "only a clean first-attempt pass closes it")

    def test_the_token_reaches_only_the_steps_that_use_it(self):
        text = workflow_text()
        self.assertEqual(text.count("GH_TOKEN:"), 2)
        for s in steps(text):
            if "GH_TOKEN" in s:
                self.assertIn("gh issue", run_script(s))
        self.assertNotIn("GH_TOKEN", "\n".join(job_env()), "not in the job-wide environment")

    def test_no_expression_is_pasted_into_a_script(self):
        # inputs and outputs reach a script through env: only (a pasted `${{ inputs.host }}` is a shell injection for anyone who can dispatch it)
        for s in steps(workflow_text()):
            self.assertNotIn("${{", run_script(s), "an expression inside a run script:\n" + s)

    def test_a_failure_is_confirmed_a_few_minutes_later(self):
        seconds = int(job_env()["CONFIRM_AFTER_SECONDS"])
        self.assertGreaterEqual(seconds, 120, "a few minutes: longer than a server restart or a router re-adding a mapping")
        self.assertLessEqual(seconds, 300, "and the two probes plus the wait fit the job's timeout")
        self.assertIn('sleep "$CONFIRM_AFTER_SECONDS"', run_script(step_with("id: probe")))

    def test_the_run_goes_red_for_an_outage_or_a_probe_without_a_verdict(self):
        last = steps(workflow_text())[-1]
        self.assertIn("steps.probe.outputs.status == 'unreachable'", last)
        self.assertIn("steps.probe.outputs.status == 'error'", last)
        self.assertIn("exit 1", run_script(last))

    def test_it_runs_the_probe_with_the_scripts_own_defaults_and_the_standard_title(self):
        self.assertNotIn("duckdns", workflow_text(), "the host name is net_probe.py's constant, not repeated here")
        self.assertEqual(job_env()["ISSUE_TITLE"], ISSUE_TITLE)
        self.assertEqual(job_env()["ISSUE_LABEL"], "reachability")
        self.assertEqual(workflow_text().count("gh issue create"), 1)
        self.assertNotIn("upload-artifact", workflow_text(), "no artifacts: nothing outside the masked log and summary")

    def test_structure_when_pyyaml_is_available(self):
        try:
            import yaml
        except ImportError:
            self.skipTest("PyYAML is not installed")
        doc = yaml.safe_load(workflow_text())
        on = doc.get("on", doc.get(True))  # YAML 1.1 reads a bare `on` as true
        self.assertEqual(sorted(on), ["schedule", "workflow_dispatch"])
        self.assertEqual(doc["permissions"], {"contents": "read", "issues": "write"})
        job = doc["jobs"]["probe"]
        self.assertEqual(list(doc["jobs"]), ["probe"])
        self.assertLessEqual(job["timeout-minutes"], 15)
        self.assertEqual(job["runs-on"], "ubuntu-24.04")
        self.assertEqual(job["env"]["ISSUE_TITLE"], ISSUE_TITLE)
        inputs = on["workflow_dispatch"]["inputs"]
        self.assertEqual(sorted(inputs), ["game_port", "host", "relay_port", "silent_port"])
        self.assertTrue(all(v["default"] == "" for v in inputs.values()), "empty means the script's defaults: the host and ports are written once")
        for step in job["steps"]:
            if "run" in step:
                self.assertNotIn("${{", step["run"])


# ---------------------------------------------------------------------------------------------------------------------------------------
# the workflow's step scripts, run as GitHub runs them (bash -eo pipefail) with a fake probe and a fake `gh`


FAKE_GH = r'''
import json, os, sys
path = os.environ["FAKE_GH_STATE"]
with open(path, encoding="utf-8") as f:
    state = json.load(f)
args = sys.argv[1:]
state["calls"].append(args)
flag = lambda name: args[args.index(name) + 1] if name in args else None
out = ""
if args[:2] == ["label", "create"]:
    state["labels"].append(args[2])
elif args[:2] == ["issue", "list"]:
    assert flag("--label") == "reachability" and flag("--state") == "open", args
    numbers = state["open"][:1] if ".[0]" in flag("--jq") else state["open"]
    out = "\n".join(str(n) for n in numbers)
elif args[:2] == ["issue", "view"]:
    out = state["last_activity"]
elif args[:2] == ["issue", "create"]:
    with open(flag("--body-file"), encoding="utf-8") as f:
        state["created"].append({"title": flag("--title"), "label": flag("--label"), "body": f.read()})
    out = "https://github.com/o/r/issues/99"
elif args[:2] == ["issue", "comment"]:
    if "--body-file" in args:
        with open(flag("--body-file"), encoding="utf-8") as f:
            body = f.read()
    else:
        body = flag("--body")
    state["comments"].append({"issue": args[2], "body": body})
elif args[:2] == ["issue", "close"]:
    state["closed"].append(args[2])
else:
    sys.exit(f"fake gh: unexpected call {args}")
with open(path, "w", encoding="utf-8") as f:
    json.dump(state, f)
if out:
    print(out)
'''

# The probe as the Probe step calls it: records its arguments, prints a result, and exits with the next code from codes.txt.
FAKE_PROBE = r'''#!/usr/bin/env bash
echo "$*" >> "$RUNNER_TEMP/calls.txt"
n=$(wc -l < "$RUNNER_TEMP/calls.txt")
code=$(sed -n "${n}p" "$RUNNER_TEMP/codes.txt")
echo '{"probe": "fake"}'
exit "${code:-0}"
'''


@unittest.skipUnless(shutil.which("bash") and sys.platform.startswith("linux"), "the steps run on ubuntu: needs bash and GNU date")
class WorkflowSteps(ProbeCase):
    def setUp(self):
        super().setUp()
        self.tmp = tempfile.mkdtemp(prefix="re2_reachability_")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.bin = os.path.join(self.tmp, "bin")
        self.venv = os.path.join(self.tmp, "venv", "bin")
        os.makedirs(self.bin)
        os.makedirs(self.venv)
        self.state_path = os.path.join(self.tmp, "gh.json")
        self.write_exe(os.path.join(self.bin, "gh"), f"#!{sys.executable}\n" + FAKE_GH)

    def write_exe(self, path, text):
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        os.chmod(path, 0o755)

    def run_step(self, needle, **env):
        full = dict(os.environ, RUNNER_TEMP=self.tmp, GITHUB_OUTPUT=os.path.join(self.tmp, "output.txt"), GITHUB_STEP_SUMMARY=os.path.join(self.tmp, "summary.md"),
                    PATH=self.bin + os.pathsep + os.environ.get("PATH", ""), FAKE_GH_STATE=self.state_path, CONFIRM_AFTER_SECONDS="0",
                    ISSUE_TITLE=job_env()["ISSUE_TITLE"], ISSUE_LABEL=job_env()["ISSUE_LABEL"], RUN_URL="https://github.com/o/r/actions/runs/7",
                    GH_REPO="o/r", PROBE_HOST="", PROBE_RELAY_PORT="", PROBE_GAME_PORT="", PROBE_SILENT_PORT="")
        full.update(env)
        p = subprocess.run(["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", run_script(step_with(needle))], cwd=ROOT, env=full,
                           capture_output=True, text=True, timeout=120)
        outputs = {}
        if os.path.exists(full["GITHUB_OUTPUT"]):
            with open(full["GITHUB_OUTPUT"], encoding="utf-8") as f:
                outputs = dict(l.strip().split("=", 1) for l in f if "=" in l)
            os.remove(full["GITHUB_OUTPUT"])
        return p, outputs

    # -- the Probe step: exit codes to a status, the second probe, the watch flag

    def probe(self, codes, **env):
        self.write_exe(os.path.join(self.venv, "python"), FAKE_PROBE)
        with open(os.path.join(self.tmp, "codes.txt"), "w", encoding="utf-8") as f:
            f.write("".join(f"{c}\n" for c in codes))
        calls = os.path.join(self.tmp, "calls.txt")
        if os.path.exists(calls):
            os.remove(calls)
        p, outputs = self.run_step("id: probe", **env)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        with open(calls, encoding="utf-8") as f:
            return outputs, f.read().splitlines(), p.stdout

    def test_a_first_pass_is_ok_after_one_probe(self):
        outputs, calls, _ = self.probe([0])
        self.assertEqual(outputs, {"status": "ok", "watch": "true"})
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0], "scripts/net_probe.py --egress", "the script's own defaults: the owner's server")

    def test_a_failure_is_probed_again_and_only_a_second_failure_is_unreachable(self):
        outputs, calls, stdout = self.probe([1, 0])
        self.assertEqual((outputs["status"], len(calls)), ("recovered", 2), "failed once, passed once: neither opens nor closes an issue")
        self.assertIn("probing again in 0 s", stdout)
        outputs, calls, _ = self.probe([1, 1])
        self.assertEqual((outputs["status"], len(calls)), ("unreachable", 2))

    def test_any_exit_but_1_on_the_second_probe_is_an_error_never_an_outage(self):
        for codes in ([4, 4], [1, 4], [70, 70], [1, 70], [3, 3], [2, 2]):
            with self.subTest(codes=codes):
                outputs, _, _ = self.probe(codes)
                self.assertEqual(outputs["status"], "error")

    def test_a_run_against_another_target_is_not_the_watch(self):
        for name, value, flag in (("PROBE_HOST", "example.org", "--host example.org"), ("PROBE_RELAY_PORT", "29100", "--relay-port 29100"),
                                  ("PROBE_GAME_PORT", "29101", "--game-port 29101"), ("PROBE_SILENT_PORT", "29102", "--silent-port 29102")):
            with self.subTest(name=name):
                outputs, calls, _ = self.probe([0], **{name: value})
                self.assertEqual(outputs["watch"], "false")
                self.assertIn(flag, calls[0])

    # -- the issue steps

    def gh(self, open_issues=(), last_activity="2026-10-20T00:00:00Z"):
        with open(self.state_path, "w", encoding="utf-8") as f:
            json.dump({"open": list(open_issues), "last_activity": last_activity, "calls": [], "labels": [], "created": [], "comments": [], "closed": []}, f)
        self.write_exe(os.path.join(self.venv, "python"), f'#!/usr/bin/env bash\nexec "{sys.executable}" "$@"\n')
        # what the probe left: a real failed result from the fakes
        argv, _ = self.world(relay=None)
        argv.remove("--no-mask")
        rc, out, _ = self.run_main(argv)
        self.assertEqual(rc, 1)
        with open(os.path.join(self.tmp, "probe.json"), "w", encoding="utf-8") as f:
            f.write(out)

    def gh_state(self):
        with open(self.state_path, encoding="utf-8") as f:
            return json.load(f)

    def ago(self, seconds):
        return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - seconds))

    def test_the_first_confirmed_failure_opens_one_issue_that_mentions_the_maintainers(self):
        self.gh()
        p, _ = self.run_step("gh issue create", EVENT="schedule")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        state = self.gh_state()
        self.assertEqual(len(state["created"]), 1)
        issue = state["created"][0]
        self.assertEqual((issue["title"], issue["label"]), (ISSUE_TITLE, "reachability"))
        self.assertEqual(state["labels"], ["reachability"], "the label is made (or kept) before the issue uses it")
        maintainers = json.loads(read(CONTRIBUTORS))["maintainers"]
        self.assertIn("cc " + " ".join("@" + m for m in maintainers), issue["body"])
        self.assertIn("Run: https://github.com/o/r/actions/runs/7", issue["body"])
        self.assertIn("FAILED: the game server answers but the relay does not", issue["body"])
        self.assertNotIn("127.0.0", issue["body"], "the address is hidden in the issue")
        self.assertEqual(state["comments"], [])

    def test_an_open_issue_gets_a_comment_at_most_every_six_hours_from_the_schedule(self):
        self.gh(open_issues=[12], last_activity=self.ago(3600))
        p, _ = self.run_step("gh issue create", EVENT="schedule")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        state = self.gh_state()
        self.assertEqual((state["created"], state["comments"], state["labels"]), ([], [], []), "an hour after the last word: nothing, and never a second issue")
        self.gh(open_issues=[12], last_activity=self.ago(7 * 3600))
        p, _ = self.run_step("gh issue create", EVENT="schedule")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        state = self.gh_state()
        self.assertEqual(state["created"], [])
        self.assertEqual([c["issue"] for c in state["comments"]], ["12"])
        self.assertTrue(state["comments"][0]["body"].startswith("Still unreachable at "))

    def test_a_manual_run_always_comments_on_the_open_issue(self):
        self.gh(open_issues=[12], last_activity=self.ago(60))
        p, _ = self.run_step("gh issue create", EVENT="workflow_dispatch")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual([c["issue"] for c in self.gh_state()["comments"]], ["12"])

    def test_a_clean_pass_says_reachable_again_and_closes_every_open_issue_and_touches_nothing_when_none_is_open(self):
        self.gh(open_issues=[12, 13])
        p, _ = self.run_step("gh issue close")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        state = self.gh_state()
        self.assertEqual(state["closed"], ["12", "13"])
        self.assertEqual([c["issue"] for c in state["comments"]], ["12", "13"])
        self.assertTrue(all(c["body"].startswith("Reachable again") and "runs/7" in c["body"] for c in state["comments"]))
        self.gh(open_issues=[])
        p, _ = self.run_step("gh issue close")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        state = self.gh_state()
        self.assertEqual([c[:2] for c in state["calls"]], [["issue", "list"]], "while things are up: one read, no write")


if __name__ == "__main__":
    unittest.main(verbosity=1)
