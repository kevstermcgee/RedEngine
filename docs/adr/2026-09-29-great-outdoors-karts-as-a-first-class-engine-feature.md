# 2026-09-29. Great Outdoors: karts as a first-class engine feature
Status: accepted
Summary: A kart is a movement mode of the existing player: a pure kart step shared by server and prediction, per-animal specs as data, per-player race progress and three pickups in the simulation; input reuses the player input packet.

## Context
"Great Outdoors" is a hosted kart racer (fairytale animals, pastel palette, one map, eight drivers: Duck, Bunny, Deer, Coyote, Hawk, Bear, Wolf, Beaver; each
has its own kart and perks; three pickups; a third-person view). The engine has none of it: no vehicles, laps or checkpoints (`search` finds a bicycle prop
and nothing else); rules-as-data variables are global, so per-player race progress cannot be written as rules (`describe rules`); the six bodies
(`player::Character`) are people and a rat. What it does have is the right seam: one pure movement function, `sim::player::step_player_tuned`, is called by
`MatchSim::tick_once` (server), `net::predict::Predictor` (client prediction and reconciliation) and the bots' rollouts, so a new movement mode added
behind that one call is automatically identical on server, client and tools. Input is already a compact struct (`PlayerInput`: `forward` and `strafe` as
signed or analog axes, `jump`, `attack`, `interact`, `sprint`, `crouch`) and state is `PlayerState` plus the replicated `PlayerSnap`. The lobby, countdown,
rounds and results flow already exists (ADR 0029). Collision is 2D boxes with a circle pushed out of them (`collide::resolve_collision`) over ground heights
and terrain. Measured headroom on the 4-core hosting box: an 8-player server tick is about 4% of the 16.7 ms budget (`benches/history/build-times.json`).

## Decision
Karts are a movement mode of the existing player, not a second game engine.
- **Kart step.** `sim::kart::step_kart(state, kart, input, spec, colliders, ground) -> speed`, pure, fixed 60 Hz, called where `step_player_tuned` is
  called when the scene has a `race` block. Steering is `strafe`, throttle and brake are `forward`, `jump` is hop/drift, `attack` uses the held pickup,
  `interact` is the driver's signature ability. No new input fields: the input packet does not change. Heading is `PlayerState.yaw`; `velocity` is the real
  motion, so a drift is velocity pointing away from heading. Extra per-kart state that must be replicated and predicted lives in a small `KartState`
  (boost ticks, drift charge, spin-out ticks, shield ticks, held pickup, ability cooldown) added to `PlayerSnap` (**protocol v11**).
- **Specs are data.** `KartSpec` rows (like `FirearmSpec`), one per driver: top speed, acceleration, braking, steer rate, grip, mass, off-road multiplier,
  boost strength, drift charge rate, plus a signature ability id. The scene may override numbers (`race.karts`); the defaults below are a starting point to tune
  by playing and by `perf`, not a promise.
- **Eight drivers, eight bodies.** `Character` grows Duck, Bunny, Deer, Coyote, Hawk, Bear, Wolf, Beaver (procedural models in `characters.rs`, each with its
  own kart, in the pastel palette). One animal per player in a race (the lobby's character choice already exists).
- **Race progress is native.** `sim::race` (pure, unit-tested, in the match checksum): the scene's `race` block lists laps, ordered checkpoint zones and the
  start grid; per player it tracks next checkpoint, lap, finish tick and live position (lap, checkpoint, distance to the next gate). A wrong-way or skipped
  gate does not count. Results feed the existing round flow. The generic rules keep working beside it (boost pads, teleports on a fall).
- **Three pickups**, from item boxes that respawn: **Mushroom** (speed burst), **Acorn** (a thrown nut that spins out the first kart it hits) and **Bubble**
  (absorbs the next hit). A fixed-size entity pool in the simulation (projectiles and hazards) so the per-tick allocation budget test still holds.
- **Beaver builds.** His kart is wooden (heavy, sturdy, mud and water do not slow it) and his signature is **Build**: `interact` lays a short plank barrier
  behind the kart (at most two at a time, cooldown), the same hazard entity a thrown Acorn is stopped by. Stretch goal: a plank bridge over the map's one
  stream shortcut that only he can lay.
- **Kart bots are in scope** (the user asked for them once the plan was drawn). A driver brain in `sim::ai` produces the same `PlayerInput` a human does and
  drives through the same `step_kart`, so the server, `sim` and `perf` treat bots and people alike. It follows a racing line built from the `race`
  checkpoints (plus optional `line` points the track author adds for tight corners), looks ahead to steer, lifts off the throttle for sharp turns and uses
  its pickup sensibly (Mushroom on a straight, Acorn when a kart is ahead in view, Bubble when one is behind). Difficulty is a per-bot skill number (top
  speed fraction, steering noise, reaction ticks). `bots.fill` tops a race up to eight, and a bot may not take an animal a human chose. Bots are also the
  test rig: an eight-bot race proves the track can be finished, and lap time per animal is the balance measurement recorded in `flows.json`.
- **Camera and client.** A third-person chase camera (`ViewCamera` through the `app` layer, ADR 0043) with a speed-based follow distance; a HUD for lap,
  position and speed; gamepad steering through `controller`.
- **Each phase is measured.** `benches/flow_bench.py` gets a flow per phase (build, check, sim scenarios, `perf` with 8 karts, `net-test`), so what each step
  costs in time and output is recorded as the game is made; friction found on the way goes in `docs/analysis/` for the engine's benefit.

Initial driver table (m/s, m/s^2; tune later):

| Animal | Top | Accel | Steer | Grip | Mass | Off-road | Signature |
|---|---|---|---|---|---|---|---|
| Duck | 24 | 12 | 95 | 0.80 | 1.0 | 0.85 | floats: puddles and shallows do not slow it |
| Bunny | 22 | 16 | 110 | 0.75 | 0.8 | 0.85 | hop over small hazards; hop-drifts charge faster |
| Deer | 27 | 10 | 85 | 0.80 | 0.9 | 0.85 | drifts charge 30% faster |
| Coyote | 25 | 12 | 90 | 0.70 | 0.9 | 1.00 | no off-road penalty: the dirt shortcuts are his |
| Hawk | 25 | 11 | 90 | 0.80 | 0.7 | 0.85 | glide: hold jump in the air off a ramp for lift and air steering |
| Bear | 22 | 9 | 70 | 0.85 | 1.6 | 0.85 | bulldoze: bumps spin others out, is not spun himself |
| Wolf | 25 | 12 | 92 | 0.78 | 1.0 | 0.85 | slipstream: drafting behind a kart charges a boost |
| Beaver | 21 | 9 | 78 | 0.85 | 1.3 | 1.00 | wooden kart, mud and water immune; Build (planks) |

## Consequences
The server and the prediction stay one code path, so a kart is identical online and offline and `sim` can prove a lap. The wire protocol changes once (v11):
every client and server must be rebuilt from the same commit, and the shared server for a game must be built from the commit its clients were (ADR
2026-09-29-develop-on-a-path-ship-on-a-pin). A movement mode behind one call keeps the first-person games untouched. Costs: eight new bodies and karts, a
chase camera, a new pure module for karts and one for races, and the largest single protocol addition since prediction. Kart bots add one AI module, and the
first-person bots keep their own brain. Not decided here: exact numbers (they come from playing), the track's layout, sound, and whether pickups get a fourth item.
