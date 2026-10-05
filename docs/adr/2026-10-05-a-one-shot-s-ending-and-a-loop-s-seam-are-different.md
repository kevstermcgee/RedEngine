# 2026-10-05. A one-shot's ending and a loop's seam are different measurements
Status: accepted
Summary: audio analysis gains end_step (the click a hard stop makes) for one-shots and keeps seam for loops, so a truncated one-shot with similar start and end levels can no longer pass.

## Context
`audio_analysis::problems` judged a one-shot "cut off" with `end_dbfs > -50 && seam > 6`, where `seam` is the loop measurement: the jump from the last frame back to the FIRST. Joining the end to the start is the wrong question for a sound that plays once. A cosine of whole cycles starts and ends on its peak: its seam is 0 and it was passed as a good one-shot while it stops dead at full level. A sound with a hard attack that decays to -45 dBFS was called cut off because its end is far from where it began.

## Decision
`Report` gains `end_step`: the last frame's magnitude (the largest channel) as a multiple of the clip's typical frame-to-frame step, which is the jump the speaker makes when the clip stops and the output returns to zero. It looks at the end alone. `seam` stays the loop measurement and is documented as meaningless for a one-shot; `end_step` as meaningless for a loop. Both use one named limit, `JUMP_LIMIT = 6` (what the signal itself does between neighbouring frames stays within a few steps, so 6 is clearly an outlier a listener hears as a tick); it was not tuned to fixtures. `audio report` prints `endjump`, `describe audio` says which is which. Fixtures cover a good fade, a truncation, a seamless loop, a bad seam, a cosine whose start and end amplitudes match (loop-perfect, one-shot-broken, both asserted), a hard attack decaying to -45 dBFS, silence, NaN and infinite samples, a constant level that stops, stereo, and clips of 0 to 3 frames. With the old rule restored the cosine and the -45 dBFS fixtures fail.

## Consequences
All 80 built-in sounds still pass; the only ones with a large `endjump` are loops (`score.ambient_drift` 15.4, `ambience.map` 11.5), which have clean seams (0.7, 1.3), the separation working. What this cannot say: whether a sound is pleasant, or whether the engine's own 2 ms playback fade hides a click a measurement finds.
