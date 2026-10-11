# 2026-10-11. Music that never repeats: an audio journey of freshly rendered sections
Status: accepted
Summary: `audio.journey` plays an endless run of sections, each one of the scene's scores rendered on a worker with a new seed, a key that moves by a musical step and (where a track lists `A | B` alternatives) a new progression, dealt like a shuffled deck and crossfaded; `audio journey SCENE` lists and mixes them headless.

## Context
A scene's music was a loop: the built-in synthwave, or one score per time of day crossfaded by the clock (ADR 2026-10-04 soundscape). However good, a
loop is heard as a loop within minutes. Wormhole asks for music that keeps changing for as long as someone watches, with the same small data a score
already is, and checkable without ears like every other sound (ADR 2026-10-04 audio you can measure).

## Decision
- `"audio": {"journey": {"scores": [..], "seed": "launch", "fade": 12, "repeats": 2}}`. `ambience::Journey` deals sections: scores like a shuffled deck
  (every score once per round, never the same twice in a row, the first score opens), a fresh seed per section (`mix32` of seed and index: no index is
  reused, so the journey never loops), and a key shift that moves by 0, a fourth, a fifth, a step or a third, folded to within a tritone of the score's own
  key so its register stays where it was written. `Section::score_json` writes the seed and key into the score's JSON; the score format is unchanged
  except that a chord track may list alternatives, `"i VI III VII | i iv VII III"`, of which the seed picks one (hashed, so neighbouring seeds differ).
- `ambience::JourneyPlayer` is the timeline with no sound card: two slots, an opening fade-in, an equal-power crossfade `fade` seconds before a section's
  `repeats` passes end, and a late section simply lets the current loop keep playing (it is seamless). re2 drives it: a long-lived worker renders what it
  wants, `Audio::replace_layer` reuses the two layers, the player's MUSIC switch and ducking apply as to any music. A scene has the hour's `music` or a journey, not both.
- `red_engine2 audio journey SCENE [--sections N] [--seed S] [--out mix.wav]` prints each section (score, key, seed, loop length, LUFS, peak) and mixes them
  through the same `JourneyPlayer` into one WAV, so a change can be heard, measured and drawn (`audio picture`).

## Consequences
- Rendering a section costs about 7 s of one core in the dev build for 80 s of music (release not measured; it is faster), on a worker, once per two to
  three minutes, so it is ready long before it is needed.
- Variety comes from the scores an author writes: five scores of four tracks give endless renderings, but one score heard in twelve keys is still that score.
  Seeds vary walks, spread pans, velocities and listed progressions; they do not vary instruments, tempo or scale.
- Undo: remove `journey` from `parse_audio` and the journey half of re2's `Ambient`; the `|` alternatives are `score::progression`.
