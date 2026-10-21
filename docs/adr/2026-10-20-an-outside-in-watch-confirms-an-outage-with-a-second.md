# 2026-10-20. An outside-in watch confirms an outage with a second probe and opens one issue
Status: accepted
Summary: A GitHub runner probes the home relay and the Killchain port hourly from the internet with no key, repeats a failed probe three minutes later instead of keeping state, opens one labelled issue that mentions the maintainers and that the next clean pass closes, and never publishes the home address.

## Context
The owner and their cousin never managed to play Killchain together, and for days nothing said why. Every check that existed ran inside the home network: the server's
journal ("0 joins, 0 bad packets" while clients were being refused), the LAN, the router's port-mapping table. The relay, the game server and the port-mapping timer were
all stopped between 2026-10-17 and 2026-10-20 and nothing noticed; a router mapping that lapsed for about 40 s an hour was visible only from outside. A connectivity
investigation on 2026-10-20 used GitHub-hosted runners as the outside, and that worked: a runner completed a relay Resolve and a QUIC handshake against the home server.
That probe (`scripts/net_probe.py`, run by hand on branch net-probe) is the only observer that sees what a friend sees. One of those manual runs had the relay answering
and the game port silent while the server was running: the outside view catches what the inside view cannot.

The box now also checks itself every five minutes (red-selfcheck, PR #215: units, the router's mappings, a hairpin Resolve, protocol drift between the server and the
released game). That is the inside view and it stays the only place that knows *why*; this watch does not repeat any of it. It answers one question from the internet:
does an outside packet get an answer from the relay and the game port, and if not, which link broke.

## Decision
- `.github/workflows/reachability.yml` runs `scripts/net_probe.py` every hour at minute 17 (and on `workflow_dispatch`; never on push or pull_request) from a GitHub runner
  against the owner's public name (`OWNER_HOST`, the name the game itself resolves). It needs no key: the relay check resolves a code that cannot exist and requires
  `CodeNotFound`; the game check sends a stateless QUIC Version Negotiation probe, then completes a QUIC/TLS handshake and stops (the join key is checked later, inside
  the encrypted Hello). Nothing it sends can join a match or create state on the relay.
- **A failure is confirmed, not counted.** The probe retries internally; if it still finds a required target unreachable (exit 1), the workflow probes once more
  `CONFIRM_AFTER_SECONDS` (180) later and only exit 1 on **both** probes is an outage. A first probe without a verdict (any other exit) is not repeated and is an error:
  a crash, a missing aioquic or a usage error is the same three minutes later, and one verdict from the second probe alone would not be confirmed. Three minutes outlast
  a server restart or a router re-adding a mapping and keep the watch stateless (no variable, artifact or comment carries a counter), at the price of ignoring an outage
  shorter than that.
- **The exit code is a contract.** 0 reachable, 1 a required target did not answer, 2 bad arguments, 3 the QUIC handshake could not be tried (no aioquic), 4 inconclusive
  (a required target failed while the runner could not get a UDP answer from public DNS resolvers, or the only failure is the runner's own resolver while a public
  resolver, asked directly, found the name; either way the failure says nothing about the server), 70 the probe crashed. Only 1 on both probes opens or updates the issue;
  everything else turns the run red and leaves the issue alone, so neither a bug in the probe nor a runner's own network or DNS announces an outage. When only the runner's
  resolver failed, the relay and the game port are still probed at the address the public resolver gave, so a real outage behind it is still exit 1.
- **One issue, the watch's own.** The standard title ("Killchain and the relay are not reachable from the internet"), the label `reachability` (made when the first issue
  is opened) and a bot author (the workflow's token) identify it: the label alone does not, since it is also the obvious label for a player's "can't connect" report,
  which the watch never comments on, reopens or closes. It mentions the maintainers listed in `.github/contributors.json`, so both are notified whatever they watch. An
  open issue gets a comment instead of a second issue (at most one every six hours from the schedule, every time from a manual run); the next clean first-attempt pass
  comments "Reachable again" and closes it; a run that failed once and then passed (`recovered`) neither opens nor closes. An outage that comes back within 24 hours of the
  close reopens that issue with a comment instead of opening a new one, so a link that goes up and down keeps one thread and does not mention everyone again. The way to
  silence the watch during a known outage is `gh workflow disable reachability.yml` (the issue body says so). The issue carries the run link, the plain-words verdict and
  the probe's JSON.
- **Only the watch touches the issue.** A manual run with any input set (another host or port) is an experiment: it reports in its summary and never opens, comments on or
  closes the issue.
- **The home address is never published.** The name is public; the address is not, and a CI log, a job summary and an issue in a public repository are public and outlive
  an address change. The probe hides every IPv4 address it prints as `x.x.x.x` (and IPv6 as `x:x::x`), not just the last octet, with a second pass over every string it
  writes (a crash's traceback included), and the issue and summary text is hidden again when it is rendered. Only the well-known public resolvers of the UDP control stay
  visible. The runner's own egress address keeps two octets only on a GitHub-hosted runner (a cloud network); run anywhere else, from the box included, it is hidden whole,
  because there the first octets are the home address's. What the name points at is recorded as a *kind* (`public`, `private`, `shared` for carrier-grade NAT): a name
  pointing at a private address is named as the cause without showing it. No artifact is uploaded.
- **The same script is the player's self-check** (`python3 scripts/net_probe.py --no-json`): the UDP control tells "this network blocks UDP" from "the server is
  unreachable", and the last lines name the failing link in plain words. The defaults are the owner's public server name and ports, no key, no default fingerprint (it
  changes whenever the identity is remade).
- The workflow has `contents: read` and `issues: write` only, hands the token to the two issue steps alone (the checkout does not leave it in `.git/config`, and
  aioquic is installed from wheels only), pins its actions like the other workflows, takes dispatch inputs through the environment, runs one at a time, and runs only in
  `kevstermcgee/RedEngine` (a fork does not probe the owner's server).
- `scripts/test_net_probe.py` (run by the python job on Linux and Windows, offline) holds the probe against fake relay, QUIC and DNS servers on loopback, and runs the
  workflow's own step scripts under bash with a fake probe and a fake `gh` whose `--jq` is the real jq: the status for each pair of exit codes, the second probe, one
  issue, the six-hour comment, the close, the reopen, and a person's labelled issue left alone. The steps' `if:` conditions are evaluated for every status, watch flag and
  event. The Linux python job also installs the pinned aioquic in a venv and runs the real QUIC handshake against an aioquic server on loopback, refusing to skip it.

## Consequences
An outage of the relay, the game server, the name or the home connection now reaches both maintainers as an issue within about an hour and five minutes, with a verdict that
says which link broke, and a recovery closes it within the hour after (an outage that returns within a day reopens the same issue). The watch cannot see what only one player's network does (a school firewall, CGNAT on their side,
IPv6-only), checks no game version or join key (a protocol mismatch is the self-check's to find), and is QUIC over UDP only: there is no TCP fallback to probe. GitHub
pauses scheduled workflows after 60 days without a push and can run them late; a paused watch is as silent as no watch, so docs/HOSTING.md, "Reachability watch", says so.
Cost: one to five minutes of a hosted runner per hour. Undo: delete the workflow; the script stays useful by hand.
