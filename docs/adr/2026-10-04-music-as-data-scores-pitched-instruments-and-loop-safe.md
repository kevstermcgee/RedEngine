# 2026-10-04. Music as data: scores, pitched instruments and loop-safe rendering
Status: accepted
Summary: A score of instruments and tracks (chords, patterns, seeded walks) in a key and scale renders to a seamless stereo loop at a stated loudness.

## Context
Sounds became data in the previous slice. Music was still one hard-coded synthwave loop (`music.rs`: eight bars in A minor at 132 BPM, every note a hand-written loop), so a game could not have its own music, and
there was no way for an AI to compose without writing DSP. The user's taste is ambient and soft (long-form, sleep-music style): slow pads, a drone, sparse notes, long reverb. That needs harmony, slow envelopes,
randomness that is reproducible, effects, and a loop with no click.

## Decision
- **Instruments are voices** (`pitched`): the sine values are multiples of the note played, so the pad, the bell and the pluck are the same JSON as any sound. `Voice::render_note(hz, hold, salt)` plays one
  (the gate plus its release, at most its own length; the salt varies the noise per note).
- **A score** (`score.rs`) is instruments, tracks and a few settings: `bpm`, `beats`, `bars`, `key`, `scale` (twelve named scales), `seed`, `lufs`, `reverb`, `delay`. A track plays an instrument as `chords` (a progression such as
  `"i VI III VII"`: each chord is the scale's own triad, with an optional `voicing` and `strum`), `pattern` (a rhythm string `"x.x. ..x."` with `notes` cycled across the hits) or `walk` (a seeded random walk through the scale:
  `every`, `density`, `range`, `leap`). Notes are **scale degrees**, so everything is in tune. Tracks have `octave`, `hold`, `gain`, `pan` (a number or `"spread"`), `send` to the effects, `vel`, and `from`/`to` bars.
  Strict parsing with paths and did-you-mean, like everything else.
- **Effects** (`audio_fx.rs`): a stereo ping-pong delay (time in beats) and a Freeverb-style reverb whose comb feedback is computed from `decay`, so the tail really falls 60 dB in that many seconds (tested). `mix` is the wet
  level relative to what was sent in (RMS), not a magic gain.
- **Seamless loops by construction**: notes are mixed with their tails wrapped modulo the loop, and the effects run over enough repeated passes (from the reverb and delay tails) that the last pass is the steady state.
  A test measures the seam (about 1 for a clean loop) with a 5 s reverb and a delay.
- **Loudness is stated, not tuned by ear**: the loop is scaled to the score's `lufs` using the BS.1770 meter from phase 1 (within 0.3 LU in tests), and turned down, not clipped, if its peak would pass full scale.
- The tools take scores: a `.json` with `tracks` is a score anywhere a sound is accepted; `score.ambient_drift` (assets/audio) is built in, listed, checked and in the golden, and `audio export score.ambient_drift` prints it as a template.

## Consequences
- The ambient example is a 40 s, 8-bar loop at exactly -26 LUFS with a seam of 0.7: a drone, four chord pads, and two seeded bell walks, in about 30 lines of JSON.
- A walk is the generative piece: a different `seed` is a different, equally in-key piece. The next slice adds the scene `audio` block and richer generators on the same model.
- Rendering is not free: that score takes a few seconds in a dev build (about 60 million sine evaluations); the shipped scores are rendered once per process.
- Not here: scene-carried music in the client, drums and filters on notes beyond the existing swells, chord extensions beyond `voicing`, tempo changes, per-track effects (only a `send` amount), and converting the synthwave loop.
