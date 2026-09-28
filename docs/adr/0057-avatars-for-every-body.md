# 0057. The avatar pool covers every body somebody can wear
Status: accepted

## Context
A client draws the other players with avatars taken from a pool of hidden bodies that is built before the renderer exists (the renderer takes its meshes from the scene at creation, ADR 0052). The
pool held avatars for the bodies a *human* could pick, and a scene that forces a body (`player.character`) got only that one. Bots do not follow that rule: a roster gives each bot a body of its own
(cowboy, wizard, alien, robot) whatever the humans are forced to. Trigger Happy forces the Human and fields a roster, so every bot was a remote player with no avatar to wear and the client skipped
it: the game shipped with enemies that shot at you and could not be seen, nor turn the crosshair red, nor show a gun or a tracer. Every test had used a Human as the opponent, so none noticed.

## Decision
`NetSession::add_avatar_pool` allocates, per body: `MAX_PLAYERS_PER_SNAPSHOT` avatars if a human can wear it (any body when nothing is forced, else the forced one), `BOT_BODY_POOL` (4) if only a bot can
(every body but the rat, when the scene's `bots.fill` is above zero), none otherwise. `update_scene` claims through `claim_avatar`: a player keeps their avatar while it is their body, else takes a free
one of their body, else a stand-in of the same rig (a server can fill the match with bots the map never mentions, `red_server --fill`): another costume is better than an enemy nobody can see. A
player who changes body hides the avatar they leave.

Tests: a forced-Human scene whose roster has a wizard, a cowboy and a robot draws all three, each in its own costume; a match filled by the server on a scene that names no bots draws them in
stand-in bodies, none taking a second; the pool sizes in each case (nothing forced, Human forced with and without bots, Rat forced with bots).

## Consequences
- A game with bots in bodies of their own carries 8 + 4 x 4 avatars instead of 8 (a few hundred more hidden draws a frame); a game without bots, or with nothing forced, costs what it did.
- The pool is still sized before the renderer exists, so it is a bound, not a promise: the fifth wearer of a bot body, or a body outside the rule, gets a stand-in.
- Lesson kept in the tests: an opponent in a test must wear the body the game gives its opponents, not the convenient one.
