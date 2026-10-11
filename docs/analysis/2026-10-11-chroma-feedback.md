# What building Chroma taught us about Red (2026-10-11)

The ask: *"Create a music visualizer with this engine. A super simple file picker to import one or multiple audio files; a variety of
visualizations (shapes, colours, movements); songs should have their own unique visualizations, and listening to the same song should look
different each time. Hide the HUD, show the playing song for 10 seconds in a discreet spot. Short catchy name. Publish, and give feedback to the
engine."* Added while building: drag and drop for one or many files, and Space to pause the music and the animation together.

The result is `examples/chroma` (a scene and one WGSL sky), published through `games-publish.json`. This is the issue log, built straight after
Wormhole, which had added game skies (`sky.shader`).

## The short version

* **The engine could not hear a file or show one to a shader.** No decoder (ADR 0008: games ship procedural sound), no picker in a game, no drag and
  drop, no analysis at a playhead, no path from sound to a sky (Wormhole's finding 2). Added **`audio.playlist`** with symphonia decoding under `gfx`,
  the picker (rfd, already a dependency), `DroppedFile` batches, an `Analyzer` (32 bands, levels, beats), 12 uniform vec4s and `sky_band()`-style
  helpers, captions in the rule overlay, and a listening clock that `sky_time()` follows so Space freezes picture and music together (ADR
  2026-10-11-the-player-s-own-music-a-playlist-the-game-hears-and-a).
* **Game skies paid off at once.** The visualizer itself is a shader in a scene: eight modes, scenes that crossfade, per-song and per-play seeds,
  all written and tuned with `frame` (now with `--song FILE --t S --play N` and `--seed N`). No game Rust.
* **Headless checking found two real bugs in my own code** before any person ran it: the playhead pinned at 0 (a run faster than real time reads a
  sound-card position that does not move) and a beat detector that fired on a steady tone. Both are now tests.

## What was built

| | |
|---|---|
| Picker / drop | O or the start card opens a multi-select picker (mp3 flac ogg wav m4a); files dropped on the window (any number, any screen) queue after the current song in the order they came, "DROP TO PLAY" while hovering; files that are not music are skipped with a note |
| Playback | one song at a time on the sound card, next at the end, round to the first; Right/Left skip (Left restarts a song that is over 3 s in); Space pauses both; N mutes |
| Title | tags ("Artist - Title") or the tidied file name, bottom-left, fades in and out over 10 s; "PAUSED (SPACE)" top-right; HUD hidden |
| Visuals | halo spectrum (bars or outline, folds), kaleidoscope (warped noise, n wedges), tunnel, light waves, liquid blobs, neon grid with spectrum mountains and a sun, starburst with beat shock rings, nested polygons; 14-30 s scenes crossfading; the song picks the palette family and 3 favourite modes; the play orders scenes, shifts colours and draws every parameter |
| Checked | unit tests (bands follow pitch, a 124 bpm kick is counted within 20-25 of 24, a steady tone is not a beat, the same moment always analyses the same, playlist order/wrap/add/shuffle, titles, a WAV decodes, a bad file says which and why); the sky passes naga + HLSL; the live client headless with `RE2_SONGS` and scripted `key:Space` / `key:ArrowRight`: mp3 with tags and ogg without, 9 beats in the first 4 s, pause holds the playhead and the clock, title gone after 10 s, the next song titled; frames of 2 songs x 3 plays at one moment are six different pictures |
| Not verified | **the picker and drag and drop in a real window** (no way to drive either here: no xdotool, and Xvfb has no file manager); Windows; real-GPU frame rate; how it feels with real music (only two synthetic songs here) |

## Ranked findings still open

### 1. Nothing can drive a native dialog or a drag and drop in a test  *(highest)*
The two features the player touches first are the two that no check here exercises: `re2 --headless` has no window, and the windowed client under
Xvfb cannot receive a drop or click a GTK/portal dialog. Engine should: a `RE2_FAKE_PICK=files` / `RE2_FAKE_DROP=files` hook the client reads where it
would open the dialog or receive the event, so a script reaches the same code path, plus a note in `describe playtest`.

### 2. The sky has no screen space
A visualizer wants the screen, not the sky dome. Chroma rebuilds the camera's axes from three `view_ray` calls per pixel to project `dir` back onto
the screen. Engine should: pass `sky(dir, px, uv)` (or a `sky_uv()` helper) with the pixel's screen position, and a scene switch that pins the
camera for a visualizer (mouse look does nothing useful here; `still` only stops movement).

### 3. Expensive skies are still unmeasured
Chroma's kaleidoscope runs two five-octave fbm per pixel, twice during a crossfade. Same as Wormhole finding 1: a render scale for the background
pass and a cost estimate in `validate` are the fix; neither exists, and this box has no GPU to measure on.

### 4. The overlay font is all capitals in a 5x7 bitmap
Song titles are names: "Sigur Rós - Hoppípolla" lost its accents and case (an unknown glyph draws as a blank). Chroma now folds accented Latin
letters to plain ones (`listen::ascii_fold`), which reads but is not right, and other scripts (Japanese, Cyrillic) still show gaps. Engine should: a
font with lower case and at least Latin-1, for player-supplied text.

### 5. A game cannot remember what the player chose
Chroma forgets the queue on exit. `persist` is for rule numbers; there is no per-game place for a list of paths. Engine should: a small persisted
string list (`settings` has a JSON file per game already).

### 6. Smaller
* The start card's button starts the game; it cannot open the picker itself (the client opens it on start instead). A card button with an action
  would be clearer.
* Score seeds: still Wormhole's finding 4 (close seeds, same first notes).
* `Audio` grows one-off sinks (music, layers, song): a small mixer-owned "music bus" object would be cleaner than a fourth kind.

## Cost of this run
Engine changes: about 1.4k lines (listen.rs analysis, playlist and decoding; the client half; audio uniform; frame flags; script keys; tests; docs;
an ADR). Game: a 20-line scene and a 420-line shader. Most of the time went into the client's I/O and the two headless-found bugs; the look took
four contact sheets.
