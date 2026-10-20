#!/usr/bin/env python3
"""red-selfcheck: is this box's game hosting actually working, and will a friend be able to join? Run every five minutes by `red-selfcheck.timer`.

  red-selfcheck [--json] [-v] [--skip units,relay,...] [--relay HOST:PORT] [--public HOST:PORT] [--game NAME] [--state-dir DIR] ...

Exit 0 when nothing failed (a warning, such as "GitHub is unreachable so the release was not compared", does not fail), 1 when something did, 2 on bad
arguments. One line goes to stdout (the verdict, or the first reason and how many more), one line per run to `<state-dir>/selfcheck.log`, and the whole
report as JSON to `<state-dir>/selfcheck.json` (`~/.local/state/box/`, which `box-status` reads). It NEVER restarts or changes anything (red-health restarts a
silent server; this only looks) and never reads or prints a join key: it does not open server.env, join-code.txt or the identity files.

Why it exists (2026-10-20): two people could not play Killchain together for weeks and nothing said why. The network was fine; a chain of small things was
not: a relay and a server that had been stopped for days with nobody watching, a server on protocol 15 while every client spoke 16 or 17, a router mapping
that lapsed 40 s an hour. Each check below is one of those things.

  units      red-relay, red-game@killchain, red-portmap.timer and red-health.timer are active.
  relay      Register + Resolve round trip against the relay on 127.0.0.1:28016 (a throwaway registration, removed again). Wire format:
             crates/red_core/src/net/relay.rs (tag 1 Register, 2 Registered + 6-byte code, 3 Resolve + code, 4 Resolved, 5 CodeNotFound, 7 Unregister).
  public     the public name (red-engine.duckdns.org) resolves to the router's current address, and a Resolve through it reaches the relay. The router hairpins,
             so this box can test the path a friend takes from outside, apart from their own ISP; `scripts/net_probe.py` from a GitHub runner is the outside-in test.
  game       a QUIC Version Negotiation probe (stateless: no connection, no join) answers on 127.0.0.1:27015 and through the public name.
  mappings   `red-portmap.py status`: UDP 28016 and 27015 are mapped on the router, by us, to this box's LAN address.
  server     the server's `stats:` journal line is fresh (< 150 s) and no `rejected` counter grew since the last run without a join (the counters of ADR
             2026-10-20-a-server-says-why-it-refused-a-join; an older build without them is tolerated).
  drift      the REVISION files of the game server and the relay exist, and the wire protocol at the server's revision equals the protocol at the engine
             revision of the newest `killchain-v<N>` release (`gh release view`; read from the cached answer when GitHub is unreachable). Different protocol
             versions cannot play together: this is the check that would have named the v15 server in minutes.
  disk       free space on / is above 15 GB.

Python standard library only. Adapted from `scripts/net_probe.py` (branch net-probe): the UDP exchange and the QUIC Version Negotiation probe.
"""
import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time

SCHEMA = 1
RELEASE_REPO = 'kevstermcgee/RedEngineGames'
ENGINE_REPO = 'kevstermcgee/RedEngine'
PROTOCOL_FILES = ('crates/red_core/src/net/protocol.rs', 'src/net/protocol.rs')
RELAY_FILES = ('crates/red_core/src/net/relay.rs', 'crates/red_core/src/net/relay_server.rs')
PROTOCOL_RE = re.compile(r'pub const PROTOCOL_VERSION:\s*u16\s*=\s*(\d+)\s*;')
SHA_RE = re.compile(r'^[0-9a-f]{7,40}$')
STATS_FRESH_SECS = 150
UDP_TIMEOUT = 2.0  # seconds to wait for one UDP reply (the test shortens it); three tries are made before a probe fails
UDP_TRIES = 3
RELEASE_CACHE_SECS = 3600
RELEASE_CACHE_MAX_SECS = 7 * 86400
LOG_KEEP_LINES = 2000
# Relay control messages (relay.rs). `0` is not in the code alphabet, so 000000 can never be a live code.
TAG_REGISTER, TAG_REGISTERED, TAG_RESOLVE, TAG_RESOLVED, TAG_NOT_FOUND, TAG_UNREGISTER = 1, 2, 3, 4, 5, 7
IMPOSSIBLE_CODE = b'000000'
CHECKS = ('units', 'relay', 'public', 'game', 'mappings', 'server', 'drift', 'disk')


def res(status, detail, **extra):
    """One check's result: status is ok, warn (reported, does not fail the run) or fail."""
    out = {'status': status, 'detail': detail}
    out.update(extra)
    return out


