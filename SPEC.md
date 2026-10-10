# red_engine2 scene language

`red_engine2` renders a single JSON **scene** file straight to an MP4 (`red_engine2` CLI) or
lets you walk around it live in a window (`re2`). This is the complete reference for that JSON
format. Read this, not the source, to author scenes — the engine's Rust internals are an
implementation detail.

## Top-level shape

```json
{
  "meta": { "fps": 30, "duration": 6.0, "resolution": [1280, 720] },
  "background": { "sky_top": "#8fc7ff", "sky_bottom": "#eef6ff" },
  "ambient": { "color": "#ffffff", "intensity": 0.25 },
  "camera": { "fov": 50, "position": [0, 2, 8], "target": [0, 1, 0] },
  "player": { "character": "human", "fov": 90, "walk_speed": 5, "sprint_speed": 8, "crouch_multiplier": 0.45, "jump_speed": 5, "gravity": 18 },
  "jump_pads": [{ "id": "launch", "position": [0,0,4], "size": [2,2], "launch_speed": 11 }],
  "post": { "ao": 0.9, "outline": 0.65 },
  "lights": [ ... ],
  "zones": [ ... ],
  "objects": [ ... ]
}
```

`post` and `zones` are optional (see [Clarity post-pass](#clarity-post-pass-post) and
[Zones](#zones)). Other optional top-level keys: `schema_version`, `recipe`, `player`, `jump_pads`, `spawns`, `portals`, `interest`,
`prefabs`, `checks` (`red_engine2 describe scene` lists them all).

### Strict fields and versioning

**An unknown key is an error, never silently ignored** — a typo like `"pos"` or `"raduis"`, or `"color"` written
on an object instead of inside `material`, would otherwise do nothing and look like an engine bug. The error names
the path and the fix: `crate_1.pos: unknown field — did you mean `position`?`. The same applies to the camera, lights,
`material`, `post`, `zones`, `spawns`, `portals`, wall `openings`, prefab instances and every `checks` group (a
misspelled check group would otherwise mean the check never runs). A number field holding a string
(`"radius": "big"`) is an error too.

To keep a note or tool data in a scene, use the **extension namespace**: any key starting with `_`, `x-` or `x_`,
plus `notes` and `$comment`, is always allowed and never interpreted, at any level.

`"schema_version": 1` (optional; omitted means 1) pins the format. A scene that names a newer version than the
engine knows is refused with a message asking to update the engine. When the format changes incompatibly the version
is bumped and this section lists the migration (rename X to Y, wrap Z in W); until then there is nothing to migrate.

- `meta.fps` — integer, frames per second. `meta.duration` — seconds (float). `meta.resolution`
  — `[width, height]` in pixels; both are rounded up to even numbers for H.264 compatibility.
- `background` is either `{"sky_top": "#hex", "sky_bottom": "#hex"}` (a vertical gradient,
  sampled behind everything) or `{"color": "#hex"}` (flat).
- `ambient` is a flat, non-directional light applied to every surface — `intensity` is a
  small multiplier (0.1–0.4 is typical); it prevents unlit surfaces from going pure black.

## World space

Right-handed, **Y-up**. `+X` right, `+Y` up, `+Z` toward the viewer by convention (but the
camera can be placed anywhere). Keep the scene roughly inside `-15..15` on X/Z and `0..15` on
Y so numbers stay small and the default camera/light framing is sane — same spirit as the 2D
engine's `-8..8` world grid.

Rotations are Euler angles in **degrees**, applied in X, then Y, then Z order. Colors are
`#rrggbb` or `#rrggbba` hex strings.

## Tracks: constants vs. keyframes

Almost every numeric field (`position`, `rotation`, `scale`, light `color`/`intensity`, camera
`fov`, humanoid joint angles, ...) accepts **either**:

- a bare literal — `"position": [0, 1, 0]` — constant for the whole clip, or
- a keyframed track — `"position": {"keyframes": [{"t": 0, "value": [0,0,0]}, {"t": 2,
  "value": [3,0,0], "ease": "inout"}]}`

Only set what changes, when it changes; the engine fills in the rest by interpolation. Each
keyframe's `ease` describes how the segment **arriving at that keyframe** is interpolated (same
convention as the 2D engine). Omit `ease` for `"linear"`.

Easing options: `linear`, `in`, `out`, `inout`, `hold` (step, no interpolation), `back`
(slight overshoot), `bounce`, `elastic`.

Vector tracks (`position`, `rotation`, color) interpolate component-wise / channel-wise; you
never need to keyframe X, Y, Z separately.

### The game's own words (`ui`)

A top-level `"ui"` block says what the *game* tells its player; `hud` only chooses which engine panels show. It is presentation (no gameplay, no checksum), read
the same way by the client, `ui-shot`, `ui-check` and the headless state dump. `describe ui` has the full syntax; in short:

```json
"ui": {
  "title": "Moonlight Delivery",
  "labels": { "stamps": "Stamps" },
  "counters": [ { "var": "delivered", "of": 6, "label": "Parcels" }, { "var": "time_left", "label": "Time", "format": "clock" } ],
  "objective": [ { "if": "delivered >= 6", "text": "Open the garden gate" }, { "text": "Bring every parcel to the depot ({delivered} of 6)" } ],
  "start": { "title": "Moonlight Delivery", "text": "Carry the parcels before dawn.", "button": "Start" },
  "pause": "{stamps:stamp|stamps} so far",
  "end": { "victory": { "title": "Delivered!", "text": "All {delivered} parcels.", "button": "Play again" }, "default": { "title": "Time is up" } }
}
```

Variables, `end` outcomes (against the rules' `end` actions), expressions and `{placeholders}` are validated when the scene loads. Offline, the game waits for the start
card's button and puts the end card up when a rule ends the match; its button restarts the scene (Enter, Space, E or a click). Online the HUD uses the labels, counters
and objective; the cards are offline only. `ui-shot game-hud|game-start|game-end --scene S.json --var delivered=3 --outcome victory` draws them and
`pause` is a line for the pause menu, under the game's name: where a game with a clean screen (`"hud": {"enabled": false}`) keeps what its HUD would have said (`ui-shot pause --scene S.json --var stamps=3`). `ui-check --scene S.json` audits them at nine window sizes. A headless script plays them with `{"press": "start"}` / `{"press": "restart"}`.

### A day that turns to night (`clock`)

```json
"clock": { "day_secs": 720, "start": 0.27, "sun_max_deg": 62, "fog": 0.0035, "stars": 1.0, "moon": true }
```

A top-level `"clock"` makes the scene's time a time of day. `start` is where the day is when the scene begins (0 midnight, 0.25 sunrise, 0.5 noon, 0.75 sunset) and `day_secs` how many real seconds a day lasts. The sun and moon cross
the sky (the sun rises in the east, +X, and climbs toward +Z), the sky changes colour with the sun's height, the first directional light (or the one named by `light`) becomes the sun with its authored colour and intensity as
multipliers (a scene with none gets a shadow-casting sun), a dim cool moonlight fills the night, the ambient follows the sky, the stars come out after sunset and wheel overhead through the night, the moon goes through its phases
over 29.5 days (`moon_phase` sets day 0), and `fog` (density per metre, 0 for none) fades the distance into the horizon colour, warmed toward the sun at dusk. Without a `clock` the sky is whatever `sky`/`background` say.
Draw any moment with `frame scene.json out.png --hour 18.5`, and a whole day as a labelled contact sheet with `sky scene.json out.png [--hours 5,6,7,12,18,19,22] [--look sun|moon]`. Presentation only: the simulation does not read it.

### Keeping what the player did (`persist`) and a day's events

`"persist": ["days_lived"]` names scene `vars` the game keeps between sessions: they are loaded before the first tick and saved whenever one changes (per game, next to its settings). With a `clock`, the engine raises the events `sunrise` and `sunset` as the sun crosses the horizon, so `{"when": {"event": "sunrise"}, "do": [{"add": ["days_lived", 1]}]}` counts days lived. In `ui` text, `{days_lived:day|days}` is the number and the right word: "1 day", "12 days". `examples/marcel/marcel.json` uses all of it: its start card is the menu.

### The sounds of a world (`audio`)

```json
"audio": { "ambience": "nature", "music": { "dawn": "audio/dawn.json", "day": "audio/day.json", "dusk": "audio/dusk.json", "night": "audio/night.json" }, "music_volume": 0.6, "ambience_volume": 0.8 }
```

`"ambience": "nature"` plays the engine's countryside as the hour and the place call for it: wind that gusts, leaves in woods, crickets after dark, bees over flowers at noon, and birds by the real day (a dawn chorus of robins, blackbirds, wrens and chaffinches, scattered song and the odd cuckoo or pigeon by day, blackbirds at dusk, an owl at night, silence between; `"birds": false` removes the songbirds, cuckoo and pigeon and keeps the owl). `music` gives a score file (see `audio` in `describe`; paths are relative to the scene) for each mood; the sun's height crossfades between them, so the music follows the day (any subset works). It needs a `clock` for the hour; without one it is always midday. The player's music setting (N, the pause menu) controls the music and their sound setting the ambience. `re2 scene.json --headless --script s.json --dump d.json` writes `/audio/{beds,music,calls,recent}` so a script can assert on it. `red_engine2 audio report ambience.nature.wind` / `bird.robin` measure the sounds; `audio picture` draws them.

The mix: sounds run through buses (music, sfx, ui, ambience) with a voice limit each (the least important sound is cut off when it is full, a burst of one sound is thinned), `"reverb": {"decay": 1.4, "mix": 0.25}` puts the nature calls in a room, `"duck": {"events": ["hit"], "depth": 0.5, "hold": 1, "release": 1.5}` pulls the music down whenever a rule raises one of those events, and `"layers": [{"score": "audio/chase.json", "var": "danger", "above": 0.5, "fade": 3, "volume": 0.6}]` fades a score in while a rule variable is at least a value. Hold the sound to a standard in `checks.audio` (run by `verify`): `{"scores": {"lufs": [-33, -24], "peak_max": -6}, "sounds": [{"sound": "bird.robin", "lufs": [-28, -18]}], "soundscape": [{"name": "night", "sun": -40, "biome": "forest", "secs": 900, "calls": {"owl": [4, 500], "robin": [0, 0]}, "beds": {"crickets": [0.4, 1]}}]}`.

### An endless world (`procgen`)

```json
"procgen": { "seed": 7, "relief": 1.0, "trees": 1.0, "flowers": 1.0, "grass": 1.0 }
```

A top-level `"procgen"` makes the ground an endless generated world: rolling hills, open meadows, wildflower fields, groves, broadleaf forest, pinewoods and glades, with twenty real species (English oak, silver birch, Norway spruce, Scots pine, weeping willow, wild cherry, hawthorn, bracken, oxeye daisy, poppy, cornflower, lavender, bluebell, dandelion, buttercup, red clover, foxglove, harebell and two grasses). The same `seed` is the same world on every machine; the multipliers (0 to 3 for `relief` and `flowers`, 0 to 2 for `trees` and `grass`) change the hills and how thick things grow. It is the floor everywhere (a scene needs no ground object), tree trunks stop the player, plants sway in the wind, and chunks around the camera stream in as it moves. It combines with `clock` (a day that turns to night). Look at it without a renderer with `procgen map.png --seed 7 --size 400 [--biomes] [--grid]` (top-down map and plant counts) and at the plant models with `flora sheet.png [--species oak --variants 6]`; draw it with `frame scene.json out.png --hour 9`. `examples/endless_meadow.json` is the smallest scene. A child to wander it: `"player": { "humans_play_as": "boy", "view": "third", "mode": "peaceful", "walk_speed": 2.9, "sprint_speed": 5.4 }` (`view` is `first` or `third`; with a `clock` the day turns in play, and `re2 scene.json --headless --script s.json` can wait through it and take pictures). Design and limits: ADRs "An endless world" and "The streamed world".

## Camera

```json
"camera": {
  "fov": 50,
  "near": 0.1,
  "far": 200,
  "position": [0, 2, 8],
  "target": [0, 1, 0],
  "roll": 0
}
```

`fov` is vertical field of view in degrees (default 90, also the live viewer's base FOV). `position`/`target`/`fov`/`roll` are all tracks.
`target` is the world-space point the camera looks at — orbiting a subject is a `position`
track around a fixed `target`, not a rotation track on the camera itself.

## Player movement and launch pads

The optional `player` block gives a game its character policy and movement profile without forking engine code.
`character` may be `human` or `rat`; when present the authoritative server enforces that body. Omit it and players are Humans
(there is no character picker; `re2 --as` is a developer override). `fov`
is 60–120 degrees; `walk_speed` and `sprint_speed` are metres/second; `crouch_multiplier` scales horizontal
speed; `jump_speed` and `gravity` control the vertical arc. Sprint speed must not be below walk speed.
Omitting the movement fields keeps the engine defaults. Human offline play, the server, bots and prediction use
the same values. The rat character retains its character-specific body profile. `throw_speed` (0–30, default 1) is
the speed a released prop gets along the look direction (pitch included) on top of the player's own horizontal and
vertical velocity; the same release function runs offline and on the server, and for 10 ticks after a release the
holder's body ignores the prop, so the arc decides where it lands (`red_engine2 describe physics`).

Optional momentum fields: `acceleration` (0–100, default 0) enables acceleration/friction when positive;
`air_acceleration` (0–30, default 1), `friction` (0–30, default 6), and `max_speed`
(at least sprint speed, at most 50 m/s, default max(20, sprint speed)) tune that profile.
Ground friction slows released input; air movement preserves momentum and adds velocity along the
wish direction up to the speed cap. Zero acceleration retains immediate legacy movement.
Horizontal velocity is part of protocol v7 and later and replay format v2; older clients/traces are rejected.

Firearm presentation uses per-model sight/grip anchors. Iron sights have open rear notches, optics
have open housings, and aiming removes hip yaw/pitch. Mouse sensitivity follows the tangent ratio
of current/base FOV. Precision rifles magnify more than ordinary irons. Automatic weapons repeat
while held; the shotgun emits nine deterministic pellets whose damage sums to the configured shot
damage. The shared ammo pool and instantaneous reload remain prototype limitations.
`cargo run --example weapon_poses -- out/weapon-poses` renders hip/aim/recoil views of every firearm
using the real live renderer without a window.

Each `jump_pads` entry is a horizontal rectangle centered at `position.xz`, active when the player's
feet are at `position.y`. Contact sets vertical velocity to `launch_speed`; normal horizontal input
continues to apply. Use non-colliding emissive geometry to show the pad. Launch pads are deterministic
simulation data, not renderer effects, so server and predicted clients agree.

## Lights

Up to 256 lights may be authored. Each rendered view evaluates all directional lights plus the nearest point lights,
up to 16 active lights total. Each light has a `type` of `"directional"` or `"point"`.

```json
{ "id": "sun", "type": "directional", "direction": [-0.4, -1, -0.3],
  "color": "#fff4e0", "intensity": 3.0, "cast_shadows": true, "shadow_radius": 15 }

{ "id": "fill", "type": "point", "position": [-4, 3, 2],
  "color": "#88aaff", "intensity": 12.0, "range": 20 }
```

- `direction` (directional only) points **from** the light **toward** the scene; it's
  normalized automatically.
- `intensity` for directional lights is roughly "sun brightness" (1–4 typical); for point
  lights it's radiant power that falls off with distance (range ~8–30 typical, wattage-like
  10–40 typical).
- Exactly one light may set `"cast_shadows": true` (must be `"directional"`). It renders a
  shadow map; `shadow_radius` (world units, default 15) is the half-size of the orthographic
  shadow frustum, centered on `shadow_center` (`[x, y, z]`, default the origin) — widen the radius
  if shadows clip on a large scene, and move `shadow_center` onto the middle of a map that isn't
  centered at the origin.
- Point lights do not cast shadows: a lamp lights every surface facing it within `range`, even
  through a wall. Keep a room's lamp away from walls shared with rooms whose props face it.
- With zero lights, the scene is lit by `ambient` only (flat, cartoon-ish look).
- **The lighting model scales for you.** Point-light `intensity` is authored "hot" and the shader
  scales it down (`POINT_GAIN`), softens the hot spot under a lamp, wraps light slightly past the
  terminator and adds a small ambient floor (`AMBIENT_FLOOR`), then tone-maps with an ACES filmic
  curve at a fixed exposure. So the same numbers read the same in every map, interiors don't blow
  out to white, and ceilings/corners stay readable. Keep authoring ~10-14 per room lamp; light-coloured
  ceilings (`#d0d0c8`-ish) read better than dark roofs seen from below.
- That softening is distance-based, so it still blows out to solid white right next to the fixture: a lamp
  tuned to look right from across the room can overexpose if the player can walk up and stand beside it. Place
  a lamp slightly out of reach (on a wall, a high shelf, behind a counter), or check it with `frame`/`tour` from
  where the player will actually stand, not just the room's establishing shot.

## Objects

Every object shares these base fields:

```json
{ "id": "ball", "type": "sphere",
  "position": [0,1,0], "rotation": [0,0,0], "scale": 1,
  "material": { "color": "#e05252", "metallic": 0.0, "roughness": 0.5, "emissive": "#000000" } }
```

`scale` may be a single number (uniform) or `[x,y,z]`. `material.metallic`/`roughness` are
0–1 (unset defaults: `metallic=0`, `roughness=0.6`; a value outside the range is a validation error, and a `roughness` below 0.04 draws as 0.04, the smoothest the shading can show). `emissive` is a hex color added on top,
unaffected by lighting (glow); default `#000000` (none). `opacity` (0–1, default 1) makes a surface see-through: glass, water, a
flame's glow. Blended objects are drawn after all solid ones, far to near, cast no shadow and do not write depth (so they are
not outlined by the clarity pass); a `plane` with `opacity` is a water sheet, a thin `box` a pane. Sorting is per object: two
crossing blended objects can show a seam, so keep them apart.

Two more base fields work on every object: `"collide": false` makes it (and, for a group, everything
inside) walk-through — no player collider, not standable, no solid volume for `lint` — and
`"lint_ignore": ["code", ...]` silences specific lint codes on it.

A third, `"movable": true|false`, only matters in the live game (`re2`): by default a `prop` or a
floor-mounted prefab that a person could lift (longest side <= 1.25 m, bounding-box volume <=
0.45 m^3, and not a fixture like a toilet, tree, rug, sofa or fridge) is a **loose prop** — pick it
up with **E**, drop it, knock it over, shove it. `false` pins an object in place (a chair that is
part of the set); `true` frees something the rules left fixed. Keyframed objects are never loose.
`lint`, `reach`, `walk` and `plan` still treat loose props as solid furniture where you put them.

### Primitives

| `type`     | extra fields                                  |
|------------|------------------------------------------------|
| `box`      | `size: [w,h,d]` (default `[1,1,1]`)             |
| `sphere`   | `radius` (default `0.5`)                        |
| `cylinder` | `radius`, `height` (defaults `0.5`, `1`)         |
| `cone`     | `radius`, `height` (defaults `0.5`, `1`)         |
| `capsule`  | `radius`, `height` (defaults `0.3`, `1`)         |
| `plane`    | `size: [w,d]` (default `[10,10]`), always faces `+Y` |

### `wall` (macro)

A straight wall with doors/windows, authored as one object. It expands at parse time into a
`group` of plain `box` pieces (so collision, rendering and every tool just see boxes) — the
arithmetic of splitting a wall around an opening is done for you, correctly.

```json
{ "id": "wall_front", "type": "wall", "from": [-7, 0], "to": [7, 0],
  "y": 0, "height": 2.8, "thickness": 0.24,
  "material": { "color": "#d9ccb0", "roughness": 0.85 },
  "trim": "#f4f1ea", "baseboard": "#efeae0",
  "openings": [
    { "at": 2.75, "width": 2.0, "kind": "window" },
    { "at": 7.0,  "width": 1.2, "kind": "door" },
    { "at": 11.25, "width": 2.0, "kind": "arch" }
  ] }
```

- `from`/`to` — the wall's **centerline** endpoints as `[x, z]`. `y` — the floor height it stands
  on (default 0); `height` (default 2.7); `thickness` (default 0.2).
- `extend` (default `true`) — grow each end by half the thickness so walls meeting at a corner
  overlap into a clean, flush corner instead of leaving a notch.
- Each opening's `at` is the distance **along the wall from `from`** to the opening's *center*.
  `kind` is `door` (default; height 2.2, sill 0), `window` (height 1.2, sill 0.9, gets a glass
  pane unless `"glass": false`), or `arch` (a wide doorless opening, ~85% of wall height).
  Override `width`, `height`, `sill` per opening. A `door` must be at least **2.05 m** tall — the
  player's body band is 2.0 m, so a lower header blocks the doorway (validation refuses it).
- `trim` (hex) frames every opening; `baseboard` (hex, or `{"color","height"}`) runs a low strip
  along the wall base. Both give rooms visible edges — a real clarity aid.
- Errors are reported as `wall_id.openings[i].field: message` (opening outside the wall,
  overlapping openings, sill+height above the wall top, ...).
- To close the wall around a stairwell or make a knee-high railing, use a low `height` (e.g.
  `1.05`) and small `thickness` (`0.08`): it blocks the player at the floor it stands on.

Wall pieces are addressed by the wall's own id in every tool (`ls`, `move`, `rm`, ...);
individual pieces are `wall_id.seg0`, `wall_id.head1`, ... and show up in `ls --all`.

### `fence` (macro)

A run of fence along a polyline, expanded to posts + panels/rails.

```json
{ "id": "fence_perimeter", "type": "fence", "closed": true,
  "points": [[-12, -10], [12, -10], [12, 26], [-12, 26]],
  "height": 2.0, "style": "panel", "post_spacing": 2.0,
  "material": { "color": "#b8a07a" }, "post_color": "#7d6a4b",
  "gaps": [ { "at": [0, -10], "width": 2.4 } ] }
```

- `points` — `[x, z]` vertices; `closed: true` joins the last back to the first.
- `style`: `panel` (solid boards between posts, default) or `rail` (open post-and-rail).
- `gaps` — cut a gate: a gap of `width` centered at the world point `at` on whichever segment
  passes nearest. Leaving a gap in a perimeter lets the player walk out of the map (`lint` flags
  it as a `leak`).

### `text` (macro)

Lettering from the engine's 5x7 font, merged into the fewest boxes, with generated ids (`sign.l0_5`). A sign no longer takes hundreds of hand-named boxes.

```json
{ "id": "sign_depot", "type": "text", "text": "DEPOT\nOPEN 24H", "position": [0, 2.2, -3.9], "height": 0.25,
  "backing": "#23303f", "material": { "color": "#ffcc66", "emissive": "#553300" } }
```

`text` is letters, digits and `!#%'()*+,-./:<=>?[]^_~` (lowercase shows as capitals; anything else is an error that lists what the font has), `\n` starts a line, at most 400
characters and 12 lines. `position` is the middle of the text block. `height` is the letters' height in metres (default 0.3; one font pixel is height/7); `depth` how far
they stand out (default one pixel); `align` `left|center|right` places shorter lines; `spacing` and `line_gap` are whole font pixels (1 and 3); `backing` is a board behind the
letters (a colour, or `{color, material, margin, thickness}`); `material` is the letters'. Lettering is decoration (`collide: false`) unless it says otherwise.

**Orientation:** the text reads along local +X and faces +Z, so a viewer looking along -Z (yaw 0) reads it normally. `rotation: [0, yaw, 0]` turns it: yaw 0 faces +Z, 90 faces +X,
180 faces -Z, -90 faces -X. A sign on the west face of a wall at x = 4 is `"rotation": [0, -90, 0]` at x just below the wall's face.

### `array` (macro)

Copies of one template, named `<id>.0`, `<id>.1`, ... (the engine requires every object to have a unique id; this makes them).

```json
{ "id": "posts", "type": "array", "count": 6, "step": [1.5, 0, 0],
  "template": { "type": "box", "size": [0.1, 1.1, 0.1], "position": [0, 0.55, 0], "material": { "color": "#8a6a40" } } }
```

Give `count` with `step`, or `positions` (a list of `[x, y, z]` offsets), up to 500 copies. `rotation_step` adds degrees per copy. The template has no `id`; its own `position` and `rotation`
are the first copy's. A group template's children (which need ids) are renamed per copy (`lamps.2.head.bulb`). `position` on the array places the whole thing.

### `stairs`

```json
{ "id": "stairs_main", "type": "stairs", "position": [-0.8, 0, 4.75],
  "width": 1.2, "run": 4.5, "rise": 3.0, "steps": 16,
  "material": { "color": "#8a5a34" } }
```

`width` and `run` are at least 0.1 m, `rise` at least 0.05 m and `steps` a whole number from 1 to 64; anything else is a validation error that says so (a staircase is never silently adjusted).
`position` is the **center of the footprint at the height of the bottom step**; local `+Z` is the
run axis (rotate about Y to point it elsewhere): the bottom is at local `-run/2`, the top (height
`rise`) at `+run/2`. It is a solid block of steps you climb from the *bottom end only*: the engine
adds solid side rails and a barrier across the tall end, so the player can't walk into the stair
volume from the side or the top. Rules for a working staircase (all checked by `lint`):

- the top end must land on a floor slab whose top is exactly `position.y + rise`, starting where
  the stairs end — and the slab must have a **hole** over the stairs, or you hit your head;
- keep clear floor beyond the bottom end and beyond the top end (a wall there = "leads nowhere");
- the opening left in the upper floor needs a railing (`wall` with `height: 1.05`) on its open
  sides, or the player can walk off it (`drop` warning);
- aim for step rise <= 0.20 and tread >= 0.26, `width` >= 1.1.

### `group`

Moves a set of child objects as one unit; children's transforms are relative to the group.

```json
{ "id": "signpost", "type": "group", "position": [2,0,0],
  "children": [
    { "id": "post", "type": "cylinder", "radius": 0.08, "height": 2, "position": [0,1,0],
      "material": {"color": "#8a6240"} },
    { "id": "sign", "type": "box", "size": [1,0.5,0.05], "position": [0,2,0.1],
      "material": {"color": "#f2f2f2"} }
  ] }
```

Children may nest further groups. A child's own `material` is required (groups have no
material of their own — they're pure transform containers).

### `humanoid`

A posable, ordinary-looking person built from capsules and squashed spheres — T-shirt (the object's
`material.color`), bare forearms and hands, jeans, shoes, neck, hair, eyes, brows, nose, mouth and
ears. Pose is forward-kinematic: each joint is a rotation *relative to its parent*.

```json
{ "id": "hero", "type": "humanoid",
  "position": [0,0,0], "rotation": [0,0,0],
  "height": 1.8, "build": 1.0,
  "material": { "color": "#e0b090" },
  "pose": {
    "spine": [0,0,0], "head": [0,0,0],
    "l_shoulder": [0,0,-10], "l_elbow": 15,
    "r_shoulder": [0,0,10], "r_elbow": 15,
    "l_hip": [0,0,-5], "l_knee": 10,
    "r_hip": [0,0,5], "r_knee": 10
  } }
```

- `height` is total standing height in world units (default `1.8`). `build` scales limb/torso
  thickness (default `1.0`).
- Optional `skin`, `hair`, `pants`, `shoes` hex colours recolour the rest of the figure (the shirt
  is `material.color`).
- `spine` and `head` are `[x,y,z]` degree tracks (bend/twist/tilt relative to their parent).
- `l_shoulder`/`r_shoulder`/`l_hip`/`r_hip` are `[x,y,z]` degree tracks (ball joints).
- `l_elbow`/`r_elbow`/`l_knee`/`r_knee` are single-number degree tracks (hinge joints — flexion
  only, always bends "forward").
- Every pose field is independently a track, so e.g. a walk cycle keyframes `l_hip`/`r_hip`/
  `l_knee`/`r_knee` out of phase while `spine` stays constant.
- The whole rig moves as a unit via the object's own `position`/`rotation`/`scale` (e.g. to
  walk the character across the scene, keyframe `position`, not the pose).

### `rat`

Cheddar, a small brownish-grey lab rat: pear-shaped body, pointed head with big round pink-lined
ears and whiskers, four scurrying legs with pink paws and a long tapering tail. Built into
`red_engine2` (see `src/characters.rs`), like `humanoid`. It is the rat player's body in `re2`;
in a scene it is decoration (or a cinematic extra).

```json
{ "id": "cheddar", "type": "rat", "position": [2,0,3], "rotation": [0,90,0],
  "material": { "color": "#7b6a5d" },
  "pose": { "gait": 0.7, "stride": 0.5, "sway": 1.0 } }
```

- Origin at the paws, nose toward local `+Z`; about 0.43 m nose-to-rump plus a 0.27 m tail and
  0.17 m tall at the back — knee-high to a chair. Use the object's `scale` to change size.
- `material.color` is the fur (default `#7b6a5d`); ears, nose, paws and tail keep their own pinks.
- `pose.gait` (radians) is the leg-cycle phase, `pose.stride` the amplitude from 0 (standing) to 1
  (flat-out scurry), `pose.sway` the idle phase of the tail/head. Each is a number or a track.
- It has no collider (decoration; the rat *player* collides as a circle of radius 0.12 m).

### `prop`

A prop-hunt prop: a handful of primitive parts (built into `red_engine2` itself, not authored
in JSON) placed as one object, the same "one keyword, prebuilt parts" idea as `humanoid`.

```json
{ "id": "crate_1", "type": "prop", "prop": "crate",
  "position": [2, 0, -1], "rotation": [0, 15, 0],
  "material": { "color": "#8a6a3f", "roughness": 0.8 } }
```

- `prop` (required) — one of the 39 kinds listed by `red_engine2 props` (with sizes):
  furniture (`sofa`, `bed`, `chair`, `armchair`, `dining_table`, `coffee_table`, `desk`,
  `nightstand`, `wardrobe`, `bookshelf`, `filing_cabinet`, `bench`, `rug`, `tv`), kitchen/bath
  (`kitchen_counter`, `refrigerator`, `stove`, `sink`, `toilet`, `bathtub`, `washer_dryer`),
  utility (`crate`, `barrel`, `box_stack`, `trash_can`, `traffic_cone`, `fire_extinguisher`,
  `vending_machine`, `mailbox`, `grill`, `picnic_table`, `fence_section`), and landscaping
  (`tree_oak`, `tree_pine`, `bush`, `flower_patch`, `hedge`, `boulder`, `potted_plant`).
- **Origin & facing.** A prop's origin is the middle of its **base**: `position.y` is simply the
  height of the surface it stands on. Props with a front (sofa, bed, tv, stove, sink, toilet,
  desk, wardrobe, chair, ...) face local `+Z`; `rotation: [0, 180, 0]` turns one to face `-Z`,
  `[0, 90, 0]` faces `+X`, `[0, -90, 0]` faces `-X`. `scale` (number or `[x,y,z]`) works too.
- **Color.** The instance `material.color` tints the body. Plants invert that: for `tree_*`,
  `bush`, `hedge`, `flower_patch` it *is* the foliage/bloom color (trunks, stems, soil are fixed).
- **Collision.** Normally one collider around the prop. Trees block only at the trunk; `bush`
  uses a tight box; `flower_patch` and `rug` don't block at all (walk-through).
- Like `humanoid`, a prop carries one `material` for its whole instance (not per-part) — a few
  parts get a small built-in metallic/roughness/color nudge off that base material (rim bands,
  foliage, ...) baked into the engine, not schema-configurable.
- Props block movement in the live viewer (one collider sized to the prop's overall footprint)
  and count as interactable (the crosshair can target one, same as any other object).

### `prefab`

A reusable, parametric object group authored in **JSON** — the way to add new furniture, food,
props-on-a-table and decor without writing Rust. `red_engine2 catalog` lists the ~155 built-in
prefabs (food, kitchen, furniture, office, school, store, decor, outdoor, art, lamps); `catalog <name>` shows one's
params and a paste-ready snippet.

```json
{ "id": "snack_1", "type": "prefab", "prefab": "apple_red",
  "position": [2, 0.78, 3], "rotation": [0, 30, 0], "params": { "color": "#8cc63f" } }
```

- An instance **expands at parse time into a plain `group`** (children ids become `snack_1.body`, ...),
  so every tool sees ordinary primitives. `position`/`rotation`/`scale` work like any object.
- `params` overrides the prefab's declared params (unknown names are errors with a "did you mean").
- For prefabs that declare a `color` param, `material: {"color":"#rrggbb"}` is a convenience alias for
  `params.color`. Supplying conflicting values is an error; other prefab material fields remain explicit params.
- **Origin & facing** follow `prop`s: origin = middle of the base (`position.y` = the surface it
  stands on; put an apple on a 0.78 m table at `y: 0.78`), front = local `+Z`. Prefabs with
  `"mount": "wall"` (pictures, clocks, blackboards, shelves) have their origin at the middle of the
  back face: put it on the wall's face and rotate so `+Z` points into the room.
- **Collision:** each `box` part blocks the player; spheres/cylinders/cones/capsules never do.
  Small prefabs (tag `small`) default to `"collide": false` (walk-through); override per instance.
  `lint` checks a floor-mounted prefab rests on something (`floating`/`sunk`) exactly like a `prop`.
- **Defining your own** (top-level `"prefabs"`, an object `{name: def}` or an array of defs with `name`;
  a scene-local def shadows a built-in of the same name):

```json
"prefabs": { "crate_stack": {
    "tags": ["storage"], "desc": "two crates",
    "params": { "gap": { "default": 0.02, "desc": "space between crates" }, "color": "#8a6a3f" },
    "objects": [
      { "id": "a", "type": "prop", "prop": "crate", "position": [0, 0, 0], "material": { "color": "$color" } },
      { "id": "b", "type": "prop", "prop": "crate", "position": [0, "=0.56+$gap", 0], "material": { "color": "$color" } } ] } }
```

  A string that is exactly `"$name"` is replaced by that param (any JSON type); a string starting
  `"="` is an arithmetic expression (`+ - * /`, parentheses, `min() max() abs()`, params as `$name`).
  `"extends": "other"` makes a variant that inherits objects/params/tags and overrides defaults
  (`apple_green` is `apple_red` with a different color). Prefabs may nest other prefabs.
- Built-in catalogue files live in focused packs under `assets/*.json` (embedded in the binary);
  `assets/packs.json` is their machine-readable registry. Every entry is test-enforced to expand,
  have tags + a description, and rest on its mount surface. `catalog --manifest` describes the
  versioned asset API and pack policy; `catalog --library game-assets.json <need>` searches a
  game-local pack together with the core before it is promoted.
- Optional definition `meta` is normalized into the catalog API: `aliases`, `roles`, `styles`,
  lifecycle `status` (`stable|experimental|deprecated`) and `revision`, `license`, creation
  `origin` (`authored|generated|modified|imported`), and free-text `provenance`. Old definitions
  default to stable revision 1, MIT, authored.

## Zones

Optional named regions that let the tools talk about rooms by name:

```json
"zones": [
  { "id": "kitchen", "rect": [1.6, 0.15, 6.85, 5.9], "y": 0 },
  { "id": "master",  "rect": [-6.85, 0.15, -1.6, 6.9], "y": 3.0 },
  { "id": "front_yard", "rect": [-11.8, -9.8, 11.8, -0.4], "y": 0, "kind": "outdoor" }
]
```

`rect` is `[x0, z0, x1, z1]`; `y` is the floor height (default 0). Zones don't affect rendering
or physics. `lint`/`reach` report whether each is reachable, `plan` labels them, `tour` renders a
view of each, and `scatter --zone <id>` plants inside one. Define one per room and per outdoor
area whenever you build a map.

## Clarity post-pass (`post`)

Every render (live and offline) ends with a depth-based pass that multiplies **contact ambient
occlusion** (soft shadow where objects meet walls/floors) and **silhouette outlines** (a dark
border on the near side of any depth jump) into the lit image, so a prop never melts into the wall
behind it. Tune per scene, all optional:

```json
"post": { "enabled": true, "ao": 0.9, "outline": 0.65, "ao_radius": 0.6 }
```

`ao` 0..3 (contact-shadow strength), `outline` 0..1 (border darkness), `ao_radius` meters.
Ambient light is also hemispherical (up-facing surfaces catch a little more than down-facing).
Map-color tip: keep props and the surface behind them at different *lightness* (a white fridge on
a cream wall is the hard case) — the post-pass helps but contrast is still the best fix.

## Game rules as data (`vars`, `rules`)

Gameplay is declared in the scene, not written in Rust. A rule fires **when** something happens, for **who**, **if** a
condition holds, and then **does** its actions:

```json
"vars": { "score": 0, "has_key": false },
"rules": [
  { "id": "take_coin_1", "when": { "enter": { "object": "coin_1", "pad": 0.3 } }, "once": true,
    "do": [ { "add": ["score", 1] }, { "hide": "coin_1" }, { "emit": "coin" } ] },
  { "id": "exit_opens", "when": { "enter": { "zone": "exit" } }, "if": "score >= 3 && !has_key",
    "do": [ { "emit": "victory" }, { "end": "victory" } ] },
  { "id": "trap", "when": { "enter": { "zone": "trap" } }, "who": "human", "cooldown": 2,
    "do": [ { "emit": "ouch" }, { "teleport": "spawn_a" } ] }
]
```

- **`when`** (exactly one): `{enter: VOLUME}`, `{exit: VOLUME}` (a player crosses the boundary), `{event: "name"}` (another
  rule `emit`ted it; chains are bounded to 4 per tick), `{every: secs}`, `{after: secs}`, `{start: true}`,
  `{prop_enter: VOLUME}` / `{prop_exit: VOLUME}` (a loose prop's origin crosses in or out; `"prop": id` beside it restricts
  the rule to one prop), `{prop_below: [prop_id, y]}` (its origin drops below `y` metres). A prop is *inside* when its origin
  is inside in x/z and its height band overlaps in y. Prop triggers fire with no triggering player (`teleport` does nothing).
- **VOLUME** (exactly one): `{zone: id [, height]}` (a `zones` rect from its floor `y` up 3 m, or `height`),
  `{object: id [, pad]}` (a top-level object's world box, grown by `pad` m — how a coin becomes a trigger), or
  `{box: [x0,y0,z0,x1,y1,z1]}`. A player is *inside* when its body circle overlaps the volume in x/z and its body height overlaps in y.
- **`who`**: `any` (default), `human`, `rat`, `team1`, `team2` (the last two need the scene's own `"teams": true`,
  or a `shooter` block, to put anyone on a team at all). **`once`**: at most once per match. **`cooldown`**: seconds between firings.
- **`player_vars`** (`"player_vars": {"laps": 0, "lives": 3}`, at most 8): variables every player has their own copy of, reset to the declared
  value when a player joins a slot. A rule reads and writes the copy of the player that triggered it as `me.name`: `"if": "me.laps >= 3"`,
  `{"add": ["me.laps", 1]}`, `{"set": ["me.lives", "me.lives - 1"]}`. The triggering player is the one who entered or left a volume, or
  who caused the event (`kill`, `pickup`, ...; an `emit` passes it on to the rules that react). `me.` in a `start`, `every`, `after` or prop
  trigger is a `validate` error (nobody triggered it). A scenario checks one player's copy with
  `{"player_var": "laps", "of": "runner", "gte": 2}`. Per-player variables are part of the match checksum and of replay; they are not
  yet sent to clients or saved (`persist`), and a native race still keeps its own laps (`race`).
- **`if`**: an expression over the `vars` and the built-ins `time` (s), `tick`, `players`: numbers, `true`/`false`,
  `+ - * / %`, `< <= > >= == !=`, `&& || !`, parentheses. `x / 0` is `0`. Built-in functions read the loose props:
  `prop_y(id)` (origin height, m), `tilt(id)` (degrees from how the map placed it: 0 upright, ~90 on its side), `held(id)`
  (1 while carried), `mass(id)` (kg), `moved(id)` (metres from the authored spot), `props_in(zone)` (loose props inside a
  zone), `in_zone(id, zone)` (1 when that one prop is inside the zone: a weighed pan is `in_zone(a, pan) * mass(a) + ...`).
  The argument is a bare id; a prop that is not loose (`movable: false`, a fixture) is a validate error.
- **`do`** (in order): `{set: [var, value]}`, `{add: [var, n]}` (value/n is a number, bool or expression string), `{emit: name}`,
  `{hide: id}` / `{show: id}` (the standard single-player client omits that object tree from rendering),
  `{collision: [top_level_id, bool]}` (enable/disable its authored static collision and standable surfaces **for players**: loose props keep colliding with the object, and a prop resting on it stays put: it is not woken),
  `{deactivate: top_level_id}` / `{activate: top_level_id}` (`hide` plus `collision: false`, and `show` plus `collision: true`, in one step: an opened door),
  `{teleport: [x,y,z] | spawn_id}` (the triggering player),
  `{end: outcome}` (the match ends; rules stop), `{impulse: {object, dir: [x,y,z], speed}}` (shove a loose prop),
  `{reset: id | [ids] | {zone: id}}` (put loose props back where the map placed them, at rest; a carried one is taken from its
  holder), `{place: [id, [x,y,z]]}` (move a loose prop's origin to a point, upright as authored, at rest).
- **Engine events** a rule can react to with `{event: name}`: `pickup`, `drop`, `shot`, `hit`, `kill`, `respawn`, `swing` (a bat
  swing started), `prop_hit` (a bat or a bullet struck a loose prop; the player is the striker).
- **Edges, measured**: rules run in declaration order each tick, but a body walking from one volume into the next overlaps both for a
  moment, so the next volume's `enter` fires before the previous volume's `exit`; an `exit` rule that resets a variable undoes the
  `enter` rule's work (key on `enter`, or use one variable per volume). A player or prop that starts inside a volume gets no `enter`
  (use a `start` rule). A variable whose name starts with `_` is internal: the generic HUD does not show it.

Everything a rule names — variables, objects, loose props, zones, spawn points, events — is checked when the scene loads, with a
did-you-mean (`rules[1] (exit_opens).if: unknown variable `scor` — did you mean `score`?`). Rules run inside the
authoritative simulation (`MatchSim`: `red_server`, `red_engine2 sim`), deterministically, and their state (prop occupancy
included) is part of the match checksum. Offline `re2` runs the same `RulesEngine`, applies hide/show, teleport, impulse, reset and place, feeds the engine
events into the rules, and shows scene-defined variables, recent events and the terminal outcome in a generic HUD. Online rule
state remains server-authoritative; protocol v7 and later repeatedly send the complete bounded presentation state (16 variables, 256 hidden
objects, 64 collision-disabled objects, recent event and outcome), so loss, reconnect and late join recover it. `red_engine2 describe rules` prints this with a
runnable example; `recipe coin_run` is a complete game.

### Proving gameplay headless (`checks.sim`, `sim`)

`red_engine2 sim scene.json` plays **scenarios** — scripted players walking through the real simulation at 60 Hz — and
checks the outcome. They live in `checks.sim` (so `verify` runs them) or a file (`--scenario`). **`bots.fill`/`roster`
are not part of this**: a scenario's `players` are the only participants simulated. To prove what a bot (an AI
opponent, a monster) actually does, use `playtest` (which hosts a real match with bots) or a real hosted match.

```json
"checks": { "sim": [ { "name": "collect all three coins, then win",
  "players": [ { "id": "p1", "character": "human", "spawn": "spawn_a" } ],
  "script": [ { "player": "p1", "walk": "-5,-3; 0,3; 5,-2; 8.8,0" } ],
  "expect": [ { "event": "coin", "count": 3 }, { "var": "has_key", "eq": true }, { "ended": "victory" },
              { "hidden": "coin_1" }, { "no_event": "ouch" }, { "player": "p1", "near": [8.8, 0], "tol": 0.8 } ] } ] }
```

Script steps per player run in order (players in parallel): `walk "x,z; x,z"` (steered with the real movement; a walk that gets stuck
fails the scenario), `wait secs`, `hold {forward, strafe, sprint, crouch, jump, yaw_deg, pitch_deg, look_at: [x,y,z], interact, attack, reload, switch,
seconds}` (`look_at` aims at a world point from the eye every tick; buttons act on the tick they go down: `interact: true` picks up / drops what the view points at within 2.3 m, `attack: true`
swings the bat, its strike landing after the windup), each with optional `until_event: name`. The run ends when a rule ends the match,
when all scripts finish (plus `settle_seconds`, default 0.5), or at `max_seconds` (default 30). No window, GPU or socket is involved.

Expectations on loose props: `{prop: id, in_zone: zone}` / `not_in_zone`, `below_y` (= `y_lt`) / `y_gt: metres`, `tilt_gt` / `tilt_lt:
degrees`, `moved: true|false`, `near: [x, z], tol?, y?`, `held_by: player_id | "none"`. The report prints where every player ended
and every prop that moved, tilted or is carried; `--json` lists every loose prop with its position, tilt, distance moved, whether it is
at rest and who holds it.

### Force fields (`fields`)

A river current, a conveyor belt or a wind tunnel is one entry, not a rule per prop: `"fields": [{"id": "current", "zone": "channel",
"velocity": [0, 1.5], "rate": 10}]`. The volume takes the same keys as a rule volume (`zone` [+ `height`], `object` [+ `pad`] or `box`).
Every tick each loose prop whose origin is inside (and that nobody is carrying) is pulled toward the target: `velocity: [x, z]` m/s
along the ground axes, `lift: m/s` upward (a prop slower than that is pulled up to it; use it for wind tunnels and hover pads), at
`rate` per second (default 10, at most 60). It is a drag toward a *speed*, so unlike a repeating `impulse` rule nothing accelerates
past the target; gravity, floors, walls and other props still act, and floor friction (about 7 m/s^2) makes a prop on the ground
settle a little under the target (raise `rate` for a stronger belt). A prop that has never been disturbed is promoted the first
tick it is inside. Players are not pushed. The push is derived from state each tick (like a rule `impulse`), so it is deterministic,
not recorded in a trace, and replays identically.

### Traces and replay (`sim --trace`, `replay`, `red_server --record`)

A **trace** is a recording of a match: header (engine version, tick rate, map hash, seed, platform), every join / leave / input /
external push (a server kick; a strike or a rule `impulse` is re-derived by the replay, not recorded) in order, the game events, a checksum of *players*, *props* and *rules* every N ticks, and periodic state dumps.
`red_engine2 replay trace.json` re-runs it with no renderer or socket and reports the **first divergent tick**, which
component differs, and a compact state diff (`--dump-every 1` when recording gives it at the exact tick); `--against other.json`
compares two traces of the same match (a desync between two machines). Exact checksums are bit-for-bit on the same platform;
the simulation's maths uses `libm`, so Windows and Linux are expected to agree, and a millimetre-quantised `coarse` checksum tells
float noise from a real divergence when they do not. See ADR 0021.

## Races (`race`)

A kart race (Great Outdoors; ADR 2026-09-29-great-outdoors-karts-as-a-first-class-engine-feature) is a scene zone list plus a `race` block. Per-player laps and
positions cannot be written as rules (a rule variable is one number for everybody), so the simulation tracks them natively (`sim::race`).

```json
"zones": [{"id": "line", "rect": [-6, -1, 6, 1]}, {"id": "bend", "rect": [30, 40, 40, 44]}, {"id": "back", "rect": [-6, 79, 6, 81]}],
"race": {"laps": 3, "gates": ["line", "bend", "back"], "countdown_secs": 3, "finish_grace_secs": 30}
```

- **`gates`**: zone ids across the track, in driving order, at least 3 and each listed once; the first is the start/finish line. The grid sits just *before* the
  line: everyone starts with gate 1 as their next gate, so crossing the line at the start earns nothing, and a lap is gates 1..N-1 and then the line again.
- **Direction**: a gate faces from the previous gate's centre towards the next one's (so it holds on bends). Only a crossing in that direction counts, a fast
  kart cannot step over a thin gate (the test is the segment from last tick's position), and a skipped gate is still owed.
- **`laps`** 1..20 (default 3). **`countdown_secs`** 0..600 (default 3): karts are held until the light. **`finish_grace_secs`** (default 30): once the first
  kart finishes, the others have this long before the race ends and they are ranked as they stand.
- **`line`** (optional): `[[x, z], ...]` racing-line points for bots on tight corners; empty = the gate centres.
- **`item_boxes`** (optional): zone ids where an item box sits, and **`item_respawn_secs`** (default 5): the first kart to touch a ready box with a free hand
  gets a **Mushroom** (a 1.5 s speed burst), an **Acorn** (thrown ahead; spins out the first kart it hits, and shatters on a wall) or a **Bubble** (a shield that
  absorbs the next hit). `attack` uses the item on the press. Which item is rolled from the tick, slot, box and place alone (so a replay draws the same items),
  weighted towards Mushrooms for the tail and Bubbles for the leader. The Beaver's `interact` lays a **plank** behind the kart (4 s cooldown; a full pool of 24
  hazards replaces the oldest plank). A hazard never hits its owner in its first 0.75 s.
- **`surfaces`** (optional): `[{"zone": id, "kind": "dirt" | "mud" | "water"}, ...]` marks patches of ground that are not road. A kart's top speed is scaled by
  its driver's multiplier for the surface under it at the start of the tick (mud 0.6 and water 0.55 for most animals, dirt 0.85; the Duck floats over water, the Beaver's
  wooden kart ignores mud and water and dirt, the Coyote ignores dirt). Later patches win where they overlap. The server and every client's prediction use the same
  lookup (`RaceCourse::surface_at`), so a predicted kart stays exact across a patch's edge. Draw the patch yourself (a plane over the zone): the engine only changes the physics.
- **Bots** (`bots` block): in a race scene `bots.fill` tops the grid up with kart bots (`skill` is their level, as for fighters). A bot is an ordinary player
  slot driven by a brain that follows the racing line (`race.line`, else the gate centres), sets its speed from the corner ahead and its skill, steers on the
  analog stick, drifts long corners once good enough, uses its pickup (Mushroom on a straight, Acorn at a kart ahead in its lane, Bubble when an Acorn is
  coming), dodges planks, and backs out of a jam. Each bot takes an animal nobody else has. Put `line` points on tight corners and make sure the first is on
  the start line.
- **Standings**: finishers by finish time, then by gates passed in total, then by distance to the next gate, then by slot. A player who joined and left is
  ranked as not finished; a slot that never joined is not listed. State is part of the match checksum.


**A hosted race and its lobby.** Add a `match` block (`"match": {"min_players": 1, "countdown_secs": 1, "join_in_progress": false}`) and the server runs the usual
lobby -> countdown -> round -> results loop around the race: in a race the lobby's character byte is the animal (`Driver::wire`, 0..7), so the lobby screen
shows an animal picker instead of a body switch (arrow keys, A/D, the d-pad or the bumpers; animals other people hold are skipped), and everyone still in the
lobby readies up. Two people who chose the same animal: the lower slot keeps it and the other gets the first free one when the round is built. The round ends
when the race is over (everyone finished, or `finish_grace_secs` after the first finisher); the winner is first place in the standings; then the results,
then the lobby again and a fresh world for the next race. The flow's own countdown runs first (keep it short: the race has its own).

## Physics rules a map author must know

- **Releasing a carried prop** gives it your own horizontal and vertical velocity plus `player.throw_speed` (default 1 m/s) along your look,
  identical offline and online, and your body ignores it for 10 ticks. Measured on `tests/fixtures/throw.json` (crate, walk 4.2 / sprint 8 m/s):
  a standing release is a short toss (about 0.2 m of travel), walk-and-stop lands it about 1.7 m past the release point, sprint-and-stop about 4 m;
  looking up 45 degrees with `throw_speed` 6 clears a 1.2 m ledge 2 m away. A body that keeps running into the landed prop still pushes it.

The live viewer's player is a 0.35 m-radius circle, 2.0 m tall, that walks at 3.2 m/s:

- **Walls block** anything whose top is more than **0.35 m above the player's feet** and whose
  bottom is below head height; anything lower is *stepped onto* (that is how stairs meet a slab).
- **Doorways** must be >= 0.9 m wide (0.7 m is the bare minimum) and **>= 2.05 m tall**.
- **Upper floors**: a floor slab is an ordinary `box` (0.2 thick works). Walls on an upper floor
  need their own objects at that floor's `y` — they don't inherit from walls below.
- The player can jump ~0.4 m (onto low props), can't crouch under things, and falls off any edge.
- Point lights don't cast shadows; only one directional light does.

`red_engine2 lint`, `reach`, `walk` and `plan` check all of this — see `AGENTS.md`.

## Checks (`verify`)

A scene can carry its own expectations in a top-level `"checks"` block; `red_engine2 verify scene.json`
runs them all with the real engine code and prints PASS/FAIL with evidence (exit 1 on any failure):

```json
"checks": {
  "lint":    { "max_errors": 0, "max_warnings": 3, "forbid": ["leak"] },
  "reach":   [ { "to": [3, -1.5], "why": "kitchen reachable" } ],
  "walk":    [ { "name": "front door to bedroom", "path": "0,8; 1.5,4; -3.25,-2.9; -3.25,2.9",
                 "ends_near": [0.5, -0.5], "tol": 0.35, "floor_y": 3.0 },
                { "name": "kitchen to the stairs", "from": [3, 2], "to": [-1, 5], "auto": true } ],
  "objects": { "exist": ["sofa_1"], "absent": ["debug_cube"], "min_count": 30,
               "count": [ { "kind": "prefab:chair_wooden_1", "min": 2 } ] },
  "views":   [ { "name": "living", "eye": [-4.4, 1.7, 2.4], "at": [-2.4, 0.7, -1.8], "fov": 75, "max_diff": 0.01 } ]
}
```

`walk` replays the route with the per-tick player physics (see [Physics rules](#physics-rules-a-map-author-must-know)), using the scene's
`player` tuning and `jump_pads` (a pad on the route launches the walker like it launches a player). Without `from` a walk starts at the first
`spawns` entry, on that spawn's floor (a spawn on a 3 m deck starts on the deck); `from: [x, z]` starts elsewhere on the ground floor and
`from_y` picks the floor there, for `reach` entries too. Lint and `reach` seed their flood-fill the same way, and a top-level object that an
unconditional `start` rule switches collision off for is open to every tool, as it is to the game from its first tick. An entry with
`"to": [x, z]` and `"auto": true` (no `path`) plans its own route on every run and prints it (`from_y` / `to_y` pick floors); a failing entry
names the object that blocked it and writes `out/verify/<scene>_walk<N>_explain.png`;
`views` are golden-image regression tests (`golden/<scene>/<name>.png` beside the scene; recorded on
first run or with `--bless`; on failure a `golden | now | diff` image is written under `out/verify/`).
`verify --no-views` skips rendering (no GPU), `--only walk` / `--only walk[2]` / `--only "front door"` runs a subset (every check is timed); add the global `--json` for the machine-readable envelope.
`checks.lint` takes `ignore: [codes]` for findings a map accepts everywhere (a pit's `drop` edges) beside `forbid` and the budgets.

**Level states (`phases`).** The tools see the level as it is before any rule has fired, so a gate that a rule opens later is a wall to them. A top-level
`"phases": { "gate_open": ["open_gate"] }` names a state by the rules assumed to have fired (applied in order after the unconditional `start` rules;
their `deactivate` / `collision` / `activate` effects count, nothing else). A `checks.reach` or `checks.walk` entry takes `"phase": "gate_open"`;
a `reach` entry also takes `"reachable": false` to assert a place is still cut off in that state (`initial`, the implicit first state, is what an
entry without `phase` checks). `lint` (in `verify` and on the command line) also runs every declared phase: a `zone`, `floor` or `unreachable`
finding that some phase resolves is dropped, and a finding that exists only in a phase (a leak the open gate creates) is reported as `[phase name] ...`.
`reach`, `lint`, `walk` and `plan` take `--phase NAME` to look at one state by hand. Phases are for the analysis tools; they change nothing in the game,
so keep a `checks.sim` scenario that really opens the gate (`recipe gated_garden`).
`checks.sim` holds headless gameplay scenarios (see [Game rules as data](#game-rules-as-data-vars-rules)).

## Blueprints (`red_engine2 build`)

A blueprint is the short way to make a whole map: rooms, doors, spawns and prop fill in about twenty lines, compiled into a complete scene
whose `checks` already pass (`red_engine2 build --example` prints a working one; `describe` lists the command). It is a separate JSON
document with `"blueprint": 1`; unknown keys are errors with a did-you-mean. Coordinates are metres, `[x0, z0, x1, z1]` rectangles, +Y up.

| Key | Meaning |
|---|---|
| `blueprint` | Format version, `1`. |
| `name` | Map name (used in the summary and the `x-blueprint` note). |
| `height` | Wall height, default `2.8` (2.3 to 8). |
| `ceiling` | `true` adds a slab over every room (default `false`: open top, lit by the sun and lamps). |
| `rooms` | List of rooms; each has an `id` (letters, digits, `_`, `-`), a `rect` (`[x0, z0, x1, z1]`), an optional `floor` colour (hex) and an optional `lamp` (`false` skips the room's lights; at most 255 generated lamps in total). Rects may touch along an edge but not overlap. |
| `doors` | List of openings; each names the two rooms it joins in `between` (`["hall", "store"]`, they must share a wall), and may set `width` (default 1.4, at least 0.9), `at` (offset from the middle of the shared wall) and `kind` (`door` or `arch`). Doors also become `portals` (interest management for the server). |
| `spawns` | List of spawn requests; each has a `room`, an optional `group` (`red_server --spawn-group`), a `count` of points spread around the room facing its centre (a lone spawn faces the first door) and an optional `id` prefix. |
| `fill` | List of prop fills; each has a `room`, a `kind` (a prop name or a list; see `props`), a `count`, a `seed`, a `scale` range (`[0.9, 1.15]`), `colors`, `min_gap`, `clearance` and an `id` prefix. Placed by `scatter` while keeping door pads, aisles between doors, spawn pads and `keep_clear` free. |
| `keep_clear` | Extra `[x0, z0, x1, z1]` rectangles fill must leave empty. |
| `extra` | Raw scene objects appended verbatim (prefabs, stairs, anything the blueprint cannot say). |
| `prefab_files` | Paths (relative to the blueprint) of prefab libraries in the `assets/*.json` format, merged into the built scene's `prefabs`. A game ships its own props this way without touching the engine (place instances with `extra`); the map stays self-contained for the server. |
| `scene` | Raw top-level scene keys merged into the result (`vars`, `rules`, `weapons`, `checks.sim`, ...). A `checks` object merges key by key into the generated one and `zones` merge by id (a zone with a generated room's id replaces it, a new id is added, so the generated portals keep their rooms); every other key, `spawns` and `camera` included, **replaces** the generated value. |

What comes out: a `floor_<room>` plane per room; `wall_ext_N` (0.24 thick) around free edges and `part_N` (0.15) on shared ones, with the
door openings; a `sun` and one lamp per 8 x 8 m of room; `zones`, `spawns`, `portals` and `interest`; a camera at the first spawn; and `checks`:
lint (0 errors, the warnings found at build time), one `reach` per room, one `auto` walk from the first spawn to every other room, an object
count. The output is deterministic, so `red_engine2 build bp.json --check` fails (exit 1) when the committed map differs from what the blueprint
builds. A game project (`red_engine2 new-game`, `game check`) wires blueprints, maps and the server together; every project also exposes
`game play-local` for direct single-player testing without starting or joining a server. Online support is additive, never a replacement
for local play. See ADR 0024.

## Validation

`red_engine2 validate scene.json` checks the file before any render time and reports errors as
`object_id.field: message` (or `camera.field` / `lights[i].field`) — same precise-pointer
convention as the 2D engine. Common failures: unknown `type`, missing required field for that
type, keyframe `t` values not sorted ascending, more than one shadow-casting light, unknown
`ease` name, malformed hex color, an opening outside its `wall`, a `door` shorter than 2.05 m.
Beyond schema validation, `red_engine2 lint scene.json` checks a *walkable map* for layout
problems (stairs that lead nowhere, unreachable rooms, overlaps, ...).

## Native controllers and sandbox inspection

The graphics client polls native gamepads through gilrs. Left-stick movement retains analog
strength; right-stick look is time-based and FOV-compensated. Input flag bit 7 selects signed
-127..127 axes; digital -1/0/1 inputs retain their old meaning. Network protocol is v<!--fact:protocol-->15<!--/fact-->.
Headless builds do not pull in gilrs. Focus loss, disconnect and menu transitions require
neutral controls before gameplay resumes. See docs/CONTROLLERS_AND_SANDBOX.md for bindings.

Character values include human, rat, wizard, cowboy, alien and robot. All except rat use
the human body and weapon logic; rules with who:human include those human-rig characters.
A humanoid object may set style to human, wizard, cowboy, alien or robot. Costume parts follow
the existing rig and preserve the twelve core bone indices.

For local projects, M / D-pad up opens the declared game.json map list. Loading a map resets
its simulation and retains the current character unless the destination has a character policy.
Maps outside the containing project are not listed. Online sessions do not permit local map travel.

## Known limits (intentional)

No imported meshes/textures, no scene-object physics in the *offline* tools (nothing falls, bounces,
or collides on its own there; in the live viewer the player has its own simple gravity/collision
model, and small props are rigid bodies you can pick up, drop and knock over — see `movable` above), no per-vertex mesh deformation/skinning beyond the fixed
capsule-rig `humanoid`, no 2-D text in the *offline* renderer (composite with the 2D engine for captions; the live game's menus, lobby and HUD use the `src/ui` kit), at most 256 authored lights, 16 active lights per view, and 1 shadow-casting light. The
goal is a small, auditable surface an AI can hold in context, not a general-purpose 3D suite.
