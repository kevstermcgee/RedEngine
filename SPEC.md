# forge3d scene language

`forge3d` renders a single JSON **scene** file straight to an MP4. This is the complete
reference for that JSON format. Read this, not the source, to author scenes — the engine's
Rust internals are an implementation detail.

## Top-level shape

```json
{
  "meta": { "fps": 30, "duration": 6.0, "resolution": [1280, 720] },
  "background": { "sky_top": "#8fc7ff", "sky_bottom": "#eef6ff" },
  "ambient": { "color": "#ffffff", "intensity": 0.25 },
  "camera": { "fov": 50, "position": [0, 2, 8], "target": [0, 1, 0] },
  "lights": [ ... ],
  "objects": [ ... ]
}
```

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

`fov` is vertical field of view in degrees. `position`/`target`/`fov`/`roll` are all tracks.
`target` is the world-space point the camera looks at — orbiting a subject is a `position`
track around a fixed `target`, not a rotation track on the camera itself.

## Lights

Up to 4 lights. Each has a `type` of `"directional"` or `"point"`.

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
  shadow frustum centered on the origin — widen it if shadows clip on a large scene.
- With zero lights, the scene is lit by `ambient` only (flat, cartoon-ish look).

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

### Primitives

| `type`     | extra fields                                  |
|------------|------------------------------------------------|
| `box`      | `size: [w,h,d]` (default `[1,1,1]`)             |
| `sphere`   | `radius` (default `0.5`)                        |
| `cylinder` | `radius`, `height` (defaults `0.5`, `1`)         |
| `cone`     | `radius`, `height` (defaults `0.5`, `1`)         |
| `capsule`  | `radius`, `height` (defaults `0.3`, `1`)         |
| `plane`    | `size: [w,d]` (default `[10,10]`), always faces `+Y` |

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

A posable 3D figure built from capsules and a sphere head — the 3D analog of the 2D engine's
`stickfigure`. Pose is forward-kinematic: each joint is a rotation *relative to its parent*.

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
- `spine` and `head` are `[x,y,z]` degree tracks (bend/twist/tilt relative to their parent).
- `l_shoulder`/`r_shoulder`/`l_hip`/`r_hip` are `[x,y,z]` degree tracks (ball joints).
- `l_elbow`/`r_elbow`/`l_knee`/`r_knee` are single-number degree tracks (hinge joints — flexion
  only, always bends "forward").
- Every pose field is independently a track, so e.g. a walk cycle keyframes `l_hip`/`r_hip`/
  `l_knee`/`r_knee` out of phase while `spine` stays constant.
- The whole rig moves as a unit via the object's own `position`/`rotation`/`scale` (e.g. to
  walk the character across the scene, keyframe `position`, not the pose).

## Validation

`forge3d validate scene.json` checks the file before any render time and reports errors as
`object_id.field: message` (or `camera.field` / `lights[i].field`) — same precise-pointer
convention as the 2D engine. Common failures: unknown `type`, missing required field for that
type, keyframe `t` values not sorted ascending, more than one shadow-casting light, unknown
`ease` name, malformed hex color.

## Known limits (intentional)

No imported meshes/textures, no physics, no per-vertex mesh deformation/skinning beyond the
fixed capsule-rig `humanoid`, no on-screen 2D text/UI overlay (that's the 2D engine's job —
composite the two if you need captions over a 3D shot), at most 4 lights and 1 shadow-casting
light. The goal is a small, auditable surface an AI can hold in context, not a general-purpose
3D suite.
