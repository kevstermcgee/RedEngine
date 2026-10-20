#!/usr/bin/env python3
"""Open and keep UDP port mappings on the home router through UPnP (no admin password).

  red-portmap.py status|enable|remove [--port N ...]

Each mapping is PERMANENT, described as 'red-engine <port>', and points at this machine's LAN address. `enable` is idempotent and meant to run from a
timer (`red-portmap.timer`, every 10 minutes): it restores a mapping the router lost (a reboot), converts an old timed lease, follows a changed LAN
address, and refuses to overwrite a mapping that is not ours. It never touches the router's firewall, DMZ or unrelated rules. `status` prints the router's
view of each port and exits 0 either way (`deploy/red-selfcheck.py` reads that text, so keep its two line shapes).

Why permanent (2026-10-20). The first version asked for a one-hour lease and renewed it every 30 minutes. This router (a TP-Link Archer A7) IGNORES an
AddPortMapping that is identical to the mapping it already holds: the lease is not extended (measured: 3373 s left before and after a renewal), so each
mapping expired once an hour and stayed gone until the next timer run, about 40 seconds in which nobody could join through the relay or reach the game
server. It does accept a lease of 0 (no expiry), so a mapping is now added once and simply stays; the timer only restores one the router lost.
Changing a mapping (a new LAN address, an old timed lease) takes a DeletePortMapping first, because an identical-looking AddPortMapping changes nothing.

The control URL is http://<default gateway>:1900/ctl/IPConn; RED_PORTMAP_URL overrides it (the test uses a fake router). Python standard library only.
"""
import argparse
import os
import socket
import subprocess
import sys
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET

SERVICE = 'urn:schemas-upnp-org:service:WANIPConnection:1'
# Permanent (0): this router ignores an identical AddPortMapping, so a timed lease is never extended and lapsed for about 40 s every hour
# (2026-10-20, measured: lease 3373 s before and after a renewal). It accepts 0, so the mapping simply stays.
LEASE = 0
DEFAULT_PORTS = [28016, 27015]
SOAP_NS = '{http://schemas.xmlsoap.org/soap/envelope/}'


def gateway():
    out = subprocess.run(['ip', '-4', 'route', 'show', 'default'], capture_output=True, text=True).stdout.split()
    if 'via' not in out:
        sys.exit('red-portmap: no default route')
    return out[out.index('via') + 1]


def control_url():
    return os.environ.get('RED_PORTMAP_URL') or 'http://%s:1900/ctl/IPConn' % gateway()


def local_ip(gw):
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.connect((gw, 9))
        return probe.getsockname()[0]


def soap(url, action, fields):
    envelope = ET.Element(SOAP_NS + 'Envelope')
    body = ET.SubElement(envelope, SOAP_NS + 'Body')
    command = ET.SubElement(body, '{%s}%s' % (SERVICE, action))
    for name, value in fields.items():
        ET.SubElement(command, name).text = str(value)
    request = urllib.request.Request(url, data=ET.tostring(envelope), headers={
        'Content-Type': 'text/xml; charset="utf-8"', 'SOAPAction': '"%s#%s"' % (SERVICE, action)})
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            data = response.read(65536)
    except urllib.error.HTTPError as error:
        data = error.read(65536)
    values = {n.tag.rsplit('}', 1)[-1]: n.text for n in ET.fromstring(data).iter() if len(n) == 0}
    if 'errorCode' in values:
        raise RuntimeError('UPnP %s: %s' % (values['errorCode'], values.get('errorDescription', '')))
    return values


def existing(url, port):
    try:
        return soap(url, 'GetSpecificPortMappingEntry',
                    {'NewRemoteHost': '', 'NewExternalPort': port, 'NewProtocol': 'UDP'})
    except RuntimeError as error:
        if str(error).startswith('UPnP 714'):
            return None
        raise


def label(port):
    return 'red-engine %d' % port


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('action', choices=['status', 'enable', 'remove'])
    parser.add_argument('--port', type=int, action='append', help='UDP port (repeatable; default %s)' % DEFAULT_PORTS)
    args = parser.parse_args(argv)
    ports = args.port or DEFAULT_PORTS
    url, gw = control_url(), gateway()
    ip = local_ip(gw)
    failed = False
    if args.action == 'status':
        print('router %s, this machine %s, public IPv4 %s' % (gw, ip, soap(url, 'GetExternalIPAddress', {}).get('NewExternalIPAddress')))
    for port in ports:
        try:
            current = existing(url, port)
            ours = bool(current) and current.get('NewPortMappingDescription') == label(port)
            if args.action == 'status':
                print('UDP %d: %s' % (port, 'not mapped' if not current else
                      '%s -> %s:%s (lease %ss)%s' % (current['NewPortMappingDescription'], current['NewInternalClient'],
                       current['NewInternalPort'], current['NewLeaseDuration'], '' if ours else '  [NOT OURS]')))
            elif current and not ours:
                print('UDP %d: held by another rule (%s); leaving it alone' % (port, current.get('NewPortMappingDescription')))
                failed = True
            elif args.action == 'remove':
                if ours:
                    soap(url, 'DeletePortMapping', {'NewRemoteHost': '', 'NewExternalPort': port, 'NewProtocol': 'UDP'})
                print('UDP %d: removed' % port if ours else 'UDP %d: nothing to remove' % port)
            else:
                if ours:
                    if (current.get('NewInternalClient') == ip and str(current.get('NewInternalPort')) == str(port)
                            and str(current.get('NewLeaseDuration')) == str(LEASE)):
                        print('UDP %d: already mapped to %s:%d%s' % (port, ip, port, ' (permanent)' if LEASE == 0 else ''))
                        continue
                    # An identical AddPortMapping changes nothing on this router; replace the mapping so the new lease/address applies.
                    soap(url, 'DeletePortMapping', {'NewRemoteHost': '', 'NewExternalPort': port, 'NewProtocol': 'UDP'})
                soap(url, 'AddPortMapping', {
                    'NewRemoteHost': '', 'NewExternalPort': port, 'NewProtocol': 'UDP', 'NewInternalPort': port,
                    'NewInternalClient': ip, 'NewEnabled': 1, 'NewPortMappingDescription': label(port),
                    'NewLeaseDuration': LEASE})
                print('UDP %d -> %s:%d (%s)' % (port, ip, port, 'permanent' if LEASE == 0 else 'lease %ds' % LEASE))
        except (RuntimeError, OSError, ET.ParseError) as error:
            print('UDP %d: %s' % (port, error))
            failed = True
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
