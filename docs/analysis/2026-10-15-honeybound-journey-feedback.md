# What the Honeybound update taught us about Red (2026-10-15)

The ask (the owner, 2026-10-15): update Honeybound to the latest engine; a simple hero's-journey story with a concrete call to action; a primary
antagonist and boss battles that make sense in the world; super simple, no-choice interactions with characters; subtly better, distinct graphics (it
looked too much like Mario); a game that feels fantastic and gets gradually harder; villains and obstacles that belong to the world; more unique, named
music that fits each part, downloadable from the pause menu while it plays; designed for a controller on PC; stable saves; a pause menu with save and
close (keyboard instructions in a controller game feel awkward); a clean game-over message; a desktop, title-bar and taskbar icon; `F` toggles
fullscreen in **every** game; the hero is Pip again.

The result: Honeybound v2, *Pip and the Heartcomb* (`examples/2d/honeybound/`), 32 stages, eight bosses, an 18-song titled soundtrack, a
controller-first pause menu and title, autosaves and Continue, an icon everywhere. This file is the issue log: what the engine could not do at the start
(and was added in the same PR), and what is still missing, ranked by what it would save the next game of this kind.

## The short version

* **The 2D engine could not make a controller game.** `describe 2d` said "a gamepad's stick/d-pad and A/B/Start map to the same actions", while
  `capabilities` said a gamepad was only "prepared" and `src/play2d.rs` had no gamepad code at all. Two claims disagreed and the code agreed with
  neither. Added: the native player reads the first pad (`controller::Controller`, the 3D client's reader) and feeds only *changes* into the game's
  actions, so keyboard and pad work side by side; a `gamepad` built-in variable lets a game show pad buttons or keys.
* **A pause menu could not be built.** The only way to stop the world was an open dialogue, and an open dialogue swallows every press, so a menu
  that a controller navigates (up/down/A/B) was impossible. Added top-level `pause_if` (the world stands still, press rules still run) and the
  actions a menu needs: `window: "fullscreen" | "close"`, `save: true`, `export_music: true`.
* **A latent timer bug:** the game clock (`time`, `every`, `after`, cooldowns) kept running while a dialogue paused the world, so after a long
  conversation every `every` rule fired once per tick until it caught up (a burst). The clock now stands still with the world.
* **The 2D renderer had a flat background and no pictures in the HUD.** Added a level `sky` gradient, parallax `backdrop` bands, a HUD `image`
  widget and the game `icon` (window title bar and taskbar; the installer reads the same sprite for the desktop shortcut).
* **F was not fullscreen everywhere.** The 3D client and Killchain had F/F11; the 2D player had only F11, the `app::shell` window loop (Workshop,
  external projects) had neither, and Idea Forge bound F to "save idea". Now F/F11 toggle fullscreen in `play2d`/`re2d` and `app::shell`, and
  `validate` refuses a 2D button bound to KeyF/F11 (Idea Forge moved to L).

## What was added to the engine (all in this PR, each with tests)

| Piece | Where | Why |
|---|---|---|
| Gamepad in the native 2D player | `src/play2d.rs` (`pad_actions`, `Controller::connected`) | stick/d-pad, A = action, X/B/RT = secondary, Start/Back = pause |
| `gamepad` built-in | `red2d::sim` | show pad glyphs to pad players, keys to keyboard players |
| `pause_if` | `red2d::game`, `sim::step` | a pause menu made of rules |
| the clock stops while paused | `sim::step` | no timer burst after a pause or a dialogue |
| `window`, `save`, `export_music` actions + `music_exported`/`music_not_exported` events | `red2d::game`, `sim::Request`, `host::take_requests/notify`, `play2d::serve_requests` | close/fullscreen from a menu; save now; the playing song to `~/Music/<Game>/<Title>.wav` |
| score `title`, `{music_title}` | `red_core::score`, `render::music_title` | named songs in a pause menu and in exported files |
| `glide` / `glide_time` in platformer feel | `sim::move_one` | Pip's flutter (hold jump while falling) |
| level `sky` + `backdrop` | `render::draw_sky/draw_backdrop` | depth without hand-placed scenery |
| `image` widget | `render` | HUD icons, pad buttons |
| `icon` | `host::icon_rgba`, `play2d` (window + Windows taskbar icon) | one icon for window, taskbar and shortcut (RedEngineGames `release_tool.game_icon`) |
| `view.camera.min_x/max_x` expressions | `sim::camera_target` | a boss arena locks the view |
| F/F11 everywhere, KeyF refused as a 2D button key | `play2d`, `app::shell`, `game::parse_widget` | the owner's rule for all games |
| 2D gamepad is "supported" in `capabilities` | `red2d::caps` | the claim now matches the code |

## Hurdles, ranked by what fixing them would save the next game

1. **A boss is ~25 rules of polling.** Each boss is a state machine written as `every: 0.05` rules with `if: "bs == 3 && time > bt + 2.2"`,
   one rule per transition, plus `become` rules duplicated per boss kind because `become.prefab` cannot be an expression. Eight bosses cost ~150
   rules and the per-tick polling is most of the rule time. *Want:* a per-entity state machine (`"states": {"walk": {"after": 2.4, "then": "hop",
   "do": [...]}, ...}` on a prefab, with `on_hit`/`on_land` transitions and the entity's own timer), or at least `become` with a `{"pick": ...}`
   prefab list and an entity-local timer variable (`self_age` exists as `age`, but not as a variable).
2. **Rules are global; levels are not.** Per-stage numbers (the honey room's exit x, a boss's den position) had to be written as sums of
   `(stage == k) * value` or read from hidden marker entities with ids (`den_l_x`). *Want:* level-scoped constants (`"levels":[{"vars":{...}}]`)
   that are set when the level loads.
3. **Dialogue state defaults to "open".** A dialogue's `<id>_line` starts at 0, so every conversation is on screen when the game starts unless a
   `start` rule closes it (the old Honeybound carried the same workaround). *Want:* dialogues closed by default (`"open": true` to start open).
4. **The solver is slow on long levels and greedy in boss fights.** Each candidate clones the whole `Sim`, including every map tile entity (thousands
   per stage), so a 212-column stage takes minutes at beam 48; in a boss den the beam collapses onto one region and every candidate dies. The
   per-stage progress expression had to be tuned by hand. *Want:* map tiles out of the cloned entity list (static, shared through the `Arc`), and
   a survival-aware beam (diversity by phase/var, not only by player position). Moving platforms are its worst case: walking off a raft's end
   scores as "furthest right" for the whole fall, so doomed candidates fill the beam and the patient ride-then-jump is never kept (2-3 needed a
   scripted waypoint, several high stages beam 320). A doomed-state test (`--avoid "... || p_y > ground"`) did not help; ranking by a
   lookahead (still alive N ticks later) or by time-to-goal estimates would. Hazards with a phase (a wasp's patrol) have the same problem: dedup by
   player position cannot tell "wait for the wasp" from "go now".
5. **`sim --every` prints every variable.** With ~60 variables a trace line is unreadable. *Want:* `--vars p_x,mode,bs`.
6. **`scripts/dev seed --from DIR` silently copied 0 entries** when DIR was a checkout rather than its `target/` (it printed "copied 0 ...
   in 0.0 s" and exit 0). *Want:* accept either, or fail with the way out.
7. **The 2D reference budget (9 000 bytes) forces trimming on every addition.** Adding these features meant shortening older lines to stay under it.
   The budget is right; consider moving examples (`describe 2d examples`) out of the always-read page.
8. **Sprites are capped at 64 px**, which is fine for characters but makes a parallax band a row of small pictures; a final boss of 48x40 was the
   largest that fit. A band `scale` (added) helps; larger backdrop images would help more.
9. **No sprite tinting/flash.** A hit boss blinks by swapping to a prefab whose sprite alternates with an empty frame; a `flash` action (tint
   white for N ms) would remove one prefab and sprite per boss.
10. **Music has no per-note length or accidentals.** A score is scale degrees on a grid with one `hold` per track; melodies with held notes or a
    chromatic passing note have to be faked. The `audio report` was very useful to check loudness and seams without ears; it also showed the
    instruments sit almost entirely below 2 kHz (highmid 0-8%): an EQ/brightness hint in `audio check` would catch dull mixes.

11. **`become` into a solid tunnels.** A boss hanging flush against the ceiling as a non-colliding prefab, turned into a colliding one to drop,
    was pushed up through the ceiling and out of the arena (invisible, unhittable, the fight unwinnable). `patrol` and velocity-driven movers with
    `collide: []` likewise drift through walls (a boss's patrol home is wherever it last `become`s). *Want:* `become` that keeps a thing out of
    solids it now collides with (resolve toward where it came from), and a `patrol` that can be bounded by world x (`"between": [x0, x1]`).
    Found only by rendering frames of the solver's partial runs; a `verify` warning "a `boss`-tagged thing left the world/arena" would have caught it.
12. **Killing one solver run from a shell.** `pkill -f`/`pgrep -f` with the level id matched the agent's own shell command line and killed it
    (twice). `red_engine2 solve` could write its pid, or `dev.py`-style drivers could run each attempt under `timeout` with a progress line.

## Not proven

Real controller in hand (the mapping is unit-tested; the pad reader is the 3D client's), the window and taskbar icon on Windows (built through
winit's `with_window_icon` / `with_taskbar_icon`, not seen), the installer shortcut icon on Windows (the .ico is tested byte-wise), how the
music sounds to people, and whether the difficulty curve feels right (every stage and boss is proven *finishable* by the solver, not *fair*).