def mask_ip(ip):
    """73.157.20.9 -> 73.157.x.x: the report is kept and pasted around."""
    parts = str(ip).split('.')
    return '.'.join(parts[:2] + ['x', 'x']) if len(parts) == 4 else str(ip)


# ----------------------------------------------------------------------------------------------------------------------------------------
# Everything that touches the machine is a method here, so the test can replace it.

class Host:
    """The real machine: systemd, the journal, git, gh, the disk, the clock."""

    def __init__(self, repos=()):
        self.repos = [r for r in repos]

    def now(self):
        return time.time()

    def run(self, cmd, timeout=20, merge=False):
        """(returncode, stdout) of a command (with stderr folded in when `merge`); (None, '') when it cannot be run or times out."""
        try:
            p = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT if merge else subprocess.PIPE, text=True, timeout=timeout)
            return p.returncode, p.stdout
        except (OSError, subprocess.SubprocessError):
            return None, ''

    def unit_state(self, unit):
        return self.run(['systemctl', '--user', 'is-active', unit], 10)[1].strip() or 'unknown'

    def journal(self, unit, minutes=10, grep=None):
        cmd = ['journalctl', '--user', '-u', unit, '--no-pager', '-o', 'short-unix', '--since', '-%dmin' % minutes]
        if grep:
            cmd += ['--grep', grep]
        rc, out = self.run(cmd, 20)
        return out if rc == 0 else ''

    def unit_started(self, unit):
        """The unit's last start as text journalctl --since accepts, or ''."""
        return self.run(['systemctl', '--user', 'show', unit, '-p', 'ActiveEnterTimestamp', '--value'], 10)[1].strip()

    def journal_since(self, unit, since, grep):
        rc, out = self.run(['journalctl', '--user', '-u', unit, '--no-pager', '-o', 'cat', '--since', since, '--grep', grep], 20)
        return out if rc == 0 else ''

    def portmap_status(self, path, ports):
        cmd = [sys.executable, path, 'status'] + [a for p in ports for a in ('--port', str(p))]
        return self.run(cmd, 25, merge=True)

    def disk_free(self, path='/'):
        return shutil.disk_usage(path).free

    def read_file(self, path):
        try:
            with open(path, encoding='utf-8') as f:
                return f.read()
        except OSError:
            return None

    def protocol_at(self, rev):
        """The wire PROTOCOL_VERSION in the engine source at `rev`: a local clone first (git show, read-only, no fetch), then GitHub."""
        for repo in self.repos:
            if not os.path.isdir(repo):
                continue
            for path in PROTOCOL_FILES:
                rc, out = self.run(['git', '-C', repo, 'show', '%s:%s' % (rev, path)], 20)
                m = PROTOCOL_RE.search(out) if rc == 0 else None
                if m:
                    return int(m.group(1))
        gh = self.gh()
        if gh:
            for path in PROTOCOL_FILES:
                rc, out = self.run([gh, 'api', '-H', 'Accept: application/vnd.github.raw', 'repos/%s/contents/%s?ref=%s' % (ENGINE_REPO, path, rev)], 25)
                m = PROTOCOL_RE.search(out) if rc == 0 else None
                if m:
                    return int(m.group(1))
        return None

    def blob(self, rev, path):
        """The git blob id of `path` at `rev` in a local clone, or None."""
        for repo in self.repos:
            if os.path.isdir(repo):
                rc, out = self.run(['git', '-C', repo, 'rev-parse', '%s:%s' % (rev, path)], 20)
                if rc == 0 and out.strip():
                    return out.strip()
        return None

    def gh(self):
        for cand in (shutil.which('gh'), '/usr/bin/gh', '/usr/local/bin/gh', os.path.expanduser('~/.local/bin/gh')):
            if cand and os.path.exists(cand):
                return cand
        return None

    def latest_release(self):
        """{'tag', 'engine_revision'} of the highest killchain-v<N> release, or None when gh is unavailable or says nothing usable."""
        gh = self.gh()
        if not gh:
            return None
        rc, out = self.run([gh, 'release', 'list', '-R', RELEASE_REPO, '--limit', '100', '--json', 'tagName'], 30)
        if rc != 0:
            return None
        try:
            tags = [r['tagName'] for r in json.loads(out)]
        except (ValueError, KeyError, TypeError):
            return None
        numbered = [(int(m.group(1)), t) for t in tags for m in [re.fullmatch(r'killchain-v(\d+)', t)] if m]
        if not numbered:
            return None
        tag = max(numbered)[1]
        rc, body = self.run([gh, 'release', 'view', tag, '-R', RELEASE_REPO, '--json', 'body', '--jq', '.body'], 30)
        m = re.search(r'"engine_revision"\s*:\s*"([0-9a-f]{7,40})"', body) if rc == 0 else None
        return {'tag': tag, 'engine_revision': m.group(1)} if m else None


