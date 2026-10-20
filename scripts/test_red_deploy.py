#!/usr/bin/env python3
"""The home-hosting deploy scripts: `deploy/red-portmap.py` (permanent UPnP mappings) and `deploy/red-selfcheck.py` (the five-minute check of relay, server, mappings
and protocol drift). Run by tests/red_deploy.rs, or directly: python3 scripts/test_red_deploy.py. Python standard library only; no router, no systemd, no network.

* The router is a small HTTP server that speaks just enough UPnP SOAP and behaves like the one at home (TP-Link Archer A7): an AddPortMapping identical to the
  mapping it already holds is accepted and CHANGES NOTHING, so a timed lease is never extended. That is what made the old hourly lease lapse for ~40 s.
* The relay and the game server are UDP servers on loopback speaking the relay's wire format (crates/red_core/src/net/relay.rs) and answering a QUIC Version
  Negotiation probe. The machine (systemd, the journal, git, gh, the disk) is a `Host` subclass whose answers the test sets.
"""
import contextlib
import importlib.util
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
import xml.etree.ElementTree as ET
from http.server import BaseHTTPRequestHandler, HTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOY = os.path.join(os.path.dirname(HERE), 'deploy')


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, os.path.join(DEPLOY, path))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


portmap = load('red_portmap', 'red-portmap.py')
selfcheck = load('red_selfcheck', 'red-selfcheck.py')
selfcheck.UDP_TIMEOUT = 0.25  # a silent probe fails in a second, not in six
LAN = '192.168.0.133'
SERVER_REV = '4a9b3972379e26d00baf6f083ca5a058fdbb2f04'
RELAY_REV = SERVER_REV
RELEASE_REV = 'c78f6b77a68a9b2e02025747491343c61453eea0'


# ----------------------------------------------------------------------------------------------------------------------------------------
# a router

class FakeRouter:
    """UPnP IGD over HTTP: GetExternalIPAddress, Get/Add/DeletePortMapping. `calls` records (action, port) for every mutating call."""

    def __init__(self, external='203.0.113.9'):
        self.mappings = {}  # port -> {'client', 'internal', 'desc', 'lease'}
        self.calls = []
        self.external = external
        router = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                body = self.rfile.read(int(self.headers['Content-Length']))
                action = self.headers['SOAPAction'].strip('"').split('#')[1]
                root = ET.fromstring(body)
                args = {e.tag.rsplit('}', 1)[-1]: (e.text or '') for e in root.iter() if len(e) == 0}
                status, fields = router.handle(action, args)
                inner = ''.join('<%s>%s</%s>' % (k, v, k) for k, v in fields.items())
                xml = ('<?xml version="1.0"?><s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body>%s</s:Body></s:Envelope>'
                       % (inner if status == 200 else '<s:Fault><detail><UPnPError>%s</UPnPError></detail></s:Fault>' % inner))
                data = xml.encode()
                self.send_response(status)
                self.send_header('Content-Type', 'text/xml')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.httpd = HTTPServer(('127.0.0.1', 0), Handler)
        self.url = 'http://127.0.0.1:%d/ctl/IPConn' % self.httpd.server_address[1]
        threading.Thread(target=self.httpd.serve_forever, kwargs={'poll_interval': 0.02}, daemon=True).start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()

    def handle(self, action, a):
        port = int(a.get('NewExternalPort', 0) or 0)
        if action == 'GetExternalIPAddress':
            return 200, {'NewExternalIPAddress': self.external}
        if action == 'GetSpecificPortMappingEntry':
            m = self.mappings.get(port)
            if not m:
                return 500, {'errorCode': 714, 'errorDescription': 'NoSuchEntryInArray'}
            return 200, {'NewInternalPort': m['internal'], 'NewInternalClient': m['client'], 'NewEnabled': 1, 'NewPortMappingDescription': m['desc'],
                         'NewLeaseDuration': m['lease']}
        if action == 'AddPortMapping':
            self.calls.append(('add', port))
            want = {'client': a['NewInternalClient'], 'internal': int(a['NewInternalPort']), 'desc': a['NewPortMappingDescription'], 'lease': int(a['NewLeaseDuration'])}
            have = self.mappings.get(port)
            if have and all(have[k] == want[k] for k in ('client', 'internal', 'desc')):
                return 200, {}  # accepted, and nothing changes: the lease is NOT extended
            if have:
                return 500, {'errorCode': 718, 'errorDescription': 'ConflictInMappingEntry'}
            self.mappings[port] = want
            return 200, {}
        if action == 'DeletePortMapping':
            self.calls.append(('delete', port))
            if port in self.mappings:
                del self.mappings[port]
                return 200, {}
            return 500, {'errorCode': 714, 'errorDescription': 'NoSuchEntryInArray'}
        return 500, {'errorCode': 401, 'errorDescription': 'Invalid Action'}

    def put(self, port, client=LAN, desc=None, lease=0):
        self.mappings[port] = {'client': client, 'internal': port, 'desc': desc or 'red-engine %d' % port, 'lease': lease}


