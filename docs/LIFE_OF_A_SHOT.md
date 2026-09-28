# Life of a shot

One trigger pull, in the order it happens: the click, what the client shows at once, the input on the wire, the server's verdict, the snapshot that carries the news back, and what
the shooter, the victim and everybody else see and hear. Each step names the function to open, the number or scene key that tunes it, **How to see it** (a command, test or state-dump
field that proves the step) and **Fails silently when** (the classic way it goes wrong with no error, and what reports it, if anything does). Paths are from the repository root;
`file.rs::f` is function `f` (a method too) in that file. The other half of a fight is `docs/LIFE_OF_A_REMOTE_PLAYER.md`. Decisions behind it: ADR 0016, 0022, 0050, 0051, 0052, 0053,
0055, 2026-09-28-remove-the-revolver and 2026-09-28-seeing-what-the-player-sees. The commands and scripts below were composed from the code, not run for this page: adapt weapon and roster to your map.

```
click        client tick T     attack_queued, then the 3-tick pulse net_pulse[1]              (fixed_step_combat)
predicted    client tick T+1   input{attack} goes out (tick T for an automatic gun); flash, recoil, sound, tracer at once   (predict_attack)
wire         InputPacket       the newest 4 inputs: seq, flags (bit 4 = attack), moves, yaw, pitch
verdict      server tick S     push_input -> queue -> tick_once -> handle_actions -> attack -> probe_lagged -> damage
news         next snapshot     every visible PlayerSnap (shots, hp, flags, weapon) + your Feedback {hits, hurt, kills, bearing, respawn}
shooter      Watcher -> Happenings -> Feel   hit or kill cue, marker, rung notice; the victim gets hurt cue, damage arc, ELIMINATED
bystanders   the same counters, 0.1 s later   Cue::Shot: sound, tracer, spark; the shooter's avatar recoils and flashes; the scoreboard moves
```

## Watch a shot without a window

```
red_engine2 playtest MAP --fill 4 --script out/shot.json     # hosts a match, plays the script in the real client, prints a verdict, exit 1 when an expect fails
re2 MAP --host --fill 4 --headless --script out/shot.json --dump out/state.json   # the same loop; a GPU only if the script takes a `shot`
red_engine2 sim examples/test_lab.json --scenario out/duel.json   # the server's half alone: no window, no socket, deterministic
```

