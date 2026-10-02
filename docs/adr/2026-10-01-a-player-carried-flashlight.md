# 2026-10-01. A player-carried flashlight
Status: accepted
Summary: The player can carry a toggleable point light that follows the camera; the engine has no spotlight kind, so it is a Point, not a cone

## Context
A dark, atmosphere-driven map (a horror house, a cave, a blackout) needs the player to be able to light their
own way instead of being lit for them by authored fixtures. Nothing in the engine gave a scene that: lights are
static, authored scene objects (`LightKind::Directional` or `LightKind::Point`, `src/schema.rs`) — there is no
"attached to the player" light, and no cone/spot kind at all, only omnidirectional point lights and the one sun.

Adding a true spotlight kind would mean new shader and GPU-uniform work (`src/shaders/**`, `src/gpu.rs`) for a
falloff cone — real rendering-engine scope, not justified by one feature. A `Point` light already does the job
well enough for a hand-carried light (a pool of light around the player that dims with distance), and the
renderer already proves it is cheap to add one more dynamic light: `MAX_LIGHTS = 16` and `src/render.rs`
already selects the *nearest* point lights to the camera every frame, so a light that is always at the camera
is always picked regardless of how many other lights a scene authors.

## Decision
- **`"flashlight": true | false`** (root scene key, default `false`), parsed exactly like the existing `"music"`
  key (`src/schema.rs::parse_scene`, same error shape; added to `src/strict.rs`'s root allow-list and
  `red_engine2 describe scene`).
- **No new light is authored in JSON.** When `scene.flashlight` is true, the standard client (`src/bin/re2/`)
  synthesizes one `Point` light (id `"__flashlight"`) the first time it is needed and updates it in place every
  frame — the same established pattern `avatar.rs` already uses to mutate the player's own body object live,
  just applied to `Scene.lights` instead of `Scene.objects`. Position is `eye + camera.forward() * 0.3` (reusing
  the exact `tick_eye()`/`camera.forward()` calls `feedback.rs::draw_own_shot` already uses for the muzzle), so
  there is no new input-to-world-space code, only a light reusing it.
- **The `T` key toggles it** (checked against every `KeyCode::` already bound in `src/bin/re2/*.rs` and
  `src/controller.rs` — free). Starts on: a found light already in hand needs no fumbling with a menu.
- **No shadow casting.** A flashlight that casts moving shadows every frame is a real, ongoing render cost; the
  atmosphere this exists for does not require it, and omitting it keeps the feature exactly as small as the
  problem (one more dynamic point light, nothing else).
- **No cone, no falloff edge, no battery.** It is a Point light players already understand (every other light in
  the engine is one); a battery/drain mechanic is a gameplay decision left to the scene that uses this, not the
  engine (the scene's own `vars`/`rules` can already track and display a resource if a game wants one).

## Consequences
A dark scene can now ask for real player-driven illumination with one boolean and no custom client. The
light has no shadows and no cone shape, so a scene relying on precise shadow-play from it would need the real
spotlight feature this ADR deliberately did not build. To undo: remove the `flashlight` field/parsing (`schema.rs`,
`strict.rs`, `describe.rs`), the `T` binding (`events.rs`), and `sync_flashlight`/`flashlight_light`
(`frame.rs`) — no scene that does not set `"flashlight": true` is affected either way.