def run_portmap(router, *args):
    """Runs red-portmap.py's main against the fake router as if this box were `LAN`: (exit code, stdout)."""
    saved = (portmap.gateway, portmap.local_ip, os.environ.get('RED_PORTMAP_URL'))
    portmap.gateway, portmap.local_ip = (lambda: '192.168.0.1'), (lambda gw: LAN)
    os.environ['RED_PORTMAP_URL'] = router.url
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out):
            code = portmap.main(list(args))
    finally:
        portmap.gateway, portmap.local_ip = saved[0], saved[1]
        if saved[2] is None:
            os.environ.pop('RED_PORTMAP_URL', None)
        else:
            os.environ['RED_PORTMAP_URL'] = saved[2]
    return code, out.getvalue()


class PortmapTests(unittest.TestCase):
    def setUp(self):
        self.router = FakeRouter()
        self.addCleanup(self.router.close)

    def enable(self, *ports):
        return run_portmap(self.router, 'enable', *[a for p in (ports or (28016, 27015)) for a in ('--port', str(p))])

    def test_enable_opens_each_port_permanently_for_this_box(self):
        code, out = self.enable()
        self.assertEqual(code, 0, out)
        for port in (28016, 27015):
            m = self.router.mappings[port]
            self.assertEqual((m['client'], m['internal'], m['lease'], m['desc']), (LAN, port, 0, 'red-engine %d' % port))
        self.assertIn('permanent', out)

    def test_a_second_run_changes_nothing_and_does_not_even_ask(self):
        self.enable()
        self.router.calls.clear()
        code, out = self.enable()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.router.calls, [], 'a timer that touches nothing when nothing is wrong')
        self.assertEqual(out.count('already mapped'), 2, out)

    def test_the_router_ignores_an_identical_add_which_is_why_a_timed_lease_never_renewed(self):
        self.router.put(28016, lease=3373)
        portmap.soap(self.router.url, 'AddPortMapping', {
            'NewRemoteHost': '', 'NewExternalPort': 28016, 'NewProtocol': 'UDP', 'NewInternalPort': 28016, 'NewInternalClient': LAN, 'NewEnabled': 1,
            'NewPortMappingDescription': 'red-engine 28016', 'NewLeaseDuration': 3600})
        self.assertEqual(self.router.mappings[28016]['lease'], 3373, 'the premise of the fix: renewing by re-adding the same mapping extends nothing')

    def test_an_old_timed_lease_is_replaced_by_a_permanent_mapping(self):
        self.router.put(28016, lease=3373)
        code, out = self.enable(28016)
        self.assertEqual(code, 0, out)
        self.assertEqual(self.router.calls, [('delete', 28016), ('add', 28016)], 'delete first: an identical-looking add would change nothing')
        self.assertEqual(self.router.mappings[28016]['lease'], 0)

    def test_a_mapping_to_an_old_lan_address_follows_this_box(self):
        self.router.put(27015, client='192.168.0.50')
        code, out = self.enable(27015)
        self.assertEqual(code, 0, out)
        self.assertEqual(self.router.mappings[27015]['client'], LAN)

    def test_a_mapping_the_router_lost_is_restored(self):
        self.enable()
        del self.router.mappings[27015]  # a router reboot
        code, out = self.enable()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.router.mappings[27015]['lease'], 0)

    def test_a_rule_that_is_not_ours_is_left_alone_and_the_run_fails(self):
        self.router.put(27015, client='192.168.0.7', desc='Plex Media Server', lease=0)
        before = dict(self.router.mappings[27015])
        code, out = self.enable()
        self.assertEqual(code, 1, 'the timer shows as failed: somebody must look')
        self.assertIn('held by another rule (Plex Media Server)', out)
        self.assertEqual(self.router.mappings[27015], before)
        self.assertNotIn(('delete', 27015), self.router.calls)
        self.assertEqual(self.router.mappings[28016]['lease'], 0, 'the other port is still opened')

    def test_remove_deletes_only_our_own_mappings(self):
        self.enable(28016)
        self.router.put(27015, desc='Plex Media Server')
        code, out = run_portmap(self.router, 'remove', '--port', '28016', '--port', '27015')
        self.assertEqual(code, 1, out)
        self.assertNotIn(28016, self.router.mappings)
        self.assertIn(27015, self.router.mappings)

    def test_status_prints_what_the_selfcheck_reads(self):
        self.router.put(28016)
        self.router.put(27015, client='192.168.0.9', lease=100)
        code, text = run_portmap(self.router, 'status', '--port', '28016', '--port', '27015', '--port', '27999')
        self.assertEqual(code, 0, text)
        info = selfcheck.parse_portmap(text)
        self.assertEqual((info['router'], info['this'], info['public']), ('192.168.0.1', LAN, '203.0.113.9'))
        self.assertEqual({k: (v['mapped'], v['ours'], v['to'], v['lease']) for k, v in info['ports'].items()},
                         {28016: (True, True, LAN, 0), 27015: (True, True, '192.168.0.9', 100), 27999: (False, False, None, None)})