# ----------------------------------------------------------------------------------------------------------------------------------------
# UDP helpers (adapted from scripts/net_probe.py)

def udp_exchange(ip, port, payload, timeout, accept=None, connected=True):
    """Sends `payload` from a fresh socket and waits for one reply. Returns (outcome, data, source): outcome is reply, refused (ICMP port unreachable,
    which only a connected socket sees) or silent. A connected socket takes replies from the target alone; an unconnected one (the public-name path,
    where a hairpinning router may answer from another address) takes them from anyone and says who."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        if connected:
            sock.connect((ip, port))
            sock.send(payload)
        else:
            sock.sendto(payload, (ip, port))
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                return 'silent', b'', None
            sock.settimeout(left)
            try:
                data, src = sock.recvfrom(4096)
            except socket.timeout:
                return 'silent', b'', None
            except ConnectionRefusedError:
                return 'refused', b'', None
            except OSError as e:
                return 'refused:' + e.__class__.__name__, b'', None
            if accept is None or accept(data):
                return 'reply', data, src
    finally:
        sock.close()


def ask(ip, port, payload, accept, connected=True):
    """udp_exchange with retries (a lost datagram is not an outage): (outcome, data, source, attempts)."""
    attempts = []
    outcome, data, src = 'silent', b'', None
    for _ in range(UDP_TRIES):
        outcome, data, src = udp_exchange(ip, port, payload, UDP_TIMEOUT, accept, connected)
        attempts.append(outcome)
        if outcome == 'reply' or outcome.startswith('refused'):
            break
    return outcome, data, src, attempts


def no_reply(what, where, attempts):
    if any(a.startswith('refused') for a in attempts):
        return '%s: nothing listens on %s (ICMP port unreachable)' % (what, where)
    return '%s: no reply from %s after %d tries' % (what, where, len(attempts))


def split_hostport(text, default_port):
    host, _, port = text.rpartition(':') if ':' in text else (text, '', '')
    return host, int(port) if port else default_port


def resolved_ok(d):
    """A Resolved (tag, fingerprint length byte and bytes, 8-byte claim token: 10 bytes for a host with no fingerprint) or CodeNotFound."""
    return (len(d) >= 10 and d[0] == TAG_RESOLVED) or d == bytes([TAG_NOT_FOUND])


def check_relays(cfg, router_ip):
    """The `relay` and `public` results. One registration made on loopback is looked up on both paths and removed at the end."""
    host, port = cfg['relay']
    out = {}
    reg = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)  # the registration belongs to this source address: Unregister must come from it too
    code = None
    try:
        reg.connect((host, port))
        reg.settimeout(UDP_TIMEOUT)
        attempts = []
        for _ in range(UDP_TRIES):
            try:
                reg.send(bytes([TAG_REGISTER, 0]))
                data = reg.recv(4096)
            except socket.timeout:
                attempts.append('silent')
                continue
            except OSError as e:
                attempts.append('refused' if isinstance(e, ConnectionRefusedError) else 'refused:' + e.__class__.__name__)
                break
            if len(data) == 7 and data[0] == TAG_REGISTERED:
                code = data[1:]
                attempts.append('reply')
                break
            attempts.append('odd')
        if code is None:
            out['relay'] = res('fail', no_reply('relay did not answer Register', '%s:%d' % (host, port), attempts) + ' (is red-relay running? is it at its registration limit?)')
        else:
            outcome, data, _, attempts = ask(host, port, bytes([TAG_RESOLVE]) + code, resolved_ok)
            if outcome != 'reply':
                out['relay'] = res('fail', no_reply('relay registered the code but did not answer Resolve', '%s:%d' % (host, port), attempts))
            elif data[0] != TAG_RESOLVED:
                out['relay'] = res('fail', 'relay answered CodeNotFound for the code it had just registered')
            else:
                out['relay'] = res('ok', 'Register + Resolve round trip on %s:%d (Resolved, %d bytes)' % (host, port, len(data)))
        out['public'] = check_public(cfg, code, router_ip)
    finally:
        try:
            reg.send(bytes([TAG_UNREGISTER]))  # the throwaway registration goes away at once instead of at the lease
        except OSError:
            pass
        reg.close()
    return out


def resolve_name(name):
    try:
        infos = socket.getaddrinfo(name, None, socket.AF_INET, socket.SOCK_DGRAM)
    except OSError as e:
        return None, str(e)
    return sorted({i[4][0] for i in infos})[0], None


def check_public(cfg, code, router_ip):
    name, port = cfg['public']
    ip, err = resolve_name(name)
    if ip is None:
        return res('fail', '%s does not resolve to an IPv4 address (%s): DuckDNS or the network is down' % (name, err))
    if router_ip and ip != router_ip:
        return res('fail', '%s resolves to %s but the router\'s public address is %s: the DuckDNS update is stale (duckdns-update.timer)' % (name, mask_ip(ip), mask_ip(router_ip)))
    payload = bytes([TAG_RESOLVE]) + (code or IMPOSSIBLE_CODE)
    outcome, data, src, attempts = ask(ip, port, payload, resolved_ok, connected=False)
    if outcome != 'reply':
        return res('fail', no_reply('Resolve through %s did not come back' % name, '%s:%d' % (mask_ip(ip), port), attempts) + ' (the router mapping, its hairpin or the relay)')
    got_code = data[0] == TAG_RESOLVED
    if code is not None and not got_code:
        return res('fail', 'Resolve through %s said CodeNotFound for a code the relay on loopback has: that name reaches a different relay' % name)
    return res('ok', 'Resolve through %s (%s:%d) answered %s' % (name, mask_ip(ip), port, 'Resolved' if got_code else 'CodeNotFound, as it must for an impossible code'))


def version_negotiation_request():
    """A QUIC long-header packet with a reserved ('greased') version, padded to 1200 bytes; a live QUIC listener answers it statelessly with the versions it speaks."""
    dcid, scid = os.urandom(8), os.urandom(8)
    first = 0xC0 | (os.urandom(1)[0] & 0x3F)
    head = bytes([first]) + (0x0A1A2A3A).to_bytes(4, 'big') + bytes([len(dcid)]) + dcid + bytes([len(scid)]) + scid
    return head + os.urandom(1200 - len(head)), dcid, scid


def is_version_negotiation(data, our_dcid, our_scid):
    if len(data) < 7 or not data[0] & 0x80 or data[1:5] != b'\0\0\0\0':
        return False
    dl = data[5]
    if len(data) < 7 + dl:
        return False
    sl = data[6 + dl]
    return data[6:6 + dl] == our_scid and data[7 + dl:7 + dl + sl] == our_dcid


def quic_listens(ip, port, connected):
    packet, dcid, scid = version_negotiation_request()
    return ask(ip, port, packet, lambda d: is_version_negotiation(d, dcid, scid), connected=connected)


def check_game(cfg):
    port = cfg['game_port']
    outcome, _, _, attempts = quic_listens('127.0.0.1', port, True)
    if outcome != 'reply':
        return res('fail', no_reply('the game server does not answer a QUIC probe', '127.0.0.1:%d' % port, attempts))
    name, _ = cfg['public']
    ip, err = resolve_name(name)
    if ip is None:
        return res('fail', '%s does not resolve (%s)' % (name, err))
    outcome, _, _, attempts = quic_listens(ip, port, False)
    if outcome != 'reply':
        return res('fail', no_reply('the game server answers on loopback but not through %s' % name, '%s:%d' % (mask_ip(ip), port), attempts) + ' (the router mapping or its hairpin)')
    return res('ok', 'a QUIC listener answers on 127.0.0.1:%d and through %s' % (port, name))


# ----------------------------------------------------------------------------------------------------------------------------------------
# the other checks

def check_units(cfg, host):
    bad = []
    for unit in cfg['units']:
        state = host.unit_state(unit)
        if state != 'active':
            bad.append('%s is %s' % (unit, state))
    if bad:
        return res('fail', '; '.join(bad))
    return res('ok', '%d units active (%s)' % (len(cfg['units']), ', '.join(u.split('.')[0] for u in cfg['units'])))


PORTMAP_HEAD = re.compile(r'router (\S+), this machine (\S+), public IPv4 (\S+)')
PORTMAP_LINE = re.compile(r'^UDP (\d+): (.*)$')
PORTMAP_MAPPED = re.compile(r'^(.+?) -> ([\d.]+):(\d+) \(lease (\d+)s\)( +\[NOT OURS\])?$')


def parse_portmap(text):
    """`red-portmap.py status` -> {'router', 'this', 'public', 'ports': {port: {'mapped', 'ours', 'to', 'lease', 'raw'}}}. The shapes are the script's own."""
    out = {'router': None, 'this': None, 'public': None, 'ports': {}}
    for line in text.splitlines():
        m = PORTMAP_HEAD.search(line)
        if m:
            out['router'], out['this'], out['public'] = m.groups()
            continue
        m = PORTMAP_LINE.match(line.strip())
        if not m:
            continue
        port, rest = int(m.group(1)), m.group(2)
        mm = PORTMAP_MAPPED.match(rest)
        if mm:
            out['ports'][port] = {'mapped': True, 'ours': mm.group(1) == 'red-engine %d' % port and not mm.group(5), 'to': mm.group(2), 'internal_port': int(mm.group(3)),
                                  'lease': int(mm.group(4)), 'raw': rest}
        else:  # `not mapped`, or the router's error for this port
            out['ports'][port] = {'mapped': False, 'ours': False, 'to': None, 'lease': None, 'raw': rest, 'error': None if rest == 'not mapped' else rest}
    return out


