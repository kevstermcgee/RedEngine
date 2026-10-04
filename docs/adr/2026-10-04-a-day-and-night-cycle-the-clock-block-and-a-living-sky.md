# 2026-10-04. A day and night cycle: the clock block and a living sky
Status: accepted
Summary: A scene clock drives the sun, moon, stars, sky colour, sun light, ambient and haze from the scene time, as a pure function that frame --hour and the sky contact sheet can draw at any moment.

## Context
The sky was a fixed gradient with one fixed sun: no time of day, no moon, no stars, no sunset. The brief for Marcel (`docs/analysis/2026-10-04-marcel-and-procgen-plan.md`) is a world that turns from day to night
and back, over and over, with soothing sunrises and sunsets and a breathtaking night sky. The renderer already routes everything the sky needs through one function (`build_globals_common`, driven by the scene time
`t`) and one uniform block, so a day cycle can be a pure function of `t` feeding the same uniforms, with new shader code only for what is new (stars, moon, sunset glow, haze).

## Decision
- A scene may declare `clock` (`daycycle.rs`): `day_secs` (real seconds per day), `start` (0 midnight, 0.25 sunrise, 0.5 noon, 0.75 sunset), `sun_max_deg`, `latitude_deg`, `fog` (haze per metre), `stars`, `moon`, `moon_phase`,
  `night_light`, and optionally the `light` it drives. Without a `clock` nothing changes (every existing scene renders as before).
- `Clock::state(t, day_offset)` returns a `DayState`: the sun's path (a great circle tilted from the vertical, rising in the east and culminating toward +Z), the moon opposite it with its phase advancing over a 29.5 day month,
  the sky's zenith and horizon colours keyed on the sun's height (night, a long coral and gold dawn and dusk, day), the glow low on the sun's side, the sun's colour and apparent size (larger and warmer near the horizon), the
  sun's light and a *separate* dim cool moonlight, the ambient (which takes the sky's own colour at twilight so the ground glows with the sunset instead of going black), the star visibility and the rotation of the celestial sphere.
  The scene time already includes whole days, so the day count is `floor(start + t/day_secs)`; a game that saves days only starts `t` later.
- Pure and tested: the sun rises in the east and culminates at the stated height; a whole day sampled every minute has no jump in any colour, light or direction; day is brighter than night and dusk warmer than noon; dawn and dusk
  differ; the stars appear after sunset and are gone before sunrise and turn once a day; the moon goes through its phases.
- Sun and moon are separate lights on purpose: blending one light's direction between them swung shadows 8 degrees a minute through dusk. The sun keeps its direction and fades out as it sets, so shadows lengthen; the moon is a fixed
  fill from its own place (no shadows). The clock drives the first directional light (or the one named by `light`), keeping its authored colour and intensity as multipliers, and supplies its own shadow-casting sun if the scene has none.
- Shader: `sky.wgsl` draws the sunset glow (a strong low glow on the sun's side, a broad wash, a pink Belt of Venus opposite), a layered procedural star field (five brightness classes with colour temperature, twinkle that is stronger
  near the horizon, stars kept at least a pixel wide with their energy conserved so they shimmer rather than crawl, glowing halos for the rare bright ones, all looked up in one cell per layer), a Milky Way band with dust lanes and a
  warm core, and a moon with phase, maria and earthshine that blocks the stars behind it. `scene.wgsl` blends the distance into the horizon colour (warmed toward the sun) when `fog` is set. The sun's halo is confined and fades once
  the sun is far below the horizon (a long tail of it had tinted the whole night sky red).
- Looking at it: `frame --hour 18.5` draws any clock hour; `sky scene.json out.png --hours ... [--look sun|moon]` draws a labelled contact sheet of a whole day. `tests/day_cycle.rs` renders noon, midnight and sunset and holds the sky
  to what the brief asks (blue at noon, dark with stars and no colour cast at midnight, warm at the horizon at sunset, deterministic), skipping where there is no GPU adapter.

## Consequences
- Any scene gets a living sky with one block. The look is stylised and soft, tuned by looking at contact sheets, not physically exact; the palette keys are in `daycycle.rs` and are the place to change the mood.
- Not here (later slices): bloom and colour grading, light shafts, shooting stars, clouds, a ground that takes the light well (the example scene is a flat plane and cones), and the infinite world.
- The shader pass costs a few noise lookups per sky pixel only at night; the day path is the old gradient plus the glow.