# ----------------------------------------------------------------------------------------------------------------------------------------
# a relay, a game server, a machine

def reply(sock, data, addr):
    try:
        sock.sendto(data, addr)
    except OSError:
        pass  # the client has already gone (Windows says so here)


class FakeRelay(threading.Thread):
    """The relay's wire format on loopback: Register -> Registered + code, Resolve -> Resolved (10 bytes) or CodeNotFound, Unregister. `mode` 'silent' never answers."""
    ALPHABET = b'ABCDEFGHJKMNPQRSTUVWXYZ23456789'

    def __init__(self, mode='ok'):
        super().__init__(daemon=True)
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(('127.0.0.1', 0))
        self.sock.settimeout(0.05)
        self.port = self.sock.getsockname()[1]
        self.mode, self.by_code, self.tags, self.stop = mode, {}, [], False
        self.start()

    def run(self):
        while not self.stop:
            try:
                data, addr = self.sock.recvfrom(2048)
            except socket.timeout:
                continue
            except ConnectionResetError:
                continue  # Windows reports an ICMP port-unreachable for an earlier reply (its client had gone) on the next receive
            except OSError:
                return
            self.tags.append(data[0])
            if self.mode == 'silent':
                continue
            if data[0] == 1:
                code = next((c for c, a in self.by_code.items() if a == addr), None) or bytes(self.ALPHABET[b % len(self.ALPHABET)] for b in os.urandom(6))
                self.by_code[code] = addr
                reply(self.sock, b'\x02' + code, addr)
            elif data[0] == 3 and len(data) == 7:
                if data[1:] in self.by_code and self.mode != 'forget':
                    reply(self.sock, b'\x04\x00' + os.urandom(8), addr)
                else:
                    reply(self.sock, b'\x05', addr)
            elif data[0] == 7:
                for c in [c for c, a in self.by_code.items() if a == addr]:
                    del self.by_code[c]

    def close(self):
        self.stop = True
        self.join(1)
        self.sock.close()


