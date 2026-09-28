# 0056. Music composed in code
Status: accepted
Summary: Music composed in code: an eight-bar synthwave loop generated at start-up, tested for pitch, rhythm and a seamless wrap

## Context
Sound effects give a game feedback; a sustained musical bed gives it a pulse. The engine's rule since ADR 0008 is that nothing is imported: no sample files to license, ship or lose, so the
sounds are synthesized. A game that plays gunshots over silence feels like a tech demo, and a music file would break the rule. Music is harder to get right than a gunshot because it cannot be
looked at, but its notes and its rhythm can be measured.

## Decision
`music::loop_samples()` composes one loop, eight bars of driving synthwave in A minor at 132 BPM (Am F C G, Am F G E), and returns interleaved stereo at the audio sample rate. Layers: a kick
on every beat, a clap on two and four, offbeat hats, a rolling bass on the chord roots, a sixteenth-note arpeggio through the chord with a bouncing dotted-eighth echo, and a soft pad. Everything but
the drums ducks on each kick so the mix pumps, notes end with a few milliseconds of fade, and every tail is written round the end of the buffer, so the loop has no seam. The client generates it in a few
hundredths of a second at the start of a game and plays it on a repeating sink under the effects (`Audio::start_music`, volume 0.17); `N` turns it off and on, `RE2_MUSIC=0` starts without.

Tests measure what a piece of music is made of: the length is exactly eight bars at the tempo, the level is steady and never clips, the sustained layers are as smooth across the wrap as anywhere, a kick
lands on every beat and every bar repeats the drums, the bass sits on each bar's chord root and the arpeggio plays the notes of the chord (a Goertzel filter finds the notes; strangers are far weaker).

## Consequences
- The game has a pulse and no audio assets to ship; the loop is 14.5 s of samples in memory (about 5 MB).
- The taste is a human's: the tests keep it in tune, in time and free of clicks, not pleasant. The tempo, progression, patterns and mix levels are constants at the top of `music.rs`.
- Limits: one loop, no change with the action (it does not intensify in the last rung), no per-map music; it is mixed under the effects but not ducked by them.
