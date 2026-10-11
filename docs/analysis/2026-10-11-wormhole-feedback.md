# What building Wormhole taught us about Red (2026-10-11)

The ask: *"Create a hypnotic endless 2001: A Space Odyssey-esque wormhole with this game engine. Call it Wormhole. It should go on forever with unique
patterns and color combinations: different shapes and movements of waves of color splashing and roaring towards us, as if falling through a colorful
vibrant wormhole forever, not the same thing on a loop. No gameplay, a simulation; you can look around as you fall/float. Beautiful and moving, with
fitting, touching, unique music throughout. Publish it to the Games website and attach feedback to the engine."*

The result is `examples/wormhole` (one scene, one 330-line WGSL sky, five scores), published through `games-publish.json` like Marcel. This file is the
issue log: what the engine could not do at the start (and was added in the same PR), and what is still missing, ranked by what it would save the next
experience-style game.

## The short version

* **At the start, the engine could not draw this at all.** Every pixel of a wormhole is a function of view direction and time; a scene could only hold
  objects and pick sky colours. The only escape was a crate of my own on `red_engine2::app` with raw wgpu, and the release pipeline cannot ship that (it
  packages `re2`/`re2d` plus data; `native_playables` exists in the manifest and nothing reads it). So the PR adds **`sky.shader`**: a game's own WGSL
  function for the background, checked by naga when the scene is validated (ADR 2026-10-11-a-game-draws-its-own-sky-with-a-checked-wgsl-function).
* **Music could only loop.** Added **`audio.journey`** (endless sections re-rendered with new seeds and keys, dealt like a deck, crossfaded), chord
  **alternatives** in scores (`"i VI III VII | i iv VII III"`), and `audio journey SCENE` to list and mix it headless (ADR
  2026-10-11-music-that-never-repeats-an-audio-journey-of-freshly).
* **A player could not float.** Gravity, walk and jump all had minimums above zero, so a scene with no floor dropped the player for ever. Added
  **`player.still`**, and taught the `leak` lint that a still player cannot walk off the map.
* What worked very well: `frame --t` as the shader's inner loop (0.3 s a picture on the software GPU, so tuning colour was a conversation with pictures),
  `audio report/check/picture` for judging music without ears, `re2 --headless --script` with `look` steps for checking the live client from every angle,
  `validate`'s strictness (it caught three typos in my first scores), and the release pipeline needing nothing new.

## What was built

| | |
|---|---|
| Sky | the view ray meets a wall around the travel axis (round, polygon, petals, a slit-scan lens; the shape morphs every 41 s); colour and pattern belong to 150-unit stretches of tunnel, each drawing palette (cosine palettes in four families), a mix of five pattern families (rings/helices, domain-warped ink veins, lit tiles, aurora ribbons, spokes), twist, arms and ripples from a hash of (seed, stretch); streaks of light stretched by the speed; random pulses that race in faster than the fall; breathing speed; a bright heart whose colour eases between stretches; filmic tone curve |
| Endless | stretch index and shape epoch grow for ever and the seed is new each launch (`seed N` printed); nothing is a loop |
| Music | five scores (Event Horizon, Glass Rain, Choir of Stars, Tide, Aurora: drones, choirs, glass and bell walks, a noise "tide"), each with 4 alternative progressions; a section is ~2.5 min, crossfaded over 12 s |
| Checked | `validate`/`verify` clean (lint 0/0); the sky passes naga parse, uniformity, no-derivatives-in-loops and HLSL translation; `audio check` passes on all five scores (loudness -24/-25 LUFS, no clipping, seams clean); a 4-section, 9.3-minute mix has no gaps (spectrogram); the live client headless, looking ahead, aside, behind and down; the windowed client under a virtual display (start card over the moving tunnel) |
| Not verified | **real-GPU frame rate** (only a software rasteriser here); how it looks on a real monitor in motion (I looked at stills, never motion); **how the music sounds** (numbers and spectrograms only); a gamepad looking around; the Windows installer on a Windows machine |

