#!/usr/bin/env python3
"""Open and renew UDP port mappings on the home router through UPnP (no admin password).

  red-portmap.py status|enable|remove [--port N ...]

Each mapping is a short lease described as 'red-engine <port>' that points at this machine's
LAN address. `enable` is idempotent and meant to run from a timer: it renews our own leases,
follows a changed LAN address, and refuses to overwrite a mapping that is not ours. It never
touches the router's firewall, DMZ or unrelated rules.
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
LEASE = 3600
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


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('action', choices=['status', 'enable', 'remove'])
    parser.add_argument('--port', type=int, action='append', help='UDP port (repeatable; default %s)' % DEFAULT_PORTS)
    args = parser.parse_args()
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
                soap(url, 'AddPortMapping', {
                    'NewRemoteHost': '', 'NewExternalPort': port, 'NewProtocol': 'UDP', 'NewInternalPort': port,
                    'NewInternalClient': ip, 'NewEnabled': 1, 'NewPortMappingDescription': label(port),
                    'NewLeaseDuration': LEASE})
                print('UDP %d -> %s:%d (lease %ds)' % (port, ip, port, LEASE))
        except (RuntimeError, OSError, ET.ParseError) as error:
            print('UDP %d: %s' % (port, error))
            failed = True
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
