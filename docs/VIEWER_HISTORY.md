# The first-person viewer, as built for the prop hunt (history)

Red Engine 2 began as a JSON-scene renderer and the base for a prop hunt game: a human with a bat against Cheddar the rat, in a fenced house and yard. The offline MP4
renderer stayed as a fast way to look at scenes while authoring, and `examples/test_lab.json` replaced the prop hunt maps as the primary development map; `house`, `school`,
`office` and `store` are legacy reference maps kept as regression fixtures (ADR 0015). What follows is the viewer's description from that era. The numbers that can be
derived from the code (the protocol version) are kept current by `red_engine2 preflight`; the rest is history, so use `red_engine2 describe` and `search` for what the engine does today.

`re2` is a real-time, walk-around viewer for a scene: it opens a window, drops you in
at the scene camera's position, and lets you look around and walk through the room. It's
built on the same scene schema, mesh generation, and shading pipeline as the offline
renderer (see [`src/viewer.rs`](../src/viewer.rs)) — the difference is the camera is driven by
player input every frame instead of a keyframe track, and frames go straight to a window
instead of an MP4.

```bash
cargo run --release --bin re2 -- examples/prop_hunt_yard.json
```

On launch a menu asks whether to play the **Human** (an ordinary person with a bat) or **Cheddar the
rat** (small, brownish-grey, brisk, small enough to run under tables; no bat) — click a side or press
`1` / `2`. `--as human|rat` (or `RE2_CHARACTER`) skips the menu.

Controls: **WASD** or the **arrow keys** to walk, the **mouse** to look, **Shift** to sprint
forward (with a subtle FOV kick), **Space** for a small jump, **Ctrl** to crouch, **left-click**
to swing the bat, **F** to toggle borderless
fullscreen vs. maximized, **Q** to toggle first-/third-person, click the window to capture the
mouse, **Escape** to release it and open the pause menu (Resume / Quit game). The window launches maximized, fit to whichever monitor it
opens on.

Scenes with `vars` and `rules` are playable here, not only in the headless simulator. Offline `re2` runs the shared rule state
machine at 60 Hz: `hide`/`show` changes rendered object visibility, `collision` can open or close authored static geometry,
teleport and prop impulse effects reach the live world, and
pickup/drop/shot/hit/kill/respawn/swing/prop_hit events can trigger rules, and rules see loose props (`prop_enter`, `tilt(id)`, `reset`). A compact generic HUD shows scene variables, recent events and an `end` outcome.
Online uses the same presentation: protocol v<!--fact:protocol-->15<!--/fact--> repeats a bounded authoritative rule-state snapshot (up to 16 variables, 256 hidden
object indices and 64 collision-disabled objects, plus the recent event and outcome). Packet loss, reconnects and late joins recover current truth without replaying events.

This is a viewer, not an editor. The seeker's primary action on objects is **hitting them with
the bat**: a raycast from the player's eye against the objects' *real shapes* finds what is
within bat reach, the crosshair turns gold when something is in range, and only a swing that
actually connects plays a thunk and flashes the object (a swing through air is silent).

**The firearms (mouse wheel).** The human's primary weapon is the bat; **scroll the mouse wheel** to
draw the next of ten firearms (pistol, machine pistol, SMG, carbine, rifle, bullpup, marksman rifle, shotgun, LMG, scout rifle; scroll
past the last to get the bat back). **Left-click fires**: a hitscan shot along the crosshair with the
gun's own range, damage and cadence, a muzzle flash, recoil kick, a gunshot, a hit flash on whatever it strikes and a
punch for loose props. Ammo is **infinite by default**; a scene turns on limited ammo for every firearm with
`"weapons": {"ammo": {"loaded": 12, "capacity": 12, "reserve": 48}}` (**R** reloads). Cheddar has no weapons. Online, weapons,
damage, death/respawn and pick-ups run on the server (`sim::interact`).

**Loose props (E).** Both characters can pick up a small prop with **E** (the crosshair turns
green when you are looking at one you can lift), carry it in front of them, and drop it with **E**
again; a dropped prop keeps your momentum and falls, bounces, tumbles and knocks smaller things
over. A person carries chairs, crates, barrels, plants, TVs and everything smaller; Cheddar carries
things about the size of his head (apples, mugs, books) but can shove or bat-knock anything loose.
Walking into a small prop pushes it; a bat hit sends light props flying. While carrying, the human
cannot swing the bat. Physics is [rapier](https://rapier.rs) (see ADR 0012); props sit exactly where
the map put them until something disturbs them. (Right-click aims down the sights of a firearm.) Walls, furniture built from `box` primitives, and every `prop` (one
collider per prop's overall footprint, not per part) block movement (a simple
circle-vs-AABB push-out, axis-aligned); other primitive shapes and the `humanoid` rig don't
collide yet. Any keyframed objects in the scene still animate on their own clock while you walk
around, since only the camera is overridden.

Movement/collision/gravity run on a fixed 60Hz timestep decoupled from the render frame rate,
with the rendered frame interpolating between the last two completed physics states (see
`App::fixed_step_physics`/`App::update` in `src/bin/re2/frame.rs`) — frame-rate-independent and
resistant to tunneling through thin colliders under a frame-time spike. Rendering itself uses
4x MSAA and backface culling (every primitive mesh is a closed solid, verified by
`mesh::tests::all_primitives_are_ccw_front_facing`), plus per-mesh frustum culling against both
the camera's frustum and the shadow-casting light's frustum.
