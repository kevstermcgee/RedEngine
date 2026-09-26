# 0036. Repeated bounded online rule state (protocol v4)
Status: accepted

## Context
The authoritative server ran data-authored rules, but network clients only received ordinary world snapshots and match-flow status.
Replicating one-shot `hide` or variable-change events would leave a client permanently wrong after loss and could not initialize a
late joiner. The offline and online clients also needed to use the same genre-neutral presentation.

## Decision
- Protocol v4 adds a signed `RuleState` datagram: sequence, round and server tick, up to 16 named `f64` variables, 256 hidden scene-object
  indices, the latest event with its tick, and the terminal outcome. Object indices are safe because the handshake already requires
  the same map hash. The maximum form plus authentication tag remains below `MAX_PACKET`.
- The server sends the complete current state beside every 5 Hz `Status`, including immediately after join/status changes. This is a
  snapshot, not a delta: the client ignores older sequences and naturally recovers after loss, reconnect, or late join.
- `re2` resolves hidden indices against its parsed scene and feeds both offline and online variables/events/outcome through
  `ui::rules::hud_layout`. A recent event is shown for two seconds by comparing authoritative ticks.
- Rule logic remains server-only online. This message contains presentation state, not effects or executable rules.

## Consequences
Data-authored online games now present shared counters, collected/hidden objects and outcomes consistently. The explicit limits keep
decode work, allocations and packet size bounded; games needing more presentation state must introduce pagination or a new protocol
version rather than silently enlarging UDP datagrams. Variables remain global, as decided in ADR 0020.