A firearm is needed: on a map whose humans start with the bat (Test Lab) one wheel notch is the pistol (`{"switch": 1}`); on a `weapons.ladder` the wheel does nothing. Script syntax and the
pointers of the state dump (`re2-dump/1`): `red_engine2 describe playtest`. `out/shot.json` (`/cues/counts/Shot` counts your shots and other players'):

```json
{"policy": "idle", "steps": [
  {"wait_for": {"at": "/online/in_round", "eq": true, "within": 30}}, {"switch": 1}, {"wait": 0.6},
  {"wait_for": {"at": "/remote/in_view", "min": 1, "within": 30, "msg": "nobody to shoot at"}},
  {"aim_at": "nearest"}, {"fire": {"clicks": 8, "every": 0.4, "track": true}},
  {"expect": {"at": "/cues/counts/Shot", "min": 1, "msg": "the click made no shot cue"}},
  {"expect": {"at": "/cues/counts/Hit", "min": 1, "within": 5, "msg": "the server never reported a hit"}}]}
```

`out/duel.json`, the server's half (the body of `tests/interactions.rs::a_scenario_can_fight_with_the_hold_step`):

```json
{"name": "pistol duel", "spawn_group": "duel", "players": [{"id": "a", "spawn": "spawn_a"}, {"id": "b", "spawn": "spawn_b"}],
 "script": [{"player": "a", "hold": {"switch": true, "seconds": 0.05}}, {"player": "a", "wait": 0.5}, {"player": "a", "hold": {"attack": true, "seconds": 0.05}},
            {"player": "a", "wait": 0.6}, {"player": "a", "hold": {"attack": true, "seconds": 0.05}}, {"player": "a", "wait": 0.6}],
 "expect": [{"event": "shot", "count": 2}, {"event": "hit", "count": 2}, {"no_event": "kill"}]}
```

## 1. The click

The handler only queues it. The left button with the mouse captured sets `attack_queued` (one shot) and `attack_held` (the level automatic weapons read); a pad's FIRE sets `attack_queued`
and arms the online pulse at once.

- **Open**: `src/bin/re2/events.rs::window_event` (the left `MouseInput` arms), `src/bin/re2/controller.rs::poll_controller`, `src/bin/re2/window.rs::set_grab`.
- **Tuned by**: `Weapon::automatic` in `src/weapons.rs` (whether holding repeats). Nothing else: input is raw.
- **How to see it**: a script step `{"fire": 1}` sets the same flags (`Driver::fire`, `src/bin/re2/headless.rs`); then `/cues/counts/Shot` in the state dump rises.
- **Fails silently when**: the mouse is not captured (the first click only captures it), the window lost focus (`set_grab(false)` clears the click and the pulse), the pause menu or an online
  lobby/results screen has the window, or you are dead, carrying a prop or mid weapon switch. Nothing counts a dropped click: read `/phase`, `/online/phase`, `/online/in_round`,
  `/player/dead`, `/player/carrying`, `/player/weapon` in the dump.

## 2. What the client shows at once

Both modes run `fixed_step_combat` on the next tick. **Offline** it is the weapon: `fire_firearm` spends ammunition, starts the cooldown, casts a ray per pellet at the static shapes and
`PropWorld::ray_props`, shoves a loose prop and injects a `shot` rule event; nobody can be hurt (bots live in a server: `re2 --host`, ADR 0054). **Online** the server owns the weapon; the
client arms the pulse and `predict_attack` applies the same button rules (press edge, held for automatics, cooldown, switch) only to drive recoil (`since_shot`), muzzle flash
(`flash_left`), the gun sound (`Feel::own_shot`) and tracer and spark (`draw_own_shot`). It never looks at ammunition or at who was hit.

- **Open**: `src/bin/re2/weapons.rs::fixed_step_combat` and `::fire_firearm`, `src/bin/re2/feedback.rs::predict_attack` and `::draw_own_shot`, `src/feel.rs::own_shot`, `src/sfx.rs::play`.
- **Tuned by**: `MUZZLE_FLASH_TIME` (0.06 s), `RECOIL_TIME` (0.30 s) in `src/weapons.rs`; per gun `cooldown_ticks` and `recoil` in `Weapon::firearm`; `streaks::POOL` (64), `TRACER_SECS`, `SPARK_SECS`.
- **How to see it**: `RE2_LOG_CUES=1` prints `[cue ...] Shot { ... own: true }`; in the dump `/cues/counts/Shot`, `/cues/recent`, `/streaks/showing`; a picture of the flash and kick:
  `RE2_FREEZE_SHOT=0.02` with `--shot-at` or F12, or a script step `{"shot": "gun"}`.
- **Fails silently when**: (1) there is no audio device (`Audio::new()` is `None`): no sound, though the cue is still counted in `/cues/counts`; (2) ammunition is limited (`weapons.ammo`): the
  client predicts from the button alone, so it flashes and sounds a shot the server refuses, and no snapshot field or HUD shows the magazine; (3) the tracer ray knows walls and drawn players
  only, so it passes through loose props the server's ray hits (ADR 0055). Nothing reports (2) or (3).

## 3. The intent leaves the client

A click is a button on an input, never a position or a result. `fixed_step_physics` builds the tick's `PlayerInput` (`build_input`: `attack = take_pulse(1) || (attack_held && automatic)`),
`NetSession::step_local` gives it the next `seq` (1, 2, 3, never reset), predicts its movement and sends it. A semi-automatic gun's first `attack` leaves one tick after the click was seen
(the pulse is armed after that tick's input was built); an automatic gun reads `attack_held` at once. The pulse keeps the bit up for 3 inputs and every `InputPacket` carries the newest 4
(seq, flags, forward, strafe, yaw, pitch) and is signed with the session HMAC. That redundancy means a lost datagram loses nothing; the server acts on the rising edge, so 3 ticks up is one shot.

- **Open**: `src/bin/re2/frame.rs::build_input`, `src/bin/re2/weapons.rs::take_pulse`, `src/net/session.rs::step_local`, `src/net/client.rs::send_input`, `src/sim/player.rs::flags` (bit layout).
- **Tuned by**: `NET_PULSE_TICKS` (3, `src/bin/re2/main.rs`); `MAX_INPUTS_PER_PACKET` (4) in `src/net/protocol.rs`; `TICK_RATE_HZ` (60) in `src/sim/clock.rs`.
- **How to see it**: `tests/net_interactions.rs::a_shot_fired_by_one_client_hurts_another_and_four_kill_them` drives a headless `Bot` with `buttons = 16` (attack) over real UDP and counts the
  server's `shot` events; `red_server --record out/m.json` writes every input with its tick (`["i", tick, slot, seq, ..., flags, ...]`), `red_engine2 replay out/m.json` re-runs them.
- **Fails silently when**: the server is not in `Phase::Playing` (lobby, countdown, results): `Server::on_input` drops the input with no counter, and the client is frozen too
  (`App::online_frozen`); the link is `Reconnecting`, so `step_local` returns `None` and nothing is sent while step 2 still flashes (the HUD banner says CONNECTION LOST - RECONNECTING...);
  a flood beyond `limits::INPUT_PACKETS_PER_SEC` (180, burst 90) is dropped and counted in `ServerStats::rate_limited`.

## 4. The server takes the input and checks the trigger

`Server::on_input` records the client's RTT, sets its view lag (step 6) and pushes the inputs; `MatchSim::push_input` drops a stale or duplicate `seq`. `tick_once` pops one input per tick
(two while more than 3 wait), `step_player_tuned` applies its movement and look (`state.yaw`, `state.pitch` are the input's), then `handle_actions` compares the buttons with the previous
input's: a press, or `attack` held on an automatic weapon, calls `attack`. The shot happens on the tick that processes the input (`GameEvent.tick`), after the inputs queued ahead of it;
`Snapshot::ack_input_seq` tells the client which input that was. `attack` refuses when the body has no bat (the rat), a prop is carried, a switch is under way, the cooldown runs, or the
magazine is empty (a dry click starts `DRY_FIRE_COOLDOWN_TICKS`). Past the first three guards an attack ends spawn protection; a real shot starts `cooldown_ticks`, adds to `Combat::shots`
and injects the `shot` rule event.

- **Open**: `src/net/server.rs::on_input`, `src/sim/match_sim.rs::push_input` and `::tick_once`, `src/sim/interact.rs::handle_actions` and `::attack`.
- **Tuned by**: the `FirearmSpec` row in `src/weapons.rs::firearm`; scene `weapons.starting`, `weapons.ladder`, `weapons.ammo`; `INPUT_QUEUE_CAP` (8), `INPUT_QUEUE_TARGET` (3); `SWITCH_TICKS`.
- **How to see it**: the duel scenario above; `tests/interactions.rs::held_automatic_trigger_repeats_but_a_pistol_requires_a_new_press` and
  `::the_pistol_has_a_cooldown_damages_kills_and_the_victim_respawns`; a server whose scene has `rules` logs `event shot by rule engine (player N) at tick T`.
- **Fails silently when**: the shooter holds the bat, which `weapons.starting` gives everyone by default: the click swings (`/player/weapon` says `baseball bat`, nothing warns). Also more
  than `INPUT_QUEUE_CAP` inputs queued (a stall of about 130 ms) drop the oldest, a press among them, uncounted.

## 5. The ray

`MatchSim::probe_lagged(eye, dir, range, shooter, lag)` casts from the shooter's server-side eye (stand or crouch height, after this input's movement) along the input's yaw and pitch and
returns the nearest of: fixed geometry (`hit::raycast_shapes` over exact shapes, collected once from every top-level object that is not a loose prop); a loose prop (`PropWorld::ray_props`;
it gets `apply_impulse`, recorded in the trace, and every client then sees it move as a prop delta); another living player (`interact::ray_cylinder`: the body's radius and height, at the
rewound position). A wall in front of a player protects them. The shotgun casts 9 rays (`Weapon::pellets`, `shot_direction`) and splits the damage (`pellet_damage`).

- **Open**: `src/sim/interact.rs::probe_lagged`, `::ray_cylinder`, `::eye_and_look`, `src/hit.rs::raycast_shapes`, `src/physics/interact.rs::ray_props`.
- **Tuned by**: `FirearmSpec::range` and `Weapon::pellets` (`src/weapons.rs`); body radius and height (`Character::body`, `src/player.rs`); the map's own shapes.
- **How to see it**: `red_engine2 ray MAP --from x,y,z --to x,y,z` (the same `raycast_shapes`: `clear`, or the first object in the way);
  `src/physics/tests.rs::rays_find_dormant_props_too_so_a_bat_or_bullet_can_hit_something_nobody_has_touched`; server impulses are recorded and replayed
  (`tests/sim_replay.rs::a_real_udp_server_session_records_a_trace_that_replays_clean`).
- **Fails silently when**: the hit shapes never change and ignore `collide`: a rule that `hide`s or un-collides a door changes what players walk through (`rebuild_static_world`) but not what
  bullets hit, and a decorative `collide: false` glass stops them too. `ray` names the blocker; nothing warns.

## 6. Lag compensation (ADR 0053)

The shooter's screen shows the others about 100 ms in the past and the shot needs half a round trip, so the server judges it against the world the shooter saw. `MatchSim` keeps the last
`HISTORY_TICKS` (16) end-of-tick positions of every player. `Server::on_input` calls `set_view_lag(slot, view_lag_ticks(rtt))`: the interpolation delay (`INTERP_DELAY`, 0.100 s) plus the
client-reported round trip clamped to 100 ms, in ticks, at most 12 (200 ms): 6 on a loopback, 8 at 40 ms; `set_view_lag` clamps again to `HISTORY_TICKS - 1`. `probe_lagged` then puts every
other player's cylinder where it stood that many ticks ago (`rewound`); walls, props, the shooter's own eye and everyone's health are as of now. Bots are judged at lag 0. A change of lag is
recorded in the trace (`["v", tick, slot, lag]`, `Entry::ViewLag`), so a replay judges the same. The shooter is favoured; a player who respawned inside the window is judged at the old spot.

- **Open**: `src/net/interp.rs::view_lag_ticks`, `src/sim/match_sim.rs::set_view_lag` and `::rewound`, `src/sim/interact.rs::probe_lagged`, `src/sim/replay.rs::replay`.
- **Tuned by**: `INTERP_DELAY`, the 100 ms RTT clamp and the 12-tick cap in `view_lag_ticks`, `HISTORY_TICKS`. No scene key and no `red_server` flag exists for it.
- **How to see it**: `tests/combat_rules.rs::a_shot_is_judged_against_the_world_its_shooter_saw` (aim at the late spot with the lag known hits; the same aim judged in the present misses; and
  the two mirror cases) and `::view_lag_is_clamped_recorded_and_replays_bit_for_bit`; `src/net/interp.rs`'s `a_clients_view_lag_is_the_interpolation_delay_plus_its_round_trip_and_is_capped`.
- **Fails silently when**: moving targets are missed on a real link although they were hit on loopback. `red_engine2 net-test` does not judge hits (its checks are connections, prediction,
  corrections, remote gliding, bandwidth), so aim over a bad link is proved only in-process by the tests above; the RTT is the client's own report, capped.

## 7. Damage, death, ladder, score

`damage(target, amount, by)`: a spawn-protected or dead target takes nothing (returns false: no `hit`, no marker); else hit points fall, `hurt` and `hurt_bearing` are set and the `hit` rule
event fires (once per damaging pellet; the slot is the shooter). `Combat::hits` counts once per pull that landed anything. At 0 hp: `dead_until = now + respawn_ticks`, a carried prop drops,
the killer's `kills` rises and the `kill` event fires; on a ladder the killer is handed `weapon_for_kills(kills)` at once (a raise animation during which the trigger does nothing, and fresh
`weapons.ammo`). `combat_tick` respawns the dead at `pick_spawn` with the rung restored, protection, and a `respawn` event. Score has three layers: `Combat::kills` (the roster's `score`, and
`match.score_to_win` through `Server::step_flow`), scene `rules` reacting to `shot`, `hit`, `kill`, `respawn` (vars, `end`), and the ladder.

- **Open**: `src/sim/interact.rs::damage`, `::combat_tick`, `::respawn`, `src/weapons.rs::weapon_for_kills`, `src/sim/rules_run.rs::inject`, `src/net/server.rs::step_flow` and `::roster`.
- **Tuned by**: scene `combat` (`respawn_secs`, `spawn`, `spawn_protect_secs`, `regen_delay_secs`, `regen_per_sec`; `src/sim/combat_cfg.rs`), `weapons.ladder` (at most `MAX_LADDER`),
  `weapons.bat.damage`, `match.score_to_win`, `PLAYER_MAX_HP` (100).
- **How to see it**: `tests/combat_rules.rs::a_kill_climbs_the_ladder_and_a_respawn_keeps_your_rung`, `::feedback_counters_count_shots_landed_hits_and_times_hurt`,
  `::spawn_protection_blocks_damage_and_ends_with_the_first_shot`; `tests/interactions.rs::engine_events_reach_scene_rules_so_a_game_can_score_kills_with_data_only`.
- **Fails silently when**: the target is spawn-protected (`combat.spawn_protect_secs`): the shot is a clean miss as far as the shooter can tell, and nobody else is drawn differently; only the
  victim's own HUD says SPAWN PROTECTED. After a ladder kill the raise time reads as "the gun stopped working".

## 8. The news comes back

`Server::send_snapshots` runs every `snapshot_every` (2) ticks, 30 Hz, per client: `player_snaps` (one `PlayerSnap` per player: pose, `flags` bit 1 swinging, bit 2 dead, `FLAG_PROTECTED`,
`weapon`, `hp`, `shots`), `visible_players` (the interest filter), `props_to_send` (moved props, oldest unconfirmed first, at most 29) and `feedback_of` (yours alone: `hits` pulls that
damaged, `hurt`, `kills`, `bearing` of the last attacker in 1/256 turns, `respawn` in tenths of a second). Everything about a shot is a wrapping counter, not an event: a lost snapshot
delays the news by one and nothing is resent. Player records are full each time; only props are deltas (per-prop acks). Roster scores travel in `Status` (5 Hz).

- **Open**: `src/net/server.rs::send_snapshots`, `src/net/snapshots.rs::player_snaps`, `::feedback_of`, `::visible_players`, `::props_to_send`. (`src/sim/snapshot.rs` is a benchmark
  placeholder, not the wire snapshot.)
- **Tuned by**: `ServerConfig::snapshot_every` (`red_server --snapshot-every`), `MAX_PLAYERS_PER_SNAPSHOT` (8), `MAX_PROPS_PER_SNAPSHOT` (29), scene `zones`, `portals`, `interest.hops`.
- **How to see it**: `Bot::heard_shots`, `landed_hits`, `times_hurt`, `kills_scored` in `tests/net_bots.rs::shots_and_damage_reach_a_client_as_events` and
  `::a_clients_own_hits_and_kills_reach_it_as_events`; `/online/snapshots` and `/online/snapshots_missed` in the dump; `red_server` prints `interest management: N rooms, M portal hop(s)`.
- **Fails silently when**: interest. Rays ignore rooms, snapshots do not: you can hit, and get a hit marker for, a player whose record you were never sent (no avatar, no red crosshair).
  Reported by lint `interest` (`red_engine2 lint MAP`) and at run time by `RemoteStats::hidden_by_interest` (`/remote/hidden_by_interest`, the F3 overlay) and
  `SessionCounters::hidden_frames` (`NetSession::counters`, `/remote/counters/hidden_frames`). A counter that jumps by more than 8 shots in one snapshot is dropped as stale (`MAX_SHOTS_PER_SNAPSHOT`, `src/net/happenings.rs`). The gun travels as its index in
  `Weapon::ALL`: inserting or removing one renumbers the rest, and only a `PROTOCOL_VERSION` bump keeps old and new builds from disagreeing about which gun is which.

## 9. The shooter's and the victim's client

`NetClient::on_message` applies the snapshot and `Watcher::observe` turns counter differences into `Happenings`; `NetSession::poll` queues them (at most 64); each frame
`App::feedback_frame` feeds `Feel::on_happened` and plays what `Feel::take_cues` returns, and `Feel::fx` goes to the renderer (`src/fx.rs`). **Shooter**: a hit is `Cue::Hit` and a hit
marker (0.28 s); a kill is `Cue::Kill`, a kill marker (0.6 s) and, on a ladder, a gold flash, `Cue::LevelUp` 0.16 s later, the notice `RUNG n - GUN` and the HUD pips; the new gun is lowered
and raised when `own.weapon` changes. **Victim**: `Cue::Hurt`, a red vignette, a damage arc pointing at the attacker relative to the view, a heartbeat at 35 hp or less; death is
`Cue::Death`, the camera dropped to `DEAD_EYE_HEIGHT`, ELIMINATED and RESPAWNING IN n; on respawn the camera faces the server's yaw.

- **Open**: `src/net/client.rs::on_message`, `src/net/happenings.rs::observe`, `src/bin/re2/feedback.rs::feedback_frame`, `src/feel.rs::on_happened` and `::fx`, `src/ui/online.rs::hud_layout`.
- **Tuned by**: `HIT_MARKER_SECS`, `KILL_MARKER_SECS`, `HURT_SECS`, `LEVEL_UP_DELAY_SECS`, `LOW_HEALTH` in `src/feel.rs`; the gains in `src/sfx.rs::play`; `NOTICE_SECS` in `feedback.rs`.
- **How to see it**: `/cues/counts/Hit`, `Kill`, `Hurt`, `Death`, `Respawn`, `LevelUp`; `/hud/lines` (ids `notice`, `leader`, `final`, `dead_title`, `sb0_score`); `/online/roster`; `/player/hp`;
  `/crosshair/state` (`enemy` over a drawn enemy); `RE2_FEEL=hit|kill|hurt|dead|low|protected|flash` holds one effect for a picture; `src/feel.rs`'s
  `a_hit_ticks_and_shows_a_marker_that_fades_out` and `damage_points_at_the_attacker_relative_to_where_we_look_and_fades`.
- **Fails silently when**: `Feel` only reacts once the server has told us our own state (`NetSession::own`), and with no audio device the cues count but nothing plays. There is no kill feed
  and no name tag anywhere in the client: who killed whom is not shown; the closest are the scoreboard, the `leader` and `final` lines and, in a scene with `rules`, a rules panel naming the
  newest event (`EVENT: SHOT`) without who or whom.

## 10. Everybody else's client

The same snapshot carries each visible player's `shots` counter. A change becomes `ShotHeard`, scheduled as `Cue::Shot { own: false }` `REMOTE_SOUND_DELAY_SECS` (the interpolation delay)
later, because their avatar is drawn that far in the past. `play_cue` then draws their tracer and spark (`draw_remote_shot`: a ray from 1.65 m above their feet along the reported aim,
stopped by walls and drawn bodies, ours included; red spark on a body, warm on a wall) and plays the gun placed by `sfx::spatial` (falloff, pan, quieter behind). The shooter's avatar, a
`ScenePool` object claimed by `NetSession::claim_avatar`, sees the same counter in its interpolated pose: `avatar::animate` gives it recoil and a `RemoteHand` flash, drawn by
`LiveRenderer::set_remote_hands`. The victim's avatar only falls when `dead` arrives. Tracers and sparks are a separate pool of 64 boxes (`streaks::add_pool`, hidden through
`Streaks::hidden_ids`); `src/bin/re2/frame.rs::hidden_ids` hands both pools to `LiveRenderer::set_hidden_objects`.

