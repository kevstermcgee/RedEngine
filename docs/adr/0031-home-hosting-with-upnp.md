# 0031. Home hosting with UPnP: `portmap` and `red_server --upnp`
Status: accepted

## Context
The realistic first host is a friend's home PC. That needs the game's UDP port opened on the router. BlueEngine ships a Python helper
that talks to `http://<gateway .1>:1900/ctl/IPConn`: it guesses the gateway as the `.1` of the local subnet, uses the SSDP port as if it
were the router's HTTP port, and hard-codes a control path only some routers use, so on most routers it fails before doing anything.

## Decision
`src/net/upnp.rs` (standard library only) and `red_engine2 portmap status|enable|remove|keep`, plus `red_server --upnp`:
- **Discover, do not guess.** SSDP `M-SEARCH` (multicast, or a gateway given with `--router`) for an Internet Gateway Device, fetch its
  description from the answered `LOCATION` (case-insensitive headers, chunked or length-framed HTTP), and read the `controlURL` of its
  `WANIPConnection` / `WANPPPConnection` service, resolving it against `URLBase` or the description's own host, naming the service
  version the router actually offers.
- **Safe by construction.** Only ever for this machine's address (found by the route to the router), always with a lease, always
  described as ours, read back and verified; a mapping that exists but is not ours is never overwritten and never deleted; a router that
  only accepts permanent mappings is refused (and a permanent one the router made anyway is removed again) unless `--allow-permanent`.
- **Honest.** `status` reports the router, this machine, the public address, and warns when the "public" address is private or
  carrier-grade NAT (nothing can fix that from here). Router error codes are explained in plain words.
- **Lifecycle.** `--upnp` and `portmap keep` renew at half the lease and remove the mapping on exit.

## Consequences
Tested against an in-process fake router that implements the description and stateful SOAP endpoints (mapping, foreign mappings,
permanent-only, faults) and a fake SSDP responder. **Not tested against a real router** (CI has none); on a network with UPnP off or
CGNAT it says so and `docs/HOSTING.md` lists the alternatives (a cloud VM, the Docker image). IPv6 pinholes and NAT-PMP/PCP are not
implemented.
