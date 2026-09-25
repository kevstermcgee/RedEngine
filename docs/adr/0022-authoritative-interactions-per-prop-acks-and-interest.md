# 0022. Authoritative interactions, per-prop acknowledgement, and spatial interest management
Status: accepted (extends ADR 0016; completes the online part of ADR 0015 roadmap items 3, 5, 7 and 8)

## Context
ADR 0016 made movement and prop pushing authoritative but left pick-up, weapons and damage single-player only, sent every player and
every changed prop to every client, and tracked "what the client has" with one change cursor per client. Three limits followed: the
interesting game rules could not run online, bandwidth grew with the whole match instead of with what a client can see, and one
lost snapshot made the cursor resend everything since.

## Decision
- **Interactions live in `MatchSim`** (`sim/interact.rs`), not in `re2`: a client sends only buttons (`PlayerInput::interact / attack /
  reload / switch_weapon`, wire flags bits 3-6, **protocol version 2**). The server decides pick-up (the prop under the crosshair within
  the character's reach and carry limits, never one already held: `PropWorld` now tracks one carried prop *per player*), bat swings
  (windup ticks, then a ray), revolver shots (cooldown, ammo, reload), damage, death (drops what is carried) and respawn. Hits are ray
  tests against exact static shapes, loose props and player cylinders. Weapon numbers are scene data (`weapons`: damage, revolver ammo),
  so the previously dead `Ammo::Limited` path is live. `pickup`, `drop`, `shot`, `hit`, `kill`, `respawn` are **engine events** injected
  into the rules engine, so scene rules can score kills or end rounds with data only. Combat state is part of the match checksum; every
  server-side push goes through `apply_impulse`, so recordings replay bit for bit (`tests/interactions.rs`, `tests/net_interactions.rs`).
- **`PlayerSnap` carries** weapon, held prop and hit points (flags: crouch, swinging, dead). The carried prop's pose is published every
  tick like any moving prop, so all clients see it move. Clients do not predict interactions (only movement is predicted).
- **Per-prop acknowledgement** replaces the per-client cursor: for each client and each moving prop the server remembers the generation of
  the pose the client last *acknowledged* (`SentSnap` lists what each snapshot carried). A prop is sent when it changed after that and is
  relevant; the oldest-unconfirmed go first when more than 30 need sending. Nothing is sent twice once acknowledged, a lost packet resends
  only what it carried, and a prop that moved while out of range is simply still "unconfirmed" when the client arrives.
- **Interest management** (`sim/interest.rs`, on by default in `red_server` when the map has zones): zones are rooms, `portals` connect them,
  and a client hears about its own room and rooms within `interest.hops` (default 1) open portals. Players and props in other rooms cost
  no snapshot bytes; things in no zone are always relevant. Map authors write only zones and portals (validated at load); relevance is
  computed. In the Test Lab six clients cost 42% fewer bytes, 67% fewer prop records (`tests/net_interest.rs`, `tests/net_budget.rs`).
- **Budgets are tests**: worst-case snapshot size (35 bytes/player, 30 bytes/prop, well under one datagram), per-client bandwidth (idle and
  worst case), an average-tick ceiling, and allocations per match tick with eight players (`tests/net_budget.rs`, `tests/alloc_budget.rs`);
  wall-clock benches (`match/tick`, `net/*`) sit beside the existing ones in `benches/sim.rs`.

## Consequences
Adding an interaction: implement it in `interact.rs` on `MatchSim`, raise an engine event if rules should see it, add the button/field to the
protocol (bump `PROTOCOL_VERSION`, update the size formula and `net_budget`), test it in `interactions.rs` and over UDP. A v1 client is
refused by the existing version check; there is no compatibility layer. Not done: single-player `re2` still runs its own weapon code (moving it
onto a local `MatchSim` is the natural next step), rule state (variables, hidden objects) is not sent to clients, and the demo characters
(human, rat) remain compiled-in profiles rather than data.
