# Frameless

An endless art museum to get lost in. Rooms open onto rooms for ever, in wings that each hold one feeling: joy, grief, longing, wonder, rage, dread,
serenity, nostalgia, tenderness, loneliness, hope, awe, restlessness, mischief, love, regret. Every painting, tapestry, sculpture and installation is
one of a kind, alive in its own way, and has a name, an artist (some of them you will meet again down the corridor) and a story.

Every room keeps a key, hidden in one of its pieces, and its doors open only when it is found. Sometimes a sculpture gives it up when touched. More
often it is inside a painting: walk up to a painting and press E, and you fly into it and stand in its world, with the picture all around you and more
paintings standing in it. Sometimes the key is in a painting inside that painting, or deeper still. A painting on the way to a key keeps a faint golden
shimmer; the curator's note beside the door of each room gives a hint.

```bash
re2 examples/frameless/frameless.json                       # visit (E steps in or touches, M the map, Esc pauses: music, sounds, footsteps)
RE2_MUSEUM_FRESH=1 re2 examples/frameless/frameless.json    # a new museum instead of the saved visit
red_engine2 museum examples/frameless/frameless.json        # the first room: its pieces, plaques, where its key is, the curator's note
red_engine2 museum examples/frameless/frameless.json --room 0,-1 --inside 1/2 --shot out/deep.png   # the world inside a painting inside a painting
red_engine2 museum examples/frameless/frameless.json --styles out/styles.png --t 6                  # every painting style on one wall
red_engine2 museum examples/frameless/frameless.json --map out/map.png --explore 3                  # the pause menu's plan of the museum
```

The visit is saved (keys, rooms, paintings stepped into), so the museum is there next time; the pause menu's map shows the plan of what you have
explored, wing by wing, with the keys you found and the doors still shut.

What it is made of (the museum itself is the engine's `museum` block, ADR 2026-10-11-an-endless-museum-is-a-scene-block-the-engine-builds):

- `frameless.json`: the scene: a peaceful first-person player, a start card, the sky, the museum, the music.
- `collection.json`: everything that makes this museum this one: 26 painting styles and 10 sculpture materials by name, 16 themes (each with room
  colours, palettes, favourite styles and sculpture families, and the music of its wing), the words its titles, artists, years, media, origin stories
  and curator's hints are written with, and 16 hand-written masterpieces, each hanging in exactly one room of its theme.
- `shaders/art.wgsl`: every painted surface: the 26 living painting styles (ink currents, Rothko fields, horizons, the sea at night, a swirling starry
  sky, stained glass, rings on water, a profile in shifting light, Bauhaus shapes, a shattered pane, embers, a forest that looks back, a lit window in
  the rain, mosaic, aurora, pointillism, a galaxy, city lights through rain, a garden, a labyrinth, an eye that follows you, two lights that never
  meet, a woven cloth, a storm, a flower breathing, a candle in the dark), the 10 sculpture materials, and the still water inside a painting that
  mirrors its world.
- `shaders/sky.wgsl`: the blue hour that never ends over the skylights, and inside a painting, the painting all around.
- `audio/*.json`: eight scores, the movements of an endless music journey steered to the wing you are in (The Long Gallery, Letters, Sunlit,
  Lanterns, Undertow, Embrace, Vigil, Cathedral).