## Ranked findings still open

### 1. A game's shader has no cost budget and nothing measures it  *(highest)*
* Happened: the first version did three five-octave fbm calls per wall evaluation and two wall evaluations in blend zones (~500 hash calls a pixel); I
  only noticed by reading my own code. This machine has no GPU, so `render-trend` and real frame times are unavailable; `frame` timings are a software rasteriser's.
* Did: cut it to three-octave noise, skip the ink pattern where a stretch barely uses it. Still unmeasured on hardware.
* Engine should: (a) a `render_scale` for the background pass (draw the sky at half resolution and upscale: a full-screen procedural sky is the one pass
  that scales with pixels only); (b) a static cost estimate in `validate` (count of noise calls and loop trips in the composed module) with a warning
  above a budget; (c) a `perf` check that times `frame` on a real GPU when one exists.
* Fixed when: `verify` on a GPU machine prints the sky's milliseconds per frame at 1080p, and `validate` warns about an obviously expensive sky.

### 2. The sky cannot hear the game, and the game cannot touch the sky
* Happened: the sky sees only time, direction and seed. Ideas that would make the fall more responsive (hold a key to dive faster, pulses on the
  music's downbeats, a colour change when the music changes key) are impossible: no rule variable or audio clock reaches the shader.
* Engine should: `sky.params: {"speed": "dive"}` binding up to 8 rule variables (smoothed) to `sky_param(i)`, and an audio clock (`music_beat()`,
  `music_section()`) in the same uniform block; `globals` has room.

### 3. A one-body peaceful scene still opened the character select
* Happened: without `player.humans_play_as` the client opens the "choose a character" screen before a scene with no body in view. I set
  `humans_play_as: "human"` once I noticed.
* Engine should: skip the select when the player is `still`, or when `player.mode` is peaceful and the view cannot be switched; or have `lint` say so.

### 4. Score seeds that are close produce nearly the same opening notes
* Happened: `score::notes` seeds each track with `Noise(seed ^ ...)` and xorshift's first output barely depends on low bits: seeds 1, 2, 3, 4 all draw
  -0.74 first. I hit it in the new chord-alternative pick (every seed chose the same progression) and fixed that with a hash. The journey is safe (its
  section seeds are full 32-bit hashes), but an author who writes `"seed": 1` then `"seed": 2` hears nearly the same first bars.
* Engine should: hash the seed before xorshift in `notes()` (one line), then re-bless `tests/fixtures/audio_golden.json` and listen to Marcel's four
  scores again: every seeded score changes, so it is a deliberate, separate change.

### 5. Long sessions and f32 time
* `sky_time()` is scene seconds as f32: a millisecond is still resolved after 4 hours, 4 ms after 10. Wormhole's depth is about 6 x time, so after a
  day of falling the finest rings would shimmer slightly. Engine should: also pass time wrapped at an hour plus the hour count, and say in `describe`
  how to build continuous patterns from them.

### 6. Smaller
* `frame` has no `--seed` for a `"launch"` sky (it always draws seed 0); to look at another seed I edited the scene.
* `Q` (third person) still works in a still scene: the player's body then floats in the tunnel. Harmless, maybe even charming, but a scene cannot lock the view.
* The background pass is outside bloom/post: glow has to be faked in the shader.
* The ocean reflects the engine's sky, not a game's (documented; no scene needs both yet).
* `native_playables` in `.release-games.json` suggests custom crates can be released; nothing builds them. Either remove the key or document the plan.

## Cost of this run
Engine changes: ~1.1k lines (sky shader composition and check, journey planner/timeline/client/tool, chord alternatives, still player, lint, tests,
docs, two ADRs). Game: 330 lines of WGSL, five scores, a 25-line scene. The longest loop was tuning the look (eight contact sheets); the longest wait
was CI.