class FakeQuic(threading.Thread):
    """Answers a QUIC Version Negotiation probe the way a live listener does."""

    def __init__(self):
        super().__init__(daemon=True)
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(('127.0.0.1', 0))
        self.sock.settimeout(0.05)
        self.port = self.sock.getsockname()[1]
        self.stop = False
        self.start()

    def run(self):
        while not self.stop:
            try:
                data, addr = self.sock.recvfrom(2048)
            except socket.timeout:
                continue
            except ConnectionResetError:
                continue
            except OSError:
                return
            if len(data) >= 7 and data[0] & 0x80:
                dl = data[5]
                dcid, sl = data[6:6 + dl], data[6 + dl]
                scid = data[7 + dl:7 + dl + sl]
                reply(self.sock, bytes([0x80 | 0x15]) + b'\0\0\0\0' + bytes([len(scid)]) + scid + bytes([len(dcid)]) + dcid + (1).to_bytes(4, 'big'), addr)

    def close(self):
        self.stop = True
        self.join(1)
        self.sock.close()


def free_udp_port():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(('127.0.0.1', 0))
    port = s.getsockname()[1]
    s.close()
    return port


def portmap_text(public='127.0.0.1', lines=None):
    lines = lines if lines is not None else ['UDP 28016: red-engine 28016 -> %s:28016 (lease 0s)' % LAN, 'UDP 27015: red-engine 27015 -> %s:27015 (lease 0s)' % LAN]
    return 'router 192.168.0.1, this machine %s, public IPv4 %s\n' % (LAN, public) + '\n'.join(lines) + '\n'


class FakeHost(selfcheck.Host):
    """The machine, as the test says it is. It has no way to restart or change anything: the check only looks."""

    def __init__(self):
        super().__init__()
        self.t = 1_800_000_000.0
        self.units = {'red-relay.service': 'active', 'red-game@killchain.service': 'active', 'red-portmap.timer': 'active', 'red-health.timer': 'active'}
        self.journal_lines = []
        self.portmap_out = (0, portmap_text())
        self.disk = 100e9
        self.files = {'/f/server/REVISION': SERVER_REV + '\n', '/f/relay/REVISION': RELAY_REV + '\n'}
        self.protocols = {SERVER_REV: 17, RELAY_REV: 17, RELEASE_REV: 17}
        self.blobs = {}
        self.release = {'tag': 'killchain-v4', 'engine_revision': RELEASE_REV}
        self.startup = ''
        self.gh_calls = 0
        self.reads = []

    def now(self):
        return self.t

    def unit_state(self, unit):
        return self.units.get(unit, 'unknown')

    def journal(self, unit, minutes=10, grep=None):
        return '\n'.join(l for l in self.journal_lines if not grep or grep in l)

    def unit_started(self, unit):
        return 'Tue 2026-10-20 10:34:12 PDT' if self.startup else ''

    def journal_since(self, unit, since, grep):
        return self.startup

    def portmap_status(self, path, ports):
        return self.portmap_out

    def disk_free(self, path='/'):
        return self.disk

    def read_file(self, path):
        self.reads.append(path)
        return self.files.get(path)

    def protocol_at(self, rev):
        return self.protocols.get(rev)

    def blob(self, rev, path):
        return self.blobs.get((rev, path))

    def latest_release(self):
        self.gh_calls += 1
        return self.release

    def stats(self, age=30, joins=0, rejected=(0, 0, 0, 0, 0, 0), counters=True, players=0):
        line = ('%.3f box red_server[1]: [100.00s] stats: players %d | phase waiting | tick avg 4 us worst 181 us | promoted props 0 | in 0.0 KB/s out 0.0 KB/s | snapshots 9 | '
                'bad packets 0 bad tags 0' % (self.t - age, players))
        if counters:
            line += ' | joins %d | rejected v%d m%d k%d b%d f%d o%d | throttled 0' % ((joins,) + tuple(rejected))
        self.journal_lines = [l for l in self.journal_lines if 'stats: players' not in l] + [line]


class SelfcheckTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix='red_selfcheck_')
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.relay, self.quic, self.host = FakeRelay(), FakeQuic(), FakeHost()
        self.addCleanup(self.relay.close)
        self.addCleanup(self.quic.close)
        self.host.stats()
        self.relay_port, self.game_port = self.relay.port, self.quic.port

    def args(self, *extra):
        return ['--relay', '127.0.0.1:%d' % self.relay_port, '--public', 'localhost:%d' % self.relay_port, '--game-port', str(self.game_port),
                '--state-dir', self.tmp, '--server-revision', '/f/server/REVISION', '--relay-revision', '/f/relay/REVISION', '--portmap', '/f/red-portmap.py', *extra]

    def run_check(self, *extra):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = selfcheck.main(self.args(*extra), self.host)
        return code, out.getvalue().strip()

    def report(self):
        with open(os.path.join(self.tmp, 'selfcheck.json'), encoding='utf-8') as f:
            return json.load(f)

    def test_a_healthy_box_passes_every_check_and_writes_the_json_and_one_log_line(self):
        code, out = self.run_check()
        self.assertEqual((code, out), (0, 'red-selfcheck: ok: 8 checks'), out)
        report = self.report()
        self.assertTrue(report['ok'])
        self.assertEqual(list(report['checks']), ['units', 'relay', 'public', 'game', 'mappings', 'server', 'drift', 'disk'])
        self.assertTrue(all(c['status'] == 'ok' for c in report['checks'].values()), report['checks'])
        self.run_check()
        with open(os.path.join(self.tmp, 'selfcheck.log'), encoding='utf-8') as f:
            lines = f.read().splitlines()
        self.assertEqual(len(lines), 2, 'one line per run')
        self.assertIn(' ok units=ok relay=ok public=ok game=ok mappings=ok server=ok drift=ok disk=ok ', lines[0])
        self.assertEqual(self.relay.by_code, {}, 'the throwaway registration was removed again')
        self.assertIn(7, self.relay.tags, 'with an Unregister')

    def test_a_relay_that_is_not_listening_fails_with_one_line_naming_it(self):
        dead = free_udp_port()
        code, out = self.run_check('--relay', '127.0.0.1:%d' % dead)
        self.assertEqual(code, 1, out)
        self.assertTrue(out.startswith('red-selfcheck: FAIL relay: relay did not answer Register'), out)
        self.assertEqual(len(out.splitlines()), 1, 'one line')
        self.assertTrue(self.report()['failed'] == ['relay'] and not self.report()['ok'], self.report()['failed'])

    def test_a_silent_relay_is_a_failure_on_both_paths(self):
        self.relay.mode = 'silent'
        code, out = self.run_check()
        self.assertEqual(code, 1, out)
        report = self.report()
        self.assertEqual(report['failed'], ['relay', 'public'], report['checks'])
        self.assertIn('FAIL relay: ', out)
        self.assertIn('(+1 more: public)', out)

    def test_the_public_name_reaching_a_different_relay_is_caught(self):
        self.relay.mode = 'forget'  # answers Register, then says CodeNotFound for the code it just gave
        code, out = self.run_check()
        self.assertEqual(code, 1, out)
        self.assertEqual(self.report()['failed'], ['relay', 'public'])

    def test_a_stale_duckdns_record_is_named(self):
        self.host.portmap_out = (0, portmap_text(public='203.0.113.9'))
        code, out = self.run_check()
        self.assertEqual(code, 1, out)
        self.assertIn('the DuckDNS update is stale', self.report()['checks']['public']['detail'])

    def test_a_game_server_with_no_quic_listener_fails(self):
        self.quic.close()
        code, out = self.run_check()
        self.assertEqual(code, 1, out)
        self.assertEqual(self.report()['failed'], ['game'])

    def test_a_stopped_unit_is_named_and_nothing_is_restarted(self):
        self.host.units['red-game@killchain.service'] = 'inactive'
        code, out = self.run_check()
        self.assertEqual(code, 1, out)
        self.assertIn('red-game@killchain.service is inactive', out)
        self.assertFalse([m for m in dir(self.host) if 'restart' in m or 'start_unit' in m], 'the check has no way to change anything')

    def test_the_router_mappings_are_checked_for_presence_owner_address_and_lease(self):
        cases = {
            'a missing mapping': (['UDP 28016: not mapped', 'UDP 27015: red-engine 27015 -> %s:27015 (lease 0s)' % LAN], 'fail', 'UDP 28016 is not mapped'),
            'a rule that is not ours': (['UDP 28016: Plex -> 192.168.0.7:28016 (lease 0s)  [NOT OURS]', 'UDP 27015: red-engine 27015 -> %s:27015 (lease 0s)' % LAN], 'fail', 'held by another rule'),
            'another machine': (['UDP 28016: red-engine 28016 -> 192.168.0.50:28016 (lease 0s)', 'UDP 27015: red-engine 27015 -> %s:27015 (lease 0s)' % LAN], 'fail', 'points at 192.168.0.50'),
            'a router error': (['UDP 28016: UPnP 501: Action Failed', 'UDP 27015: red-engine 27015 -> %s:27015 (lease 0s)' % LAN], 'fail', 'the router said: UPnP 501'),
            'a timed lease': (['UDP 28016: red-engine 28016 -> %s:28016 (lease 3373s)' % LAN, 'UDP 27015: red-engine 27015 -> %s:27015 (lease 0s)' % LAN], 'warn', 'timed lease'),
        }
        for name, (lines, status, text) in cases.items():
            with self.subTest(name):
                self.host.portmap_out = (0, portmap_text(lines=lines))
                code, out = self.run_check('--skip', 'relay,public,game')
                result = self.report()['checks']['mappings']
                self.assertEqual(result['status'], status, result)
                self.assertIn(text, result['detail'])
                self.assertEqual(code, 1 if status == 'fail' else 0, out)

    def test_a_router_that_does_not_answer_fails_the_mappings_check(self):
        self.host.portmap_out = (1, 'Traceback (most recent call last):\nurllib.error.URLError: <urlopen error timed out>\n')
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 1, out)
        self.assertIn('the router did not answer UPnP', self.report()['checks']['mappings']['detail'])

    def test_a_missing_or_old_stats_line_fails_the_server_check(self):
        self.host.journal_lines = []
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 1, out)
        self.assertIn('no stats line', self.report()['checks']['server']['detail'])
        self.host.stats(age=151)
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 1, out)
        self.assertIn('151 s old (limit 150 s)', self.report()['checks']['server']['detail'])
        self.host.stats(age=149)
        self.assertEqual(self.run_check('--skip', 'relay,public,game')[0], 0)

    def test_refusals_growing_without_a_join_fail_once_and_then_recover(self):
        skip = ('--skip', 'relay,public,game')
        self.host.stats(joins=3, rejected=(0, 0, 0, 0, 0, 0))
        self.assertEqual(self.run_check(*skip)[0], 0)
        self.host.t += 300
        self.host.journal_lines.append('%.3f box red_server[1]: [400.00s] join refused (version): the client speaks protocol v16 and this server v17, so the client is the old side and must update; '
                                       'peer 73.157.x.x' % (self.host.t - 100))
        self.host.stats(joins=3, rejected=(2, 0, 1, 0, 0, 0))
        code, out = self.run_check(*skip)
        self.assertEqual(code, 1, out)
        detail = self.report()['checks']['server']['detail']
        self.assertIn('version +2, needs-key +1', detail)
        self.assertIn('nobody joined', detail)
        self.assertIn('the client speaks protocol v16 and this server v17', detail, 'the last refusal line is quoted')
        self.assertNotIn('box red_server', detail, 'without the journal prefix')
        self.host.t += 300
        self.host.stats(joins=3, rejected=(2, 0, 1, 0, 0, 0))
        code, out = self.run_check(*skip)
        self.assertEqual(code, 0, 'no new refusals since the last check: it recovers on its own')
        self.assertIn('FAIL server', self.report()['last_failure']['summary'], 'and the last failure is remembered for box-status')

    def test_refusals_beside_a_join_a_restart_and_an_older_build_are_not_failures(self):
        skip = ('--skip', 'relay,public,game')
        self.host.stats(joins=3, rejected=(2, 0, 0, 0, 0, 0))
        self.run_check(*skip)
        self.host.t += 300
        self.host.stats(joins=4, rejected=(5, 1, 0, 0, 0, 0))
        self.assertEqual(self.run_check(*skip)[0], 0, 'somebody joined in the same interval')
        self.host.t += 300
        self.host.stats(joins=0, rejected=(0, 0, 0, 0, 0, 0))
        self.assertEqual(self.run_check(*skip)[0], 0, 'the counters started over: the server restarted')
        self.host.t += 300
        self.host.stats(counters=False)
        code, out = self.run_check(*skip)
        self.assertEqual(code, 0, out)
        self.assertIn('predates the reject counters', self.report()['checks']['server']['detail'])

    def test_protocol_drift_between_server_and_the_latest_release_fails_and_names_both(self):
        self.host.protocols[RELEASE_REV] = 18
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 1, out)
        detail = self.report()['checks']['drift']['detail']
        for part in ('protocol drift', '4a9b397237', 'speaks v17', 'killchain-v4', 'c78f6b77a6', 'speaks v18', 'cannot join'):
            self.assertIn(part, detail)

    def test_missing_revision_files_fail(self):
        del self.host.files['/f/relay/REVISION']
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 1, out)
        self.assertIn('/f/relay/REVISION', self.report()['checks']['drift']['detail'])
        self.host.files['/f/relay/REVISION'] = 'not a commit\n'
        self.assertEqual(self.run_check('--skip', 'relay,public,game')[0], 1)

    def test_gh_being_unavailable_is_a_warning_and_a_cached_release_is_used(self):
        skip = ('--skip', 'relay,public,game')
        self.assertEqual(self.run_check(*skip)[0], 0)
        self.assertEqual(self.host.gh_calls, 1)
        self.assertEqual(self.run_check(*skip)[0], 0)
        self.assertEqual(self.host.gh_calls, 1, 'a fresh cached answer: GitHub is not asked every five minutes')
        self.host.t += 7200
        self.host.stats()
        self.host.release = None  # gh down
        code, out = self.run_check(*skip)
        self.assertEqual(code, 0, out)
        self.assertEqual(self.report()['checks']['drift']['status'], 'ok', 'the cached release still answers the comparison')
        shutil.rmtree(self.tmp)
        os.makedirs(self.tmp)
        self.host.stats()
        code, out = self.run_check(*skip)
        self.assertEqual(code, 0, 'no cache and no gh: tolerated')
        self.assertEqual(self.report()['checks']['drift']['status'], 'warn')
        self.assertIn('warning', out)

    def test_a_revision_file_that_disagrees_with_the_running_binary_fails(self):
        self.host.startup = 'protocol: wire v18 (a client must speak the same version to join)\n'
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 1, out)
        self.assertIn('REVISION is stale', self.report()['checks']['drift']['detail'])

    def test_a_relay_built_from_different_wire_code_than_the_release_is_a_warning(self):
        for f in selfcheck.RELAY_FILES:
            self.host.blobs[(RELAY_REV, f)] = 'aaa'
            self.host.blobs[(RELEASE_REV, f)] = 'bbb' if f.endswith('/relay.rs') else 'aaa'
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 0, out)
        result = self.report()['checks']['drift']
        self.assertEqual(result['status'], 'warn')
        self.assertIn('relay.rs', result['detail'])

    def test_low_disk_fails(self):
        self.host.disk = 14.9e9
        code, out = self.run_check('--skip', 'relay,public,game')
        self.assertEqual(code, 1, out)
        self.assertIn('only 14.9 GB free', self.report()['checks']['disk']['detail'])
        self.host.disk = 15.1e9
        self.assertEqual(self.run_check('--skip', 'relay,public,game')[0], 0)

    def test_no_secret_is_read_or_printed(self):
        canary = 'SECRET-JOIN-KEY-CANARY'
        self.host.journal_lines.append('%.3f box red_server[1]: join key (generated): %s' % (self.host.t - 5, canary))
        self.host.files['/home/x/.config/killchain/server.env'] = 'RED_KEY=%s\n' % canary
        code, out = self.run_check('--json', '-v')
        self.assertEqual(code, 0, out)
        everything = out
        for name in ('selfcheck.json', 'selfcheck.log'):
            with open(os.path.join(self.tmp, name), encoding='utf-8') as f:
                everything += f.read()
        self.assertNotIn(canary, everything)
        self.assertEqual(sorted(self.host.reads), ['/f/relay/REVISION', '/f/server/REVISION'], 'the only files it opens are the two REVISION files')
        with open(os.path.join(DEPLOY, 'red-selfcheck.py'), encoding='utf-8') as f:
            source = f.read()
        for name in ('server.env', 'join-code', 'identity'):
            self.assertNotIn("'%s" % name, source.split('"""', 2)[2], 'the script never names a secret file')

    def test_the_log_keeps_the_last_two_thousand_lines(self):
        path = os.path.join(self.tmp, 'selfcheck.log')
        with open(path, 'w', encoding='utf-8') as f:
            f.write('old\n' * 2600)
        self.run_check('--skip', 'relay,public,game')
        with open(path, encoding='utf-8') as f:
            lines = f.read().splitlines()
        self.assertEqual(len(lines), selfcheck.LOG_KEEP_LINES)
        self.assertIn(' ok ', lines[-1])

    def test_a_bug_in_one_check_is_that_checks_failure_and_the_others_still_report(self):
        def boom(rev):
            raise RuntimeError('boom')
        self.host.protocol_at = boom
        code, out = self.run_check()
        self.assertEqual(code, 1, out)
        report = self.report()
        self.assertEqual(report['failed'], ['drift'], report['checks'])
        self.assertIn('the drift check itself crashed: RuntimeError: boom', report['checks']['drift']['detail'])
        self.assertTrue(all(report['checks'][n]['status'] == 'ok' for n in ('units', 'relay', 'public', 'game', 'mappings', 'server', 'disk')))

    def test_a_bad_skip_name_is_a_usage_error(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as e:
            selfcheck.main(self.args('--skip', 'nonsense'), self.host)
        self.assertEqual(e.exception.code, 2)

    def test_the_real_portmap_script_and_the_check_agree_on_the_status_text(self):
        router = FakeRouter(external='127.0.0.1')
        self.addCleanup(router.close)
        router.put(28016)
        router.put(27015)
        code, text = run_portmap(router, 'status', '--port', '28016', '--port', '27015')
        self.assertEqual(code, 0, text)
        self.host.portmap_out = (0, text)
        code, out = self.run_check()
        self.assertEqual((code, self.report()['checks']['mappings']['status']), (0, 'ok'), out)
        router.put(27015, client='192.168.0.50')
        code, text = run_portmap(router, 'status', '--port', '28016', '--port', '27015')
        self.host.portmap_out = (0, text)
        self.assertEqual(self.run_check()[0], 1)


@unittest.skipUnless(shutil.which('git'), 'needs git')
class ProtocolFromGitTests(unittest.TestCase):
    def test_the_protocol_at_a_revision_is_read_from_a_local_clone_without_fetching(self):
        repo = tempfile.mkdtemp(prefix='red_selfcheck_git_')
        self.addCleanup(shutil.rmtree, repo, True)

        def git(*a):
            p = subprocess.run(['git', '-C', repo, '-c', 'user.name=t', '-c', 'user.email=t@example.invalid', '-c', 'commit.gpgsign=false', '-c', 'core.fsync=none', *a], capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stderr)
            return p.stdout.strip()

        git('init', '-q')
        revs = {}
        for version in (16, 17):
            path = os.path.join(repo, 'crates', 'red_core', 'src', 'net')
            os.makedirs(path, exist_ok=True)
            with open(os.path.join(path, 'protocol.rs'), 'w', encoding='utf-8') as f:
                f.write('//! x\npub const PROTOCOL_VERSION: u16 = %d;\n' % version)
            with open(os.path.join(path, 'relay.rs'), 'w', encoding='utf-8') as f:
                f.write('// relay %d\n' % version)
            git('add', '-A')
            git('commit', '-q', '-m', 'v%d' % version)
            revs[version] = git('rev-parse', 'HEAD')
        host = selfcheck.Host([os.path.join(repo, 'no-such-clone'), repo])
        self.assertEqual((host.protocol_at(revs[16]), host.protocol_at(revs[17])), (16, 17))
        self.assertNotEqual(host.blob(revs[16], 'crates/red_core/src/net/relay.rs'), host.blob(revs[17], 'crates/red_core/src/net/relay.rs'))
        self.assertIsNone(host.blob('0' * 40, 'crates/red_core/src/net/relay.rs'))


if __name__ == '__main__':
    unittest.main(verbosity=1)
