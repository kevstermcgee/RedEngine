# Wormhole

An endless fall through a tunnel of light, in the spirit of the Star Gate in *2001: A Space Odyssey*: colours, patterns, shapes, waves of light and music that never come round again. Look around and let it carry you.

Nothing is asked of you: move the mouse to look ahead, aside or behind as you fall. Every launch draws a new seed, and from it a journey that does not loop.

```bash
re2 examples/wormhole/wormhole.json                                       # fall (mouse looks, Esc pauses: music can be turned off there, or with N)
red_engine2 frame examples/wormhole/wormhole.json out/w.png --t 140       # a picture of any moment (seed 0); --eye 0,0,0 --at 10,0,-4 looks at the wall
red_engine2 audio journey examples/wormhole/wormhole.json --sections 6    # the music's first six sections: score, key, seed, length, loudness
red_engine2 audio journey examples/wormhole/wormhole.json --sections 3 --out out/journey.wav   # ...mixed exactly as the game plays them
```

What it is made of (all of it the engine's; the only code is the sky, which the engine checks for every GPU backend when the scene is validated):

- `wormhole.json`: a still player (`player.still`: only looks), a clean screen (`hud.enabled: false`), a start card, the sky and the music.
- `sky/wormhole.wgsl`: `fn sky(dir, px)`, the colour seen in each direction (`sky.shader`, ADR 2026-10-11-a-game-draws-its-own-sky-with-a-checked-wgsl-function). The view ray meets a wall around the travel axis; colour and pattern belong to 150-unit stretches of tunnel (palette, five pattern families, twist, arms, ripples, each drawn from a hash of the seed and the stretch number), so a new world appears at the far end and roars past; the tunnel's shape (round, polygon, petals, a slit-scan lens) morphs on its own 41-second clock; pulses of light race in at random; the speed breathes.
- `audio/*.json`: five scores, the movements of an `audio.journey` (ADR 2026-10-11-music-that-never-repeats-an-audio-journey-of-freshly): Event Horizon (it always opens), Glass Rain, Choir of Stars, Tide and Aurora. Each section is one of them re-rendered with a new seed in a key moved by a musical step, with chord tracks choosing among their listed progressions, crossfaded over 12 seconds.

`seed N` is printed at launch: the same number is the same sky and music.
