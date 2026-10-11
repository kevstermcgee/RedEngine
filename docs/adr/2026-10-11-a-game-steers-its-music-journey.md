# 2026-10-11. A game steers its music journey
Status: accepted
Summary: `JourneyPlayer::prefer(scores)` draws the next sections from the scores a game names (a museum wing's), drops a waiting section that is not one of them, and crossfades as soon as the new section is rendered; the deck and its never-repeating sections are unchanged otherwise.

## Context
A journey (ADR 2026-10-11-music-that-never-repeats-an-audio-journey-of-freshly) deals its scores like a deck. A museum wants the music to follow the
wing the player walks into: a lonely vigil in the Wing of One Chair, sunlit harps in the Sunlit Wing, and to change soon after the player crosses into
another wing, not two minutes later.

## Decision
`Journey::prefer(Some(scores))` draws the next sections from those scores (never the same one twice running when there is a choice);
`JourneyPlayer::prefer(scores)` re-plans a waiting section that is not one of them and, when what plays is not one of them, moves on as soon as the
new section is rendered (crossfading over the journey's `fade`). Asking again for the same scores changes nothing. A museum theme lists its scores by
index into `audio.journey.scores` (`themes[].music`, checked against the journey's length).

## Consequences
- Music follows place without a second music system: sections are still rendered with fresh seeds in moving keys, so a wing's music never repeats.
- A change of wing costs one section render (seconds, on the journey's worker) before the crossfade starts.