def check_mappings(cfg, status):
    rc, text = status
    ports = cfg['map_ports']
    if rc is None:
        return res('fail', 'could not run %s (is it installed?)' % cfg['portmap']), None
    info = parse_portmap(text)
    if not info['this']:
        return res('fail', 'the router did not answer UPnP (%s)' % (text.strip().splitlines()[-1][:120] if text.strip() else 'no output')), info
    bad, warn = [], []
    for port in ports:
        p = info['ports'].get(port)
        if p and p.get('error'):
            bad.append('UDP %d: the router said: %s' % (port, p['error'][:120]))
        elif not p or not p['mapped']:
            bad.append('UDP %d is not mapped on the router (red-portmap.timer restores it every 10 minutes)' % port)
        elif not p['ours']:
            bad.append('UDP %d is held by another rule: %s' % (port, p['raw']))
        elif p['to'] != info['this'] or p['internal_port'] != port:
            bad.append('UDP %d points at %s:%d but this box is %s' % (port, p['to'], p['internal_port'], info['this']))
        elif p['lease'] != 0:
            warn.append('UDP %d has a timed lease (%ds) that will lapse; the timer converts it to permanent' % (port, p['lease']))
    if bad:
        return res('fail', '; '.join(bad)), info
    if warn:
        return res('warn', '; '.join(warn)), info
    return res('ok', 'UDP %s mapped (permanent) to %s' % (', '.join(str(p) for p in ports), info['this'])), info


