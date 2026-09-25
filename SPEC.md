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
  "post": { "ao": 0.9, "outline": 0.65 },
  "lights": [ ... ],
  "zones": [ ... ],
  "objects": [ ... ]
}
```

`post` and `zones` are optional (see [Clarity post-pass](#clarity-post-pass-post) and
[Zones](#zones)). Other optional top-level keys: `schema_version`, `recipe`, `spawns`, `portals`, `interest`,
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

## Lights

Up to 16 lights. Each has a `type` of `"directional"` or `"point"`.

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

## Objects

Every object shares these base fields:

```json
{ "id": "ball", "type": "sphere",
  "position": [0,1,0], "rotation": [0,0,0], "scale": 1,
  "material": { "color": "#e05252", "metallic": 0.0, "roughness": 0.5, "emissive": "#000000" } }
```

`scale` may be a single number (uniform) or `[x,y,z]`. `material.metallic`/`roughness` are
0–1 (unset defaults: `metallic=0`, `roughness=0.6`). `emissive` is a hex color added on top,
unaffected by lighting (glow); default `#000000` (none).

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

### `stairs`

```json
{ "id": "stairs_main", "type": "stairs", "position": [-0.8, 0, 4.75],
  "width": 1.2, "run": 4.5, "rise": 3.0, "steps": 16,
  "material": { "color": "#8a5a34" } }
```

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
- Built-in catalogue files live in `assets/*.json` (embedded in the binary); every entry is
  test-enforced to expand, have tags + a description, and rest on its mount surface.

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
  rule `emit`ted it; chains are bounded to 4 per tick), `{every: secs}`, `{after: secs}`, `{start: true}`.
