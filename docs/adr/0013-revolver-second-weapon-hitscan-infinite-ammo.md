# 0013. The silver revolver: a second human weapon, hitscan, infinite ammo (for now)
Status: accepted

## Context
The human's primary weapon stays the bat. A silver revolver is added, chosen with the mouse wheel.
Ammo is unlimited for the moment but will be capped later, so the cap must be a small change.

## Decision
- **Weapons are data** (`weapons.rs`): `Weapon::{Bat, Revolver}`, the revolver's numbers (0.42 s between
  shots, 80 m range, 40 N·s knock-back), and `Ammo::{Infinite, Limited{loaded,capacity,reserve}}`.
  `REVOLVER_AMMO = Ammo::Infinite` is the single switch; `Limited` (with `try_fire`, `reload`, an
  empty-chamber click already wired) is implemented and tested but unused. A reload key and an ammo readout
  are the only missing pieces when the cap arrives.
- **Wheel = switch** (any direction cycles; human only; not while carrying a prop). A 0.34 s lower/raise
  animation swaps the models at the halfway point. Left click swings the bat *or* fires the revolver.
- **Hitscan**: a bullet is a ray from the eye along the crosshair, tested against the exact static
  shapes (`hit::raycast_shapes`) and the loose-prop physics world (`PropWorld::ray_props`), nearest wins.
  A hit logs, flashes the object and gives a loose prop an impulse (`strike_impulse`); firing always
  makes the shot sound (unlike the bat's thunk, which only plays on contact).
- **Models are procedural** (`revolver.rs`, ADR 0008): steel frame/barrel/fluted cylinder, walnut grip,
  hand, sleeve and an emissive muzzle flash. `HeldPart` gained `weapon`, `emissive`, `muzzle_flash`, so the
  renderer draws only the active weapon's pieces (first- and third-person) and the flash only while lit.
- **Third person**: the gun arm raises to aim where the player looks and kicks on a shot; the gun is welded
  to the same forearm bone as the bat.

## Consequences
- A third weapon = a `Weapon` variant, a parts function and one match arm in `re2`'s fire/transform code.
- Bullets do not yet leave decals or penetrate; the crosshair is gold whenever something is in range.
- Fixed a latent bug found while testing: ray queries for loose props filtered on "dynamic", but dormant
  props are fixed bodies, so nothing untouched could be hit (ADR 0012). The filter is now "belongs to a prop".