STATS_RE = re.compile(r'stats: players (\d+) \| phase (\S+)')
REJECTED_RE = re.compile(r'\| rejected v(\d+) m(\d+) k(\d+) b(\d+) f(\d+) o(\d+)')
JOINS_RE = re.compile(r'\| joins (\d+)')
REASONS = ('version', 'map', 'needs-key', 'bad-key', 'full', 'other')


def last_stats(journal):
    """(unix time, line) of the newest stats line in a `-o short-unix` journal, or None."""
    found = None
    for line in journal.splitlines():
        if 'stats: players ' in line:
            try:
                found = (float(line.split()[0]), line)
            except (ValueError, IndexError):
                continue
    return found


def check_server(cfg, host, prev):
    """Returns (result, new_state): `prev` is the previous run's record of the server's counters."""
    unit = 'red-game@%s.service' % cfg['game']
    journal = host.journal(unit, 10)
    stats = last_stats(journal)
    if not stats:
        return res('fail', 'no stats line from %s in the last 10 minutes: the server is silent (red-health restarts it after 240 s)' % unit), None
    ts, line = stats
    age = host.now() - ts
    m = STATS_RE.search(line)
    players = int(m.group(1)) if m else None
    if age >= STATS_FRESH_SECS:
        return res('fail', 'the server\'s last stats line is %d s old (limit %d s)' % (age, STATS_FRESH_SECS)), None
    rej, joins = REJECTED_RE.search(line), JOINS_RE.search(line)
    if not rej:
        return res('ok', 'stats line %d s old, %s player(s); this build predates the reject counters, so refusals are not visible' % (age, players), players=players), None
    counts = [int(x) for x in rej.groups()]
    state = {'stats_ts': ts, 'joins': int(joins.group(1)) if joins else None, 'rejected': dict(zip(REASONS, counts))}
    detail = 'stats line %d s old, %s player(s), refused so far: %s' % (age, players, ' '.join('%s %d' % (k, v) for k, v in state['rejected'].items() if v) or 'none')
    if prev and isinstance(prev.get('rejected'), dict) and prev.get('stats_ts') != ts:
        before = [int(prev['rejected'].get(k, 0)) for k in REASONS]
        if sum(counts) >= sum(before) and all(c >= b for c, b in zip(counts, before)):
            grew = [(k, c - b) for k, c, b in zip(REASONS, counts, before) if c > b]
            joined = state['joins'] is not None and prev.get('joins') is not None and state['joins'] > prev['joins']
            if grew and not joined:
                lines = [l for l in host.journal(unit, 15, 'join refused').splitlines() if 'join refused' in l]
                last = lines[-1].split(': ', 1)[-1][:240] if lines else None
                reason = 'joins were refused since the last check (%s) and nobody joined' % ', '.join('%s +%d' % g for g in grew)
                return res('fail', reason + ('; last: ' + last if last else ''), players=players, last_refusal=last), state
        # else: the server restarted (the counters started over) or somebody joined: nothing to say
    return res('ok', detail, players=players), state


