# 2026-10-04. Sounds as data: the JSON voice and audio export
Status: accepted
Summary: A voice has a compact strict JSON form, built-in voices export to it, and the audio commands take JSON files, so a sound is written and judged without Rust.

## Context
Phase 2 gave sounds one model (`Voice` of `Layer`s). To make sounds cheap for an AI to author, that model has to be writable as data: a sound should be a few lines of JSON, rendered, measured and corrected
in a loop that never touches Rust. Three things stood in the way: some sounds still needed things the model could not say (a pitch glide with a clean integrated phase, a swell in and out, a noise filter
that opens as the sound swells), there was no serialised form, and the `audio` tool only knew built-in names.

## Decision
- The model learns three things, each taken from sounds that were hand-written for lack of them: `Src::Sweep` (an exponential glide with integrated phase), `Env::attack`/`release` (a raised-cosine swell in
  and out, measured to the end of the voice), and `Filter::LowOpen` (a low-pass whose coefficient follows the layer's own swell). A bump is `attack = release = half the length`, which is exactly the
  `sin^2` window the swing, jump and throw breaths used. Death, pad launch, swing, jump and throw are now voices; every sound still matches its golden.
- `voice_spec.rs` is the JSON form: `{"seconds", "level", "seed", "attack", "layers": [...]}`, a layer being one source (`sine` with `harmonics`, `glide`, `sweep`, `noise`) plus `decay`, `fade_in`, `delay`,
  `attack`, `release`, `gain`. `noise` is a short string (`"white"`, `"lp 0.2"`, `"hp 0.5"`, `"lp 0.05..0.55"`). Numbers are checked for range, unknown fields get a did-you-mean with the list of fields,
  and every message carries its path (`layers[2].decay`).
- `voice_to_json` is the exact inverse (defaults left out, numbers printed as the shortest exact decimal), and a test proves that every built-in voice survives the trip and renders the same.
- `audio export NAME` prints any built-in voice as JSON (the `json` rows of `audio list`), and `report`, `render`, `picture` and `check` accept a `.json` sound file anywhere a name works. The
  edit loop is: export or write, `report` for numbers, `picture` to look, `check` for the standard, `render` for a WAV.

## Consequences
- A gun is about 250 bytes of JSON, a chime four lines; both are measured the same way as built-in sounds. A sound that fits needs no Rust.
- 40 of the 67 built-in sounds are voices (24 guns and 16 cues); the rest (explosion and the kit sounds, respawn, alert, beeps, ambience, music) are still hand-written
  and `audio export` says so. Most need only an ADSR or two more filter shapes, which the next slices add.
- The format is deliberately flat and small; everything in it also exists as plain Rust (`Layer::new`, `Env`), so there is one model, not two.
- Not here: scenes carrying their own sounds (the next slices bring a scene `audio` block), scores and a sequencer, stereo and effects, loudness targeting.
