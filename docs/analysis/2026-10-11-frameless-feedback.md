# What building Frameless taught us about Red (2026-10-11)

The ask: *"Create an endless 3d art museum game. Sculptures, paintings, any kind of art; doors between rooms. Each piece has a name, an origin story
and an artist. Art runs the gamut of human emotions; rooms have themes. First person: walk up to pieces and interact; faint footsteps (can be turned
off). Paintings are virtual: the player steps into them; each room has a key hidden in one of its pieces, sometimes a painting inside a painting.
Stepping in should feel special. Unique pieces, a variety of frames, art that feels alive. A catchy name. Count keys and rooms; a map in the pause
menu of the layout and the paths explored. A game to get lost in, with phenomenal touching music."*

The result is **Frameless** (`examples/frameless`: one scene, a 1 700-line collection, two WGSL files, eight scores) on a new engine block,
`museum`. This file is the issue log: what the engine could not do at the start (and was added), and what is still missing, ranked by what it would
save the next game of this kind.

## The short version

* **At the start the engine could not build this at all.** A scene is a fixed list of objects uploaded once; an endless building has to be made as
  the player walks, its doors have to open, its paintings have to be pictures (not flat colours) and the player has to be able to stand inside them.
  The only endless world (`procgen`) is Marcel's meadow. So the PR adds a **`museum` block** and the engine code behind it (layout, furnishing, text,
  keys, the worlds inside paintings, sculptures, the visit, a GPU layer, the client) driven by a **collection file** of pure data, and four general
  pieces: a game's **painted surfaces** (`museum.art`, a checked WGSL `surface` function) with **`game_param`** numbers for a game's shaders,
  **book type** from the player's own system serif, **journey steering** for music, and a **footsteps** setting.
* What worked very well: the sky-shader precedent (a game's WGSL, checked by naga with its own line numbers) extended to surfaces in an hour;
  `validate` + the typed-content drift test caught every collection mistake; `re2 --headless --script` with `approach`/`look_at`/`expect`
  played the whole loop (walk to the painting, step in, take the key, step out, doors open, walk into the next wing) with pictures at every step;
  `audio check`/`report`/`picture` judged eight scores without ears.

## What was built

| | |
|---|---|
| Plan | an endless grid of rooms (five sizes, four heights, pillars, partitions, skylights, four floors), a binary-tree maze plus a third of the other walls as doors, wings of 3x3 rooms per theme with guest rooms, gallery numbers in a square spiral |
| Art | per room 6-11 pieces: paintings (10 frame styles), triptychs (one picture over three panels), tapestries that stir in the air, 14 standing and 4 hanging sculpture families, each with motions that answer the viewer (a head turns to watch you, a flower opens as you come near, a heart beats faster, shards tremble, a mobile turns, chimes ring when touched) |
| Words | titles, artists (a handful resident per wing, so names recur), years ("c. 1890" on the label, "around 1890" in a story), media matched to materials, 140 origin-story templates, curator's hints; 16 hand-written masterpieces, each in exactly one room of its theme; a per-room deck so no story repeats in a room |
| Keys | one per room: in a sculpture (touch it and it gives the key up) or in a painting 1 to 3 worlds deep; the first room teaches stepping in; a painting on the way glints gold; doors swing open when either room's key is found |
| Worlds | inside every painting: the painting all around (the sky draws it with the painting's own function), mirror water that reflects it exactly, painted ground, dunes or stepping stones over nothing, more paintings on easels, pillars or in the air, the key on a pool of light, the way back |
| Flight | the camera glides into the canvas until it fills the view, the painting's colour floods the screen, a shimmer of bells; the world opens outward from where you look as the field of view widens; out again in reverse |
| Client | serif labels with the story under each piece, room and painting titles, key and room counters, a pause menu (map, music, sounds, footsteps, step out), the map (wings, doors open or shut, keys, glimpsed rooms, you), saved progress, faint reverberant footsteps |
| Music | eight scores (piano, strings, choir, harp, celesta, bells, cello; written melodies over chords plus generative counter-lines) in an endless journey that follows the wing |
| Tools | `museum` (a room's plaques and key; `--shot`, `--look N`, `--inside N/I/J`, `--map`, `--styles`), script targets `museum:N`, `museum:key`, `museum:way-back`, `museum:door:SIDE`, `RE2_SEED`, `RE2_MUSEUM_FRESH`, `RE2_FONT` |

## Ranked findings still open

### 1. There is no general way to add a generated world  *(highest)*
* Happened: the procedural meadow has its own streamer and GPU layer (`stream_gpu`); the museum needed another (`museum_gpu`), its own colliders hand-fed
  to the player, its own lamps copied into `scene.lights` every frame, its own hooks in both renderers. The next generated world (a dungeon, a city)
  will write a third.
* Engine should: one trait (`models()`, `colliders_near()`, `lamps()`, `shader_params()`) that both renderers draw and the client and tools ask, with
  the meadow and the museum as its first two implementations.

### 2. The live client has no seam for a game mode
* Happened: wiring the museum into `re2` touched eleven files (`main`, `frame`, `events`, `window`, `online`, `weapons`, `feedback`, `dump`, `ambient`,
  `headless`, `help`): update after the player moves, freeze input during a flight, own the camera, draw a HUD, replace the pause menu, take E and M,
  footsteps, the state dump, script targets.
* Engine should: a `GameMode` hook with those eight entry points, the museum moved behind it; then a kart race and a museum are the same kind of thing.

### 3. The pause menu cannot be extended
* Happened: the museum needed MAP, FOOTSTEPS and STEP OUT; the engine's menu has a fixed list, so the museum draws its own.
* Engine should: `PauseInfo` takes extra rows (id, label) and returns their ids from `pause_action_at`; the footsteps toggle (now a setting) belongs in
  the engine's own menu for every game.

### 4. No lights for pictures
* Happened: sixteen lights for the whole frame and no spot lights: a museum wants a picture light per painting. Paintings glow a little instead
  (`surface` returns a glow), which reads well but is not light falling on canvas.
* Engine should: clustered or per-room light lists (the museum knows which room each light is in), and a spot light kind.

### 5. Scene objects cannot use a painted surface
* `ObjectUniform` has `surface` and `palette` now, but only the museum layer sets them; `material.surface` for authored objects is the obvious next
  step (a poster, a stained-glass window, a TV in any map).

### 6. Smaller
* A script `shot` is taken at the next frame, after the following instant steps have run: `press: key:Escape`, `shot`, `press: key:M` photographs the
  map, not the menu. Take the shot before the next step, or say so in `describe playtest`.
* `approach` fails when its target disappears (the key, taken when reached). An `until` (a state pointer) would end it cleanly.
* The score format has rhythm per track, not per phrase: a melody with long and short notes is several `pattern` tracks with `from`/`to`. A `melody`
  play kind (`"notes": "5:1 3:0.5 ..."`, degree:beats) would make writing tunes direct.
* `describe` overview budget: adding one command meant trimming three other commands' lines.
* `Collider2D` had neither `Debug` nor `PartialEq` (added).
* Not measured here: real-GPU frame time (software rasteriser only), a gamepad, the Windows installer, how the music and footsteps actually sound.