def read_revision(host, path):
    text = host.read_file(path)
    rev = text.strip().split()[0] if text and text.strip() else None
    return rev if rev and SHA_RE.match(rev) else None


def check_drift(cfg, host, release_cache):
    """Returns (result, new release cache)."""
    srv_path, relay_path = cfg['server_revision'], cfg['relay_revision']
    srv, relay = read_revision(host, srv_path), read_revision(host, relay_path)
    missing = ['%s (%s)' % (p, 'missing or not a commit id') for p, r in ((srv_path, srv), (relay_path, relay)) if not r]
    if missing:
        return res('fail', 'REVISION file(s) missing: ' + '; '.join(missing) + ' (write them when you deploy: docs/HOSTING.md)'), release_cache
    release, cache = None, release_cache
    now = host.now()
    if release_cache and now - release_cache.get('fetched', 0) < RELEASE_CACHE_SECS:
        release = release_cache
    else:
        got = host.latest_release()
        if got:
            release = cache = dict(got, fetched=now)
        elif release_cache and now - release_cache.get('fetched', 0) < RELEASE_CACHE_MAX_SECS:
            release = release_cache
    srv_proto = host.protocol_at(srv)
    relay_proto = host.protocol_at(relay) if relay != srv else srv_proto
    facts = {'server_revision': srv, 'server_protocol': srv_proto, 'relay_revision': relay, 'relay_protocol': relay_proto}
    unit = 'red-game@%s.service' % cfg['game']
    started = host.unit_started(unit)
    said = re.findall(r'protocol: wire v(\d+)', host.journal_since(unit, started, 'protocol: wire v')) if started else []
    if said and srv_proto is not None and int(said[-1]) != srv_proto:
        return res('fail', 'REVISION says the server is revision %s (protocol v%d) but the running binary reports protocol v%s: REVISION is stale, write it at every deploy'
                   % (srv[:10], srv_proto, said[-1]), **facts), cache
    if srv_proto is None:
        return res('warn', 'cannot read PROTOCOL_VERSION at the server\'s revision %s (no local clone has it and GitHub is unreachable)' % srv[:10], **facts), cache
    if not release:
        return res('warn', 'server is revision %s (protocol v%d); the latest release was not compared (gh unavailable and nothing cached)' % (srv[:10], srv_proto), **facts), cache
    rel_proto = host.protocol_at(release['engine_revision'])
    facts.update(release_tag=release['tag'], release_engine_revision=release['engine_revision'], release_protocol=rel_proto)
    if rel_proto is None:
        return res('warn', 'cannot read PROTOCOL_VERSION at %s\'s engine revision %s' % (release['tag'], release['engine_revision'][:10]), **facts), cache
    if rel_proto != srv_proto:
        return res('fail', 'protocol drift: the server (revision %s) speaks v%d but %s (engine %s) speaks v%d, so its players cannot join: redeploy the server from the release\'s engine revision, or release again'
                   % (srv[:10], srv_proto, release['tag'], release['engine_revision'][:10], rel_proto), **facts), cache
    # The relay carries no game protocol, but its own wire code must match what the clients were built with (the stale relay of 2026-10-08).
    pairs = [(f, host.blob(relay, f), host.blob(release['engine_revision'], f)) for f in RELAY_FILES]
    differ = [f for f, a, b in pairs if a is not None and b is not None and a != b]
    facts['relay_code_differs'] = differ
    note = 'server revision %s and %s both speak protocol v%d' % (srv[:10], release['tag'], srv_proto)
    if differ:
        return res('warn', note + '; the relay (revision %s) was built from different %s than the release: redeploy it if the relay wire format changed' % (relay[:10], ' and '.join(os.path.basename(f) for f in differ)), **facts), cache
    return res('ok', note, **facts), cache


