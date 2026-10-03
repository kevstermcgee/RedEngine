# 2026-10-03. Object-based player actions for sim and the scripted client
Status: accepted
Summary: approach, look_at and interact take an object id in both checks.sim scenarios and the playtest script, with one definition of done and of failure.

## Context
Moonlight Delivery's author (`docs/analysis/2026-10-02-moonlight-delivery-feedback.md`, item 2) could aim a `checks.sim` scenario at world coordinates
(`walk`, `hold {look_at: [x,y,z]}`), but the graphical script (`playscript`) only had keys and angles, so each pick-up needed a hand-computed
move duration and camera angle, and the first attempts missed. The two ways of driving a player shared no vocabulary, so a scenario proven in `sim`
could not be replayed in the real client without being rewritten, and a miss said nothing about why.

## Decision
- Three steps exist in both: `approach: "id"` (optional `within` metres, `timeout` seconds), `look_at: "id"` and `interact: "id"`. The id is a top-level
  object; a typo is a parse error with a did-you-mean in `sim`, and a named failure in the client (whose scene is only known at run time).
- One module defines what they mean, `sim::approach` (pure: no sim, no client): the player's horizontal gap to the object's footprint
  (`Target::gap`), the default `within` (60% of the body's pickup reach, so the eye-to-object ray an interact casts is well inside it), the aim
  (`aim`), and `Approach`, a tracker that says Arrived, Moving or Failed (stuck: no 0.2 m progress in 2 s; timeout: 12 s unless set). Both
  runners call it; the failure sentence (`failure_message`) is shared too, so it reads the same wherever it fails: the object, where the player
  stopped, how far from it, and what to do.
- `interact: "id"` is approach, face (two settle ticks), press, then a check that a loose prop is carried. A miss quotes the pick-up reason the
  simulation already records (`pickup_misses`: too big, too far, nothing aimed at). Hands already full fail early, since E would drop instead.
  A non-prop target (a button a rule watches) is just approached, faced and pressed.
- In `sim`, targets come from the scene's bounds, loose props from the live physics world. In the client, `Driver` gains `pose`, `locate` and
  `carrying` (default: unknown, so existing drivers are untouched); `locate` reads the client's own scene copy, into which a loose prop's pose is
  written every frame both offline and online, so it is where the prop is now. `carrying` is known offline only: online, the holder is the
  server's, so `interact: "id"` there presses and does not check.
- Old forms are unchanged: `interact: true` still taps E, `hold` and `walk` work as before.

## Consequences
- A pick-up reads `{"interact": "parcel_1"}` in `checks.sim` and in `re2 --script` / `playtest`; the same scene is exercised both ways
  (`tests/object_actions.rs`, `tests/client_headless.rs`).
- Steering is a straight line (like `walk`), so a wall between player and object makes `approach` fail as stuck. The message says to `walk` round
  it first; planning a route (`pathing::plan_route`) would need the analysis tools, which the headless simulation does not depend on, and is not done.
- Targets are single top-level objects, boxes by bounds: a child of a group or a very odd shape cannot be addressed by id.
- The client's `interact` check is unavailable online; that is a protocol question (expose the holder to the client), not a script one.
- Not covered: stairs and other floors (the gap is horizontal), and a moving target outrunning the player (it is re-located every frame, so a prop
  being carried or rolled is chased, and times out if it escapes).
