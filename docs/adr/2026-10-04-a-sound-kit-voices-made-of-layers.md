# 2026-10-04. A sound kit: voices made of layers
Status: accepted
Summary: One small model (a voice is layers of source, envelope and gain) replaces the copied helpers and the per-weapon plumbing, with a golden that proves no sound changed.

## Context
Phase 1 (ADR 2026-10-04 "Audio you can measure") made sounds measurable. Reading `sfx.rs` to see what a kit would need showed that the 1.1 k lines are mostly one idea written out 40 times: a sound is a few
layers, each a source (a sine, a bell's partials, a pitch glide, noise through a one-pole filter) multiplied by a decay and a gain, summed, given a click-free attack and normalised. The same helpers
(`Noise`, `decay`, `attack`, `finish`, `partial`, the PolyBLEP saw) were copied into `sfx.rs`, `music.rs` and `audio.rs`/`synth.rs`, and a gun was a 13-number table feeding a hand-written loop.

## Decision
- `dsp.rs` holds the primitives once (`Noise`, `decay`, `attack`, `partial`, `saw`, `hz`, `OnePole`, `finish`, `samples`, `time`) and the model: a **`Voice`** (length, level, noise seed, attack on or off) of **`Layer`s**, each a **`Src`**
  (`Tone` with optional harmonics, `Glide`, or `Noise` through a `Filter`), an **`Env`** (exponential decay, optional fade-in, optional delay) and a gain. It is plain data and renders deterministically.
- Layers of one voice share one noise stream, as the real sounds did (a gun's crack, body and tail come from the same blast), and a layer that has not started yet still lets its filter hear the noise, so a
  delayed layer is identical to one that was always running and silenced.
- All 31 gun voices (one function over the existing table) and ten cues (hit tick, kill ding, level-up, hurt, bat hit, heartbeat, footstep, landing, victory, defeat) are re-expressed as voices. Other
  sounds (death, respawn, pad launch, the swing and jump breaths, alert, beeps, ambience, the music) use the shared primitives but are not layers yet: they need an integrated pitch sweep or a filter
  whose cutoff follows an envelope, which are the next two things the model should learn.
- **`audio golden`** stores how each built-in sound measures (length, loudness, peak, brightness, band shares, pitch for tonal sounds, clipped samples) in `tests/fixtures/audio_golden.json` and a test
  compares with tolerances (0.3 LU, 0.5 dB, 3 percent, not exact bits: `sin` and `exp` differ in the last digits between Linux and Windows). A refactor, or a tweak that changes how something sounds, is a line
  in a diff; `audio golden --write` accepts an intended change. This is what made the port safe: every sound matched its golden before and after.
- The clip-end rule in the standard was wrong in phase 1 (it flagged a loud tone that `finish` fades over 2 ms) and is now: audible last samples AND a jump to silence.

## Consequences
- A gun is `Voice::new(...)` plus four layers; adding a sound that fits is ten lines and no loop. `sfx.rs` lost about 90 lines net and every copy of the helpers.
- The pump-action shotgun's two clacks now draw from the shared noise instead of two extra draws per sample, so those few samples differ from before (the sound measures the same, inside the golden).
- The model is the vocabulary for phase 3: a voice described in JSON is these same structs, and a scene can carry its own. Not here: serialization, an integrated sweep, an envelope on a filter, an ADSR, FM,
  stereo voices, effects (delay, reverb, compressor), and music written in the same terms.