def check_disk(cfg, host):
    free = host.disk_free('/') / 1e9
    if free < cfg['min_free_gb']:
        return res('fail', 'only %.1f GB free on / (limit %g GB): run disk-guard --now' % (free, cfg['min_free_gb']), free_gb=round(free, 1))
    return res('ok', '%.1f GB free on /' % free, free_gb=round(free, 1))


# ----------------------------------------------------------------------------------------------------------------------------------------

def load_json(path):
    try:
        with open(path, encoding='utf-8') as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def write_atomic(path, text):
    tmp = path + '.tmp'
    with open(tmp, 'w', encoding='utf-8') as f:
        f.write(text)
    os.replace(tmp, path)


def append_log(path, line):
    try:
        with open(path, encoding='utf-8') as f:
            lines = f.read().splitlines()
    except OSError:
        lines = []
    lines.append(line)
    if len(lines) > LOG_KEEP_LINES + 500:
        lines = lines[-LOG_KEEP_LINES:]
    write_atomic(path, '\n'.join(lines) + '\n')


def guarded(name, fn, *args, shape='one'):
    """Runs one check; a bug in it (or a surprise from the machine) is that check's failure, not the end of the run that would hide every other check.
    `shape` is what the check returns: 'one' (a result), 'pair' (a result and something else) or 'both' (the relay and public results)."""
    try:
        return fn(*args)
    except Exception as e:  # noqa: BLE001 - the report must be written whatever happens
        crashed = res('fail', 'the %s check itself crashed: %s: %s' % (name, e.__class__.__name__, e))
        return {'one': crashed, 'pair': (crashed, None), 'both': {'relay': crashed, 'public': crashed}}[shape]


def run_checks(cfg, host, prev):
    """The report (a dict) for one run. `prev` is the previous report: counters and the release cache are carried from it."""
    started = time.monotonic()
    skip = set(cfg['skip'])
    results, state = {}, dict(prev.get('state') or {})
    status_cache = []

    def portmap():
        if not status_cache:
            status_cache.append(host.portmap_status(cfg['portmap'], cfg['map_ports']))
        return status_cache[0]

    info = None
    if 'units' not in skip:
        results['units'] = guarded('units', check_units, cfg, host)
    if not {'mappings', 'public'} <= skip:
        results['mappings'], info = guarded('mappings', lambda: check_mappings(cfg, portmap()), shape='pair')
    router_ip = info['public'] if info else None
    if not {'relay', 'public'} <= skip:
        relays = guarded('relay', check_relays, cfg, router_ip, shape='both')
        results.update({name: relays[name] for name in ('relay', 'public')})
    if 'game' not in skip:
        results['game'] = guarded('game', check_game, cfg)
    if 'server' not in skip:
        results['server'], new = guarded('server', check_server, cfg, host, state.get('server'), shape='pair')
        if new:
            state['server'] = new
    if 'drift' not in skip:
        results['drift'], cache = guarded('drift', check_drift, cfg, host, state.get('release'), shape='pair')
        if cache:
            state['release'] = cache
    if 'disk' not in skip:
        results['disk'] = guarded('disk', check_disk, cfg, host)
    results = {name: r for name, r in results.items() if name not in skip}
    ordered = {name: results[name] for name in CHECKS if name in results}
    failed = [n for n, r in ordered.items() if r['status'] == 'fail']
    warned = [n for n, r in ordered.items() if r['status'] == 'warn']
    at = host.now()
    report = {
        'schema': SCHEMA, 'unix': int(at), 'at': time.strftime('%Y-%m-%dT%H:%M:%S%z', time.localtime(at)),
        'ok': not failed, 'failed': failed, 'warnings': warned, 'seconds': round(time.monotonic() - started, 1), 'checks': ordered,
        'summary': summarize(ordered, failed, warned), 'state': state,
    }
    if failed:
        report['last_failure'] = {'unix': report['unix'], 'at': report['at'], 'summary': report['summary']}
    elif prev.get('last_failure'):
        report['last_failure'] = prev['last_failure']
    return report


def summarize(results, failed, warned):
    """One line: the verdict, or the first reason and how many more."""
    if failed:
        first = failed[0]
        more = ' (+%d more: %s)' % (len(failed) - 1, ', '.join(failed[1:])) if len(failed) > 1 else ''
        return 'FAIL %s: %s%s' % (first, results[first]['detail'], more)
    n = len(results)
    return 'ok: %d checks%s' % (n, ' (%d warning%s: %s)' % (len(warned), '' if len(warned) == 1 else 's', '; '.join('%s: %s' % (w, results[w]['detail']) for w in warned)) if warned else '')


