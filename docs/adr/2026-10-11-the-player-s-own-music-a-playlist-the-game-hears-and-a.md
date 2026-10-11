# 2026-10-11. The player's own music: a playlist the game hears and a sky that sees it
Status: accepted
Summary: `audio.playlist` plays audio files the player chooses (file picker, drag and drop; mp3/flac/ogg/wav/m4a decoded by symphonia under `gfx`), shows each title for a few seconds, pauses music and picture together with Space, and hands a game sky 32 spectrum bands, levels, beats and per-song/per-play seeds through 12 new uniform vec4s; `frame --song` draws any moment headless.

## Context
Chroma (examples/chroma) is a music visualizer: it is nothing without the player's music. The engine had no way to read an audio file (ADR 0008:
games ship procedural sound only), no file picker in a game, no way for a sky to hear anything (the Wormhole feedback's finding 2), and no way to
freeze a sky's clock.

## Decision
- **ADR 0008 still holds for what games ship.** A playlist game ships no audio: the files are the player's, read from their disk when they choose
  them, never copied. Decoding is symphonia (pure Rust: mp3, flac, ogg vorbis, wav/pcm, m4a/aac), an optional dependency of `gfx`, so the headless
  server and bot do not carry it.
- `"audio": {"playlist": true | {"title_secs": 10, "corner": "bottom_left", "shuffle": false}}` (not with `music` or `journey`). The live client
  (`bin/re2/listen.rs`) opens the picker when the game starts with nothing queued; O opens it again (replacing the queue); files dropped on the window
  are queued after the current song (a batch in its order); Right/Left skip; Space pauses; N mutes; a file that will not play is skipped with a note.
  Decoding runs on a worker (the newest request wins). Captions (the title for `title_secs` with fades, PAUSED, notes, "DROP TO PLAY") join the rule
  HUD overlay, small, in a corner, never over a card. `RE2_SONGS` queues files for scripted runs; a script presses `key:Space`/`key:ArrowRight`.
- **Pure parts in the library** (`listen.rs`, headless, tested): the `Analyzer` (2048-sample FFT at 22.05 kHz, 32 log bands 30 Hz-11 kHz against a
  slowly sinking loudest-so-far reference, eased; onsets from the rise of low-band energy against its own recent spread, at most four a second, and
  only when the low end carries a real share of the sound), `Playlist` (queue, wrap, shuffle per round), titles from tags or a tidied file name,
  `title_alpha`, the `playlist` parser.
- **The sky hears it**: `GlobalUniform.audio` (12 vec4) carries `Heard::uniform()`: bands, (level, bass, mid, treble), (beat, beats, song time,
  playing), (song seed, play seed, listening clock, 1). sky.wgsl names them `sky_band(i)`, `sky_level()`... `sky_song()` is stable per song (file name
  and length), `sky_play()` new every play. **With a playlist, `sky_time()` is the listening clock**, which stops while paused: Space freezes music and
  picture together without the engine knowing what the shader animates. All zero without a playlist, so no other sky changes.
- The playhead is counted per frame and steered toward the sound card's position only while that moves (a run faster than real time, or a device
  that consumes nothing, would otherwise pin it).
- `frame --song FILE --t S --play N` analyses the file up to that moment (deterministically) and draws it; `frame --seed N` draws another sky seed.

## Consequences
- A visualizer is a scene plus a shader: Chroma is one short scene and a 420-line WGSL file.
- A song is decoded whole into memory (stereo f32 + 22 kHz mono: about 100 MB for five minutes); fine for songs, not for hour-long mixes.
- Not verified on a person's machine: the picker on Windows, drag and drop on Windows. Opus files are not offered (symphonia has no opus decoder).
- Undo: remove `playlist` from `parse_audio`, the `audio` uniform and its sky.wgsl helpers, `listen.rs` and `bin/re2/listen.rs`, and symphonia from `gfx`.
