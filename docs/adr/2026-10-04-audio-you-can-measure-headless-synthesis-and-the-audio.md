# 2026-10-04. Audio you can measure: headless synthesis and the audio command
Status: accepted
Summary: Synthesis and loudness analysis build headless; red_engine2 audio lists, reports, renders, draws and checks the engine's sounds so audio can be judged without ears.

## Context
The engine's audio is all synthesized in code (ADR 0008: nothing imported; ADR 0056: music in code), which is the right foundation, but until now it could not be inspected. A sound could be
judged only by a unit test that checked a few properties, or by a person listening. An AI author has no ears, and CI has none either. The synthesis (`sfx.rs`, `music.rs`) was also locked behind
the `gfx` feature, only because the sample rate and WAV encoder sat in the `rodio` player's file, so no headless tool could even render a sound.

## Decision
- The device-free part of `audio.rs` moves to `synth.rs` (`SAMPLE_RATE`, the WAV encoder, the two one-off clips); `audio.rs` re-exports it, so every path still works. `sfx` and `music` are no longer
  `gfx`-gated. The headless server dependency tree is unchanged (CI checks it).
- `audio_analysis` measures a clip and returns a `Report`: peak and RMS (dBFS), crest, **integrated loudness in LUFS** per ITU-R BS.1770-4 (K-weighting designed for any rate, 400 ms blocks, absolute
  and relative gates; a mono clip counts as dual mono because that is how the engine plays it; checked against the EBU reference: a -23 dBFS 1 kHz stereo sine reads -23 LUFS at 44.1 and 48 kHz),
  clipped samples, DC, silence before and after, how abruptly it ends, the loop seam (the last-to-first jump as a multiple of the typical step), the spectrum (own radix-2 FFT: brightness, five
  energy bands, strongest pitch as a note with cents, flatness) and stereo correlation. It also draws a spectrogram. Everything is pure and deterministic.
- `red_engine2 audio list|report|render|picture|check`: a catalog of every built-in sound (guns, effects, the tactical set, the one-off clips, music, ambience), one numeric line per sound (or `--json`),
  a WAV, one PNG (waveform over spectrogram) and a standard with FAIL lines (exit 1) and WARN lines. A `.wav` path works anywhere a name does. `describe audio` is the reference.
- The standard is in `audio_analysis::problems`: finite, no full-scale samples, audible, little DC (looser for one-shots, which are built on a low thump), one-shots end on zero (judged on the last 8
  samples, not on a longer window, because the engine fades every clip over its last 2 ms), loops have no seam. A loudness outlier (10+ LU from the median of its group) is a warning, not a failure:
  loudness is a mixing choice, but a cue that drowns or vanishes should be seen.
- A test holds every built-in sound to the standard (`every_builtin_sound_meets_the_standard`), so a new or edited sound that clips or clicks fails CI.

## Consequences
- First measurement of the engine's own sounds (73 clips): one real defect, fixed here: `synth_bat_hit` clipped (21 full-scale samples) and ended on a click. The loudness spread across the whole catalog is
  25 LU (tonal cues such as `fx.go` and the countdown beep sit near -7 LUFS, reloads near -27, the restrained `tactical` set near -24); within a group only `fx.reload_mag` is a 10+ LU outlier. Whether
  to rebalance is a taste call left to the author; the tool now makes it visible.
- The melee weapons and grenades have no gun voice (their sound is the swing or the kit's), so they are not listed as guns.
- Limits: sample peak, not true peak (no oversampling); one spectrum per clip, not a time-varying loudness curve (the picture shows time); pitch is the strongest partial, not a transcription.
- This is the measurement layer. Authoring tools (a shared DSP kit, declarative sound and score descriptions, an ambient generator), runtime mixing (buses, ducking, voice limits, reverb, per-listener
  mixes) and scene-level audio checks are planned in `docs/analysis/2026-10-04-audio-survey-and-plan.md`.
