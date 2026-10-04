# Audio: survey and plan (2026-10-04)

Goal (from the user): reliable, high-quality music and audio tooling that **minimizes token usage** while giving the audio side of the engine **tremendous depth**.

## Design principles (the user asked for "elegant yet powerful and effective")
- **One model.** Sounds, music and ambience are built from the same few composable pieces (oscillators, noise, envelopes, filters, a small set of effects, a sequencer). A gunshot, a pad and a drone
  differ in how they are described, not in what they are made of. No parallel systems for effects versus music.
- **Few concepts, deep results.** The description language stays small enough that its reference fits in about 2 KB; depth comes from composition and seeded variation, not from a long list of options.
- **The loop closes on measurement.** Everything an author makes can be rendered and judged with `audio report|picture|check`, so "effective" is a number and a picture, not a hope.
- **Deterministic and headless.** Same description, same samples, on every machine; no device needed to author, test or verify.

## What exists
- All audio is synthesized in code, nothing imported (ADR 0008). `sfx.rs` (1169 lines): one gun voice per firearm from a 13-number table, ~35 hand-written cues, the map ambience. `music.rs` (337 lines): ONE
  hard-coded 8-bar synthwave loop (A minor, 132 BPM). `kart_sound.rs` maps race HUD changes to cues.
- Every clip is hand-written DSP: loops and constants per sound. The same helpers (a noise generator, a saw, `decay`) are copied between `sfx.rs`, `music.rs` and `audio.rs`. A new sound or song is new Rust.
- Playback (`audio.rs`, `rodio`): fire-and-forget clips, one looping music sink, a music volume and an SFX on/off switch (persisted, ADR 2026-10-01). No buses, voice limit or priority, ducking, reverb or
  occlusion; one global listener (`sfx::spatial` pans by one listener's yaw), which split-screen will have to change.
- The scene has `music: bool` and nothing else: no per-scene music, no audio checks, no way to describe a sound as data.
- Before this work there was no way to inspect any of it: no render-to-file, no measurement, no picture.

## Phases
1. **Measure (done in this PR).** Headless synthesis; `audio list|report|render|picture|check`; BS.1770 loudness, spectrum, pitch, seam, a standard that CI enforces. See ADR 2026-10-04.
2. **A DSP kit (done, PR in review; ADR 2026-10-04 "A sound kit").** One shared, tested set of building blocks (oscillators incl. band-limited saw/square, noise colours, ADSR and exponential envelopes, one-pole/biquad/state-variable filters, delay,
   a reverb, a compressor/limiter, a sidechain duck, soft clip) replacing the copies. Existing sounds are re-expressed on it with their `audio report` lines as the golden: the refactor must not move them.
3. **Describe sounds and music as data.** A compact JSON patch (voices, envelopes, filter moves) and score notation (patterns as strings like `x..x..x.`, chords as scale degrees, seeded variation), so a
   sound is ~10 lines instead of ~40 of Rust and an AI writes it cheaply; `describe audio` stays under ~2 KB. A **generative ambient engine** (seeded drones, slow pads, sparse motifs from a scale, long
   reverb tails; the style of long-form ambient/sleep music the user referenced), checked by the same measurements (no seam, loudness range, spectral balance, no clipping).
4. **Runtime mixing.** Buses (master, music, sfx, ui, ambience) with the persisted toggles; voice limiting and priority; ducking music under important cues; a per-scene reverb send; a music state machine
   driven by rule variables (intensity layers); listeners per player, which is also what split-screen needs.
5. **Audio in the game's own checks.** `checks.audio` in a scene (loudness range, no clipping, loop seam, silence) run by `verify`, and the headless client's state dump gets an audio section (what played,
   when, how loud) so a script can assert it.

## Why this order
Measurement first: it found a real defect in the first run and gives every later phase a golden to hold to. The DSP kit next, because data-described sounds need something to be described in terms of.
Runtime mixing waits until there is something worth mixing, except that listeners per player must be designed together with split-screen.

## What phase 2 taught phase 3
- A voice is already the data model: `Voice { seconds, level, seed, attack, layers: [Layer { src, env, gain }] }`. Phase 3 is mostly `serde` for these structs plus a scene `audio` block, not a new engine.
- A gun in that model is 4 layers; as JSON about 250 bytes against a Rust table and a loop.
- The model should learn next (each needed by sounds that are still hand-written): an **integrated pitch sweep** (death, respawn, pad launch), a **filter whose cutoff follows an envelope** (the swing and
  jump breaths), an **ADSR / held note** (beeps, alert), **stereo voices and a few effects** (delay, reverb, compressor), and a **sequencer** so music is the same layers placed on a grid.
- `audio golden` is the safety net for all of it: port a sound, run it, and it either matches or the diff says by how much.

## Phase 3 progress
- **3a (done): sounds as data.** Sweep, swell (attack/release) and an opening filter added to the model; JSON voice form (`voice_spec`); `audio export`; the audio commands take `.json` sound files. 40 of 67 built-in sounds are voices.
- **3b (done; ADR "Music as data"): scores.** A sequencer (the same layers placed on a grid, patterns as strings, chords and scales), pitched voices (an instrument is a voice whose partials are multiples of the note), stereo placement,
  a delay and a reverb, loop-safe rendering (the reverb run twice so the tail wraps), and **loudness targeting** (a score states its LUFS and the meter makes it exact).
- **3c (next): richer generators and a scene `audio` block** (seeded drones, slow pads, sparse motifs from a scale, long reverb; the client plays the scene's own music and ambience).