def log_line(report):
    parts = ' '.join('%s=%s' % (n, r['status']) for n, r in report['checks'].items())
    return '%s %s %s (%.1fs)' % (report['at'], 'ok' if report['ok'] else 'FAIL', parts, report['seconds']) + ('' if report['ok'] else ' ' + report['summary'])


def parse_args(argv):
    home = os.path.expanduser('~')
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--relay', default='127.0.0.1:28016', help='the relay on this box (default %(default)s)')
    ap.add_argument('--public', default='red-engine.duckdns.org:28016', help='the relay through its public name (default %(default)s)')
    ap.add_argument('--game', default='killchain', help='the game server instance: unit red-game@NAME (default %(default)s)')
    ap.add_argument('--game-port', type=int, default=27015)
    ap.add_argument('--map-ports', default='28016,27015', help='router mappings that must exist (default %(default)s)')
    ap.add_argument('--units', help='comma list of user units that must be active (default: red-relay.service, red-game@GAME.service, red-portmap.timer, red-health.timer)')
    ap.add_argument('--state-dir', default=os.path.join(home, '.local', 'state', 'box'))
    ap.add_argument('--portmap', default=os.path.join(home, '.local', 'share', 'red-relay', 'bin', 'red-portmap.py'))
    ap.add_argument('--server-revision', help='default ~/.local/share/GAME/bin/REVISION')
    ap.add_argument('--relay-revision', default=os.path.join(home, '.local', 'share', 'red-relay', 'bin', 'REVISION'))
    ap.add_argument('--repo', action='append', help='a local clone of RedEngine to read PROTOCOL_VERSION from (repeatable; default ~/workspace/RedEngine-nightly and ~/workspace/RedEngine)')
    ap.add_argument('--min-free-gb', type=float, default=15)
    ap.add_argument('--skip', default='', help='comma list of checks not to run: ' + ', '.join(CHECKS))
    ap.add_argument('--json', action='store_true', help='print the report as JSON instead of the one-line verdict')
    ap.add_argument('-v', '--verbose', action='store_true', help='print every check to stderr')
    args = ap.parse_args(argv)
    skip = [s for s in args.skip.split(',') if s]
    bad = [s for s in skip if s not in CHECKS]
    if bad:
        ap.error('unknown check(s) to skip: %s' % ', '.join(bad))
    try:
        relay, public = split_hostport(args.relay, 28016), split_hostport(args.public, 28016)
        map_ports = [int(p) for p in args.map_ports.split(',') if p]
    except ValueError:
        ap.error('--relay/--public are HOST:PORT and --map-ports a comma list of numbers')
    units = [u for u in (args.units or '').split(',') if u] or ['red-relay.service', 'red-game@%s.service' % args.game, 'red-portmap.timer', 'red-health.timer']
    return {
        'relay': relay, 'public': public, 'game': args.game, 'game_port': args.game_port, 'map_ports': map_ports, 'units': units, 'state_dir': args.state_dir,
        'portmap': args.portmap, 'server_revision': args.server_revision or os.path.join(home, '.local', 'share', args.game, 'bin', 'REVISION'),
        'relay_revision': args.relay_revision, 'repos': args.repo or [os.path.join(home, 'workspace', 'RedEngine-nightly'), os.path.join(home, 'workspace', 'RedEngine')],
        'min_free_gb': args.min_free_gb, 'skip': skip, 'json': args.json, 'verbose': args.verbose,
    }


def main(argv=None, host=None):
    cfg = parse_args(sys.argv[1:] if argv is None else argv)
    host = host or Host(cfg['repos'])
    os.makedirs(cfg['state_dir'], exist_ok=True)
    json_path, log_path = os.path.join(cfg['state_dir'], 'selfcheck.json'), os.path.join(cfg['state_dir'], 'selfcheck.log')
    report = run_checks(cfg, host, load_json(json_path))
    try:
        write_atomic(json_path, json.dumps(report, indent=1) + '\n')
        append_log(log_path, log_line(report))
    except OSError as e:
        print('red-selfcheck: cannot write %s: %s' % (cfg['state_dir'], e), file=sys.stderr)
    if cfg['verbose']:
        for name, r in report['checks'].items():
            print('%-8s %-4s %s' % (name, r['status'], r['detail']), file=sys.stderr)
    print(json.dumps(report, indent=1) if cfg['json'] else 'red-selfcheck: ' + report['summary'])
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    sys.exit(main())
