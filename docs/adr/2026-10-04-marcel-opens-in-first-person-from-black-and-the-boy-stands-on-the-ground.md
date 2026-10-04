# 2026-10-04. Marcel opens in first person from black; no birdsong; the boy stands on the ground and bushes are walls
Status: accepted
Summary: `audio.birds: false`, `player.fade_in`, feet planted under the walk cycle, and leafy shrubs and willows that block the player at their crown, not just their trunk.

## Context
Playing Marcel showed five rough edges: birdsong the player did not want, white blobs on the hawthorns that read as nothing, a boy who hovered when he walked, an opening behind the character, and bushes the boy ran straight through.

## Decision
- `audio.birds: false` silences the songbirds, cuckoo and pigeon (`Ambience::birds_off`); the beds and the night owl stay, as the owl is not birdsong. The default is unchanged (birds on).
- `player.fade_in: secs` fades the picture in from black when play begins (a start card waits on black; the timer runs only while the game does). It rides the existing screen-effects pass as an opaque flash, and shows even when the HUD hides combat effects.
- The hawthorn's white blossom blobs are gone; they floated against the leaves.
- The walk cycle bends knees and swings hips but never lowers the pelvis, so the soles left the ground. `update_player_body` now solves the pose, finds the lowest foot and lowers the body by that much (at most 15 cm). The body also takes the interpolated foot height rather than the last tick's.
- Shrubs and willows block the player at their crown reach (`blocking_crown`); the trunk radius alone let the boy walk through a bush whose leaves filled his body band. Bracken, which has no trunk, stays walkable.
- Marcel opens in first person (`player.view` removed); Q still switches to the view from behind.

## Consequences
The physics was already within 7 mm of the drawn ground (tested), so the floating was the animation, not the height. Straight-line walks in tests now sidestep when a bush stops them. Open: spruce skirts and willow streamers can still touch the boy's head when he stands at their trunk.
