# 2026-10-05. An online player always reads the outcome, and split-screen limits are stated once
Status: accepted
Summary: The plain outcome banner stands in for any end card that is not shown (always online); the split-screen guest limits are one list that describe, the client and scripts all use.

## Context
A scene with a `ui` block can declare an end card per outcome. Offline, `sync_card` shows the card and the plain outcome banner is suppressed for that outcome (`rules_overlay`). Online (`net.is_some()`) `sync_card` returns early, so no card ever appears, but the banner was still suppressed because a card was *declared*: an online player of such a scene saw the counter at its final value and no result at all (reproduced in `tests/client_headless.rs`). Separately, the split-screen limit "loose props and the flashlight are player 1's" lived only in an ADR, in nothing an authoring AI reads.

## Decision
Cards are an offline presentation. `cards::outcome_banner_needed(online, has_end_card)` is `online || !has_end_card`: the banner stands in for the card wherever no card is shown. `splitscreen::GUEST_LIMITS` is the one list of what a guest cannot do (props, flashlight, online). `describe multiplayer` prints it, `re2 --players N` prints the limits the scene runs into (`guest_limits_for`: loose props via `physics::loose_props`, the flashlight), and a script step that makes a guest `interact` with a prop fails at once with the reason and `{"player": 1}`. `split_screen_guests_cannot_carry_props_and_the_engine_says_so` fails if guests are ever given props, forcing the list and docs to change with the behaviour. Not generalised: per-player prop ownership (a clean version needs the physics world to know a holder per local player, and the carried pose, the HUD prompt and the flashlight per player; none is verifiable here without real gamepads).

## Consequences
An online game gets its outcome from the plain banner whatever cards it declares. A two-player game built on carrying is told so at start-up and in the failing script, not by a player who cannot pick anything up. Not verified: that online outcome presentation is good (only that it is present), or anything with real gamepads.
