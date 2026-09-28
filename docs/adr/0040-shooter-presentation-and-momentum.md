# 0040. Shooter presentation and momentum
Status: accepted
Summary: Shooter sight anchors, automatic fire, pellets and replicated momentum

## Context
Generic revolver placement made the firearm library float away from its sleeve and retain hip
yaw while aiming. Solid optics blocked the sight line. Arena games also need velocity carried
between movement ticks rather than simply increasing direct movement speed.

## Decision
- Geometry and presentation share firearm sight/grip anchors. ADS removes hip rotation and puts
  the sight axis on the camera ray. Optics use open housings; long guns gain support hands and
  continuous forearms. The camera's FOV tangent ratio compensates zoom sensitivity.
- Automatic trigger repetition and deterministic shotgun pellets run in both combat paths.
- Scene-authored acceleration, air acceleration, friction and speed caps opt into momentum.
  Default zero acceleration preserves legacy direct-speed movement.
- Horizontal velocity belongs to PlayerState, authoritative snapshots, prediction and replay.
  Protocol v6 and trace v2 reject incompatible peers/recordings.
- The weapon_poses example exercises the actual LiveRenderer headlessly for visual inspection.

## Consequences
Tactical and arena games can choose different movement responses without simulation forks.
Finite per-weapon magazines, timed reloads, projectile/splash weapons, team rules and timed
resource pickups remain separate work; these changes do not claim those features.