- **VOLUME** (exactly one): `{zone: id [, height]}` (a `zones` rect from its floor `y` up 3 m, or `height`),
  `{object: id [, pad]}` (a top-level object's world box, grown by `pad` m — how a coin becomes a trigger), or
  `{box: [x0,y0,z0,x1,y1,z1]}`. A player is *inside* when its body circle overlaps the volume in x/z and its body height overlaps in y.
- **`who`**: `any` (default), `human`, `rat`. **`once`**: at most once per match. **`cooldown`**: seconds between firings.
- **`if`**: an expression over the `vars` and the built-ins `time` (s), `tick`, `players`: numbers, `true`/`false`,
  `+ - * / %`, `< <= > >= == !=`, `&& || !`, parentheses. `x / 0` is `0`.
- **`do`** (in order): `{set: [var, value]}`, `{add: [var, n]}` (value/n is a number, bool or expression string), `{emit: name}`,
  `{hide: id}` / `{show: id}` (state a renderer or client acts on), `{teleport: [x,y,z] | spawn_id}` (the triggering player),
  `{end: outcome}` (the match ends; rules stop), `{impulse: {object, dir: [x,y,z], speed}}` (shove a loose prop).

Everything a rule names — variables, objects, zones, spawn points, events — is checked when the scene loads, with a
did-you-mean (`rules[1] (exit_opens).if: unknown variable `scor` — did you mean `score`?`). Rules run inside the
authoritative simulation (`MatchSim`: `red_server`, `red_engine2 sim`), deterministically, and their state is part of the
match checksum. `red_engine2 describe rules` prints this with a runnable example; `recipe coin_run` is a complete game.

### Proving gameplay headless (`checks.sim`, `sim`)

`red_engine2 sim scene.json` plays **scenarios** — scripted players walking through the real simulation at 60 Hz — and
checks the outcome. They live in `checks.sim` (so `verify` runs them) or a file (`--scenario`):

```json
"checks": { "sim": [ { "name": "collect all three coins, then win",
  "players": [ { "id": "p1", "character": "human", "spawn": "spawn_a" } ],
  "script": [ { "player": "p1", "walk": "-5,-3; 0,3; 5,-2; 8.8,0" } ],
  "expect": [ { "event": "coin", "count": 3 }, { "var": "score", "eq": 3 }, { "ended": "victory" },
              { "hidden": "coin_1" }, { "no_event": "ouch" }, { "player": "p1", "near": [8.8, 0], "tol": 0.8 } ] } ] }
```

Script steps per player run in order (players in parallel): `walk "x,z; x,z"` (steered with the real movement; a walk that gets stuck
fails the scenario), `wait secs`, `hold {forward, strafe, sprint, crouch, jump, yaw_deg, seconds}`, each with optional
`until_event: name`. The run ends when a rule ends the match, when all scripts finish (plus `settle_seconds`, default 0.5), or at
`max_seconds` (default 30). No window, GPU or socket is involved.

### Traces and replay (`sim --trace`, `replay`, `red_server --record`)

A **trace** is a recording of a match: header (engine version, tick rate, map hash, seed, platform), every join / leave / input /
server impulse in order, the game events, a checksum of *players*, *props* and *rules* every N ticks, and periodic state dumps.
`red_engine2 replay trace.json` re-runs it with no renderer or socket and reports the **first divergent tick**, which
component differs, and a compact state diff (`--dump-every 1` when recording gives it at the exact tick); `--against other.json`
compares two traces of the same match (a desync between two machines). Exact checksums are bit-for-bit on the same platform;
the simulation's maths uses `libm`, so Windows and Linux are expected to agree, and a millimetre-quantised `coarse` checksum tells
float noise from a real divergence when they do not. See ADR 0021.

## Physics rules a map author must know

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

`walk` replays the route with the per-tick player physics (see [Physics rules](#physics-rules-a-map-author-must-know)). An entry with
`"to": [x, z]` and `"auto": true` (no `path`) plans its own route on every run and prints it (`from_y` / `to_y` pick floors); a failing entry
names the object that blocked it and writes `out/verify/<scene>_walk<N>_explain.png`;
`views` are golden-image regression tests (`golden/<scene>/<name>.png` beside the scene; recorded on
first run or with `--bless`; on failure a `golden | now | diff` image is written under `out/verify/`).
`verify --no-views` skips rendering (no GPU), `--only walk` / `--only walk[2]` / `--only "front door"` runs a subset (every check is timed); add the global `--json` for the machine-readable envelope.
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
| `rooms` | List of rooms; each has an `id` (letters, digits, `_`, `-`), a `rect` (`[x0, z0, x1, z1]`), an optional `floor` colour (hex) and an optional `lamp` (`false` skips the room's lights; at most 15 lamps in total). Rects may touch along an edge but not overlap. |
| `doors` | List of openings; each names the two rooms it joins in `between` (`["hall", "store"]`, they must share a wall), and may set `width` (default 1.4, at least 0.9), `at` (offset from the middle of the shared wall) and `kind` (`door` or `arch`). Doors also become `portals` (interest management for the server). |
| `spawns` | List of spawn requests; each has a `room`, an optional `group` (`red_server --spawn-group`), a `count` of points spread around the room facing its centre (a lone spawn faces the first door) and an optional `id` prefix. |
| `fill` | List of prop fills; each has a `room`, a `kind` (a prop name or a list; see `props`), a `count`, a `seed`, a `scale` range (`[0.9, 1.15]`), `colors`, `min_gap`, `clearance` and an `id` prefix. Placed by `scatter` while keeping door pads, aisles between doors, spawn pads and `keep_clear` free. |
| `keep_clear` | Extra `[x0, z0, x1, z1]` rectangles fill must leave empty. |
| `extra` | Raw scene objects appended verbatim (prefabs, stairs, anything the blueprint cannot say). |
| `prefab_files` | Paths (relative to the blueprint) of prefab libraries in the `assets/*.json` format, merged into the built scene's `prefabs`. A game ships its own props this way without touching the engine (place instances with `extra`); the map stays self-contained for the server. |
| `scene` | Raw top-level scene keys merged into the result (`vars`, `rules`, `weapons`, `checks.sim`, ...); a `checks` object merges into the generated one. |

What comes out: a `floor_<room>` plane per room; `wall_ext_N` (0.24 thick) around free edges and `part_N` (0.15) on shared ones, with the
door openings; a `sun` and one lamp per 8 x 8 m of room; `zones`, `spawns`, `portals` and `interest`; a camera at the first spawn; and `checks`:
lint (0 errors, the warnings found at build time), one `reach` per room, one `auto` walk from the first spawn to every other room, an object
count. The output is deterministic, so `red_engine2 build bp.json --check` fails (exit 1) when the committed map differs from what the blueprint
builds. A game project (`red_engine2 new-game`, `game check`) wires blueprints, maps and the server together; see ADR 0024.

## Validation

`red_engine2 validate scene.json` checks the file before any render time and reports errors as
`object_id.field: message` (or `camera.field` / `lights[i].field`) — same precise-pointer
convention as the 2D engine. Common failures: unknown `type`, missing required field for that
type, keyframe `t` values not sorted ascending, more than one shadow-casting light, unknown
`ease` name, malformed hex color, an opening outside its `wall`, a `door` shorter than 2.05 m.
Beyond schema validation, `red_engine2 lint scene.json` checks a *walkable map* for layout
problems (stairs that lead nowhere, unreachable rooms, overlaps, ...).

## Known limits (intentional)

No imported meshes/textures, no scene-object physics in the *offline* tools (nothing falls, bounces,
or collides on its own there; in the live viewer the player has its own simple gravity/collision
model, and small props are rigid bodies you can pick up, drop and knock over — see `movable` above), no per-vertex mesh deformation/skinning beyond the fixed
capsule-rig `humanoid`, no on-screen 2D text/UI overlay (that's the 2D engine's job — composite
the two if you need captions over a 3D shot), at most 16 lights and 1 shadow-casting light. The
goal is a small, auditable surface an AI can hold in context, not a general-purpose 3D suite.
