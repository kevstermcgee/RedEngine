# Chroma

Music you can see: choose songs from your computer (or drop them on the window) and watch them become light. Every song has its own look, and every listen is different.

Choose music with the button on the start card, O at any time, or by dragging one or many files onto the window (mp3, flac, ogg, wav, m4a). Space pauses the music and the picture together; the arrow keys skip; N mutes. The song's title shows for ten seconds in the bottom-left corner when it starts; otherwise the screen is only the picture.

```bash
re2 examples/chroma/chroma.json                                                      # play (the picker opens when you start)
RE2_SONGS=a.mp3:b.flac re2 examples/chroma/chroma.json                               # start with these songs queued
red_engine2 frame examples/chroma/chroma.json out/c.png --song a.mp3 --t 30 --play 7 # one moment of a song, as play 7 would show it
```

What it is made of (all of it the engine's; the only code is the sky, which the engine checks for every GPU backend when the scene is validated):

- `chroma.json`: a still player, a clean screen, a start card, `audio.playlist` (the player's own music, ADR 2026-10-11-the-player-s-own-music-a-playlist-the-game-hears-and-a) and the sky.
- `sky/chroma.wgsl`: eight modes (halo spectrum, kaleidoscope, tunnel, light waves, liquid blobs, neon grid with spectrum mountains, starburst, nested polygons) drawn in screen space and fed by `sky_band`, `sky_bass`, `sky_beat`, ... A listen is a run of scenes of 14 to 30 seconds that crossfade. The song (`sky_song()`) picks the palette family and three favourite modes it keeps returning to; each play (`sky_play()`) orders the scenes, shifts the colours and draws every shape parameter, so a song looks like itself and never the same twice.

Chroma ships no audio: the songs are yours, read from your disk while you listen.