- **Open**: `src/feel.rs::on_happened`, `src/bin/re2/feedback.rs::play_cue` and `::draw_remote_shot`, `src/sfx.rs::spatial`, `src/avatar.rs::animate`, `src/net/session.rs::apply_view`.
- **Tuned by**: `REMOTE_SOUND_DELAY_SECS` (`src/feel.rs`), `streaks::POOL`, `TRACER_SECS`, `SPARK_SECS`, `REMOTE_HANDS` (`src/viewer.rs`), `FALL_SECS` (`src/avatar.rs`).
- **How to see it**: `/cues/counts/Shot` and `/cues/recent` (entries with `own: false` are other players'), `/streaks/showing`, `/remote/players`; a script picture from behind another player:
  `{"shot": "watch", "camera": "follow"}`; `src/feel.rs`'s `shots_heard_become_positioned_cues_and_our_own_are_flagged`, `src/avatar.rs`'s `a_shot_kicks_the_gun_and_flashes_once_then_settles`.
- **Fails silently when**: the shooter is outside your interest range (no record, no counter: no sound, tracer or flash; `/remote/hidden_by_interest`); the shooter has no avatar (the
  invisible bot of ADR 0057: `/remote/undrawn`, a `warning:` line): sound and tracer still come, body and gun do not; a player's first snapshot only sets a baseline, so one shot in it is
  unheard; more than 64 tracers and sparks alive drop the oldest.

## 11. Bots

A bot is an ordinary slot with a `Brain`. Before inputs are consumed each tick, `MatchSim::run_bots` asks every brain for a `PlayerInput` (perceive with a real ray, aim with human-limited
swing and wobble, `fire` sets `attack` once it has reacted and is aligned) and queues it with `push_input`, so steps 4 to 7 run unchanged: same cooldowns, ammunition, ray, damage, events,
ladder. The one difference is lag 0: bots shoot the present. Clients see them as any player (`PlayerSnap`, `ROSTER_BOT` in the roster), so steps 8 to 10 are identical, and a recorded match
holds their inputs, so a replay needs no brain. `Server::sync_bots` fills empty slots from the top; a joining human replaces the weakest bot. No window, no wire, no client is involved.

- **Open**: `src/sim/ai/mod.rs::run_bots`, `::think`, `::fire`, `src/net/server.rs::sync_bots`, `src/sim/ai/skill.rs::level_from_name`.
- **Tuned by**: scene `bots` (`fill` 0 to 8, `skill`, `roster[].name`, `character`, `skill`, `style`); `red_server --fill N --bot-skill LEVEL`; `re2 --host --fill --bot-skill`; steps 4 to 7.
- **How to see it**: `cargo run --release --example bot_match -- MAP --bots 6 --secs 60 --weapon smg` (no window, no socket, faster than real time: kills, accuracy, distance, time standing
  still); `tests/net_bots.rs::shots_and_damage_reach_a_client_as_events`; `red_engine2 playtest MAP --fill 4`.
- **Fails silently when**: a bot wears a body the client has no avatar for (ADR 0057): it shoots you and cannot be seen; reported by `/remote/undrawn`, the `warning:` line and
  `red_engine2 game check` (see `docs/LIFE_OF_A_REMOTE_PLAYER.md`, step 6). A scene with `bots` but no `weapons.starting` or ladder gives everybody the bat, and the gunfight is melee, with
  no warning. A slab within jump reach shoves a jumper sideways, bots too: lint `jump-clearance`; a broken waypoint graph: `red_engine2 nav MAP`.
