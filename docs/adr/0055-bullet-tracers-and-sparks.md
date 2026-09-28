# 0055. Bullet tracers and impact sparks
Status: accepted

## Context
Every weapon here is hitscan: a shot happens in one tick and leaves nothing behind. On screen that reads as nothing at all: no sense of where a burst went, no way to see that the
sniper on the far deck is shooting at you, no sign of where a bullet landed. Sound and the muzzle flash say *that* someone fired; the eye needs to see *where*. The renderer has no
line or particle pass, and adding one for a tenth of a second of light per shot is a lot of machinery.

## Decision
A pool of hidden glowing boxes in the scene does it (`streaks`, headless). `streaks::add_pool` adds 64 unit boxes with `collide: false` and `movable: false` before the renderer is built (it
takes its meshes from the scene at creation, exactly as for the avatar pool), and `Streaks::update` each frame lays live ones along their shots (a box scaled to the shot's length and a few
centimetres thick, turned to point down it), shrinks and dims them over about a tenth of a second, puts a spark (a tumbling cube that pops and fades over 0.18 s) where a shot landed, and hides the rest.
The oldest gives way when the pool is full.

The client works out where a shot went itself, from what it already knows: our own shot from the camera ray (predicted, like the flash), another player's from where they stood and the way the
snapshot says they aimed, delayed to when their avatar is drawn firing (the same moment as their sound). The ray is tested against the map's solid shapes and the drawn players (and us, for
someone else's shot); a pellet gets a tracer each, red sparks mark a person and warm ones a wall. It is presentation only: what a shot really hit is the server's word (ADR 0053).

## Consequences
- Firefights are legible: streaks cross the arena, a burst's spread is visible, sparks mark misses on walls, and a tracer aimed at you says so before the damage arc does.
- No new pipeline, shader or asset: the streaks are ordinary scene meshes, so they are lit, culled and shadowed like any other (thin enough that the shadow is invisible).
- Limits: a tracer is opaque (it fades by thinning and dimming, not by alpha); the ambient-occlusion and outline pass treats it as geometry; props are not part of the client's ray (a shot
  at a crate ends at the wall behind it, drawn); at most 64 at once.
