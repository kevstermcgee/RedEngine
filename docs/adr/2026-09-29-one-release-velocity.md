# 2026-09-29. One release velocity: how a carried prop leaves the hand, the same offline and on the server
Status: accepted
Summary: A carried prop leaves the hand with the holder's own velocity plus player.throw_speed along the look, through one function on the client and the server; the holder's body ignores it for 10 ticks and the hold pose sweeps the prop's box, not a ray.

## Context
Fling Delivery measured it and Knockdown Alley read it in the source: a drop in the authoritative simulation (`MatchSim::interact`) gave the prop
1 m/s along the flat look direction and nothing of the player's motion, while the offline client gave it the player's planar velocity plus the
same toss. Online and offline were different games, and `sim` could not prove a throw. What distance a drop *did* travel in the sim came from the
holder's kinematic body shoving the prop it had just let go of for the ticks the two overlapped (walk on: 4.9 m, sprint on: 9-10 m, stop dead:
0.36 m), so the result depended on whether the player stopped, which nobody can read from the arc. Inheritance was horizontal only, so a
parcel could not be tossed up onto a ledge, and the hold pose kept a carried box out of walls with a ray from the eye, which passed over any
wall lower than the eye (a parapet, a mail-slot wall) and left the crate inside it.

## Decision
- **One function.** `sim::player::release_velocity(state, look, throw_speed)` = the holder's horizontal `velocity` and vertical `vy` plus
  `throw_speed` along the (pitched) look. `MatchSim::interact` and the offline client's `interact` both call it; the client no longer keeps its
  own position-delta velocity. `player.throw_speed` is authored per scene (0-30 m/s, default `THROW_SPEED` = 1, the old toss).
- **A grace window.** After a release the prop's colliders filter out the holder's body for `physics::RELEASE_GRACE_TICKS` (10) ticks, using
  rapier collision groups: every player body is a member of its slot's group, and a released prop's filter drops that one group until the window
  ends (`PropWorld::start_grace` / `end_grace`; picking it up again ends it early). Nothing else changes: other players, the map and other
  props are solid to it throughout, and a body that keeps running into the landed prop still pushes it, as it should.
- **A box sweep for the hold pose.** `PropWorld::hold_pose` sweeps the prop's own upright box (`box_clearance`, a rapier shape cast against
  fixed geometry) ahead of the player at the height it will be held, instead of a ray from the eye, so a wall lower than the eye stops the box.
  A box that already overlaps something where it starts (a wide prop in a tight spot) falls back to the old ray so it is never snapped to the chest.
- Documented in `describe physics` (`release_velocity`, `release_grace_ticks`, `hold_pose`) and SPEC; proven by `tests/throw.rs` on
  `tests/fixtures/throw.json` (standing < walking < sprinting; looking up 45 degrees at throw speed 6 lands on a 1.2 m ledge 2 m away; the real
  client and the sim release a crate to the same centimetre; a crate held facing a 1.2 m wall stops at its face; the grace window holds for 9
  ticks and ends) and by the unit test of the function itself.

## Consequences
- A throw is now a skill with a readable arc: sprint and release lands the crate about 4 m past the release point on the fixture, walk about
  1.7 m, standing 0.2 m; the same on the server and offline, so `checks.sim` can prove a game's throwing mechanic.
- Simulation results with drops changed, so a trace recorded before this ADR that contains a drop no longer replays; the committed
  `coin_run` fixture has no props and is unaffected. The wire protocol is unchanged (the server owns online drops; nothing about a release is predicted).
- Scenes that liked the old flat 1 m/s toss get it back with `player.throw_speed: 1` and standing still; a scene that wants a real throw sets
  `throw_speed` higher. Vertical inheritance means a jump-release lofts the prop, which games can now use.
- Undo: set `RELEASE_GRACE_TICKS` to 0 for the old shove behaviour; the function and the sweep are otherwise independent of it.
