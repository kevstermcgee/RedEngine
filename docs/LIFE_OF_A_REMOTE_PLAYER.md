# Life of a remote player

Another player, a person on another machine or a bot inside the server's simulation, is on your screen only if ten things go right in a row. This page follows one of them from its first
Hello to the frame it is drawn, and out again when it leaves. Each step names the function to open, the number or scene key that tunes it, **How to see it** (a command, test or state-dump
field that proves the step) and **Fails silently when** (the classic way it goes wrong with no error, and what reports it, if anything does). Paths are from the repository root;
`file.rs::f` is function `f` (a method too) in that file. The other half of a fight is `docs/LIFE_OF_A_SHOT.md`. Decisions behind it: ADR 0016, 0022, 0050, 0054, 0057 and
2026-09-28-seeing-what-the-player-sees. The commands and scripts below were composed from the code, not run for this page: adapt them to your map.

```
join     Hello{character, name} -> apply_character_policy -> free_slot -> Welcome{player_id}  bots: sync_bots, the top slots, their own bodies
tick     MatchSim -> PlayerSnap (pose, weapon, hp, shots, flags) every 2nd tick               interest: only your room and the rooms `hops` portals away
client   RemoteWorld::apply -> a History per id -> view() at server time - 100 ms             NetSession::apply_view -> an avatar of that body from a ScenePool
draw     avatar::animate (pose, walk, aim, flash) + RemoteHand (the gun)                      frame.rs::hidden_ids -> LiveRenderer::set_hidden_objects
leave    Bye, or 3 s of silence -> the record vanishes -> the avatar is released and hidden
```

## Watch the others without a window

```
red_engine2 playtest MAP --fill 4          # a generated script; the verdict says "other players: N drawn of M in view, K undrawn, ..."; exit 1 when one is undrawn
red_engine2 game check --dir GAME          # a game project: `map X: avatars: every body in play is drawn (Human x8, Cowboy x4, ...)`, or the bot nobody could see
red_bot --server 127.0.0.1:27015 --as cowboy --behavior circle:30   # a remote player of your choice (a server that forces `player.humans_play_as` overrides the body)
RE2_STATS=1 re2 MAP --host --fill 4        # every 2 s `remote: N drawn of M in view, K undrawn, S stand-in, H hidden by interest, U unposed`; F3 shows it live
```

Script syntax and the pointers of the state dump (`re2-dump/1`): `red_engine2 describe playtest`. A script that fails the run when anybody is missing (`re2 MAP --host --fill 4 --headless --script out/others.json`):

```json
{"policy": "idle", "steps": [
  {"wait_for": {"at": "/online/in_round", "eq": true, "within": 30}},
  {"wait_for": {"at": "/remote/in_view", "min": 1, "within": 30, "msg": "the server sent nobody: interest?"}},
  {"expect": {"at": "/remote/undrawn", "eq": 0, "msg": "an enemy nobody can see"}}, {"expect": {"at": "/remote/standins", "eq": 0}},
  {"expect": {"at": "/remote/players/0/avatar", "contains": "net_"}}]}
```

## 1. Choosing a body (the Hello)

A human is the Human unless `--as` / `RE2_CHARACTER` says otherwise (there is no picker). A scene whose `player.humans_play_as` (older spelling `player.character`) names one forces it on every
*human*: the client uses it (`resolved_character`: the scene beats `--as`) and the server overrides what a Hello or lobby packet asks for (`apply_character_policy`). Bots are not
humans: they wear `bots.roster[].character` (human, wizard, cowboy, alien or robot, never the rat) and, beyond the roster, a rotation of cowboy, wizard, alien, robot (`BotsConfig::spec`).
A body travels as one byte (`character_to_wire`, the order of `Character::ALL`) in `Hello`, `Welcome`, every `PlayerSnap` and the roster. In a lobby C cycles bodies unless one is forced.

- **Open**: `src/bin/re2/main.rs::resolved_character`, `src/net/server.rs::apply_character_policy` and `::on_hello`, `src/sim/ai/mod.rs::spec`, `src/net/protocol.rs::character_to_wire`.
- **Tuned by**: scene `player.humans_play_as`, `bots.roster[].character`, `bots.fill`; `re2 --as`.
- **How to see it**: `/online/roster` (`character`, `bot` per entry) and `/player/character` in the dump; `src/net/server.rs`'s `a_scene_character_overrides_join_and_lobby_requests`,
  `src/bin/re2/main.rs`'s `a_scene_policy_skips_and_overrides_the_generic_picker`; `red_bot --as cowboy` joins in that body.
- **Fails silently when**: the scene forces the humans' body and its bots wear others. Forcing does not reach the bots, so the client must prepare avatars for both (step 6, the bug that
  shipped Trigger Happy with invisible enemies). A request the setting overrides (a rat asked for in a Human game) gets no message; only `Welcome.character` and the roster show it.
  The connect form (`src/bin/re2/online.rs::try_connect`) asks for only a Human or a Rat; any other body comes from `re2 --connect` (menu pick or `--as`) or from the lobby's C key.

## 2. A slot

`Server::on_hello` gives the joiner the lowest player id nobody holds (`free_slot`); an id parked for a dropped player's resume is avoided until every free one is reserved, then the oldest
reservation is given up. That id is `Welcome.player_id`, the slot in `MatchSim`, the index in every snapshot and roster line, and it survives the lobby, every round and a reconnect with the
token. The body appears when the world does: at once in open play, or on joining a running countdown or round when `match.join_in_progress` allows it; otherwise when the next countdown
starts (`begin_round_world`). Bots take the highest free slots (`sync_bots`), a joining human takes the weakest bot's place and a leaver hands it back; `bots_spawned` restarts each round.

- **Open**: `src/net/server.rs::free_slot`, `::on_hello`, `::welcome_for`, `::sync_bots`, `::begin_round_world`, `src/sim/match_sim.rs::add_player_in_slot`.
- **Tuned by**: `MAX_PLAYERS` (8, `src/sim/match_sim.rs`), `ServerConfig::resume_grace` (30 s) and `client_timeout` (3 s), `limits::MAX_PARKED` (32), scene `match.join_in_progress`, `bots.fill`.
- **How to see it**: `/online/id` and `/online/roster` (`id`, `bot`, `in_round`) in the dump; `red_server` logs `join ADDR as player N 'NAME'`;
  `tests/net_bots.rs::a_server_fills_empty_slots_with_bots_a_human_takes_a_place_and_gives_it_back`, `tests/net_flow.rs::a_player_who_joins_during_the_countdown_is_in_the_round_not_waiting_for_the_next_one`.
- **Fails silently when**: the roster lists a player in the round whom you never see: that is step 4, not the slot (compare `/remote/roster_others` with `/remote/in_view`). Code that assumes
  ids `0..n` are humans is wrong: bots hold the top ids and a parked player keeps its id for 30 s. A ninth player is refused loudly (`RejectReason::Full`).

## 3. State into snapshots

Every `snapshot_every` (2) ticks, 30 Hz, `player_snaps` writes one `PlayerSnap` per player in the sim: id, body, `flags` (bit 0 crouching, bit 1 swinging, bit 2 dead, `FLAG_PROTECTED`),
position (x, foot y, z), yaw, pitch, speed, vy, velocity, weapon (index in `Weapon::ALL`), carried prop, hp, `shots`. Records are complete, not deltas, and capped at 8; the server never sends an
animation, only the state that drives one. Adding a state others must see touches, in order: `PlayerSnap` with its encode and decode, `player_snaps`, `impl From<&PlayerSnap> for PlayerPose` and
`Blend for PlayerPose` (`src/net/interp.rs`), `avatar::animate`, and `PROTOCOL_VERSION`. The compiler checks the struct literals; nothing forces `animate` to read the field or you to bump the version.

- **Open**: `src/net/snapshots.rs::player_snaps`, `src/net/server.rs::send_snapshots`, `src/net/protocol.rs::snapshot_bytes`.
- **Tuned by**: `ServerConfig::snapshot_every`, `MAX_PLAYERS_PER_SNAPSHOT` (8) in `src/net/protocol.rs`; the size budget of `tests/net_budget.rs`.
- **How to see it**: `/remote/players` in the dump (`id`, `body`, `avatar`, `pos`, `dead`); `BotFrame::remote` as read by `tests/net_e2e.rs::two_clients_join_move_and_see_each_other_smoothly`;
  `red_bot --server ADDR --duration 5` prints JSON reports of who it sees.
- **Fails silently when**: a field is added on the server but `animate` never reads it: it reaches every client and changes nothing on screen (today's example: `PlayerPose::protected` is
  carried to every client and nothing reads it, so a spawn-protected player looks like anyone else). A layout change without a `PROTOCOL_VERSION`
  bump lets builds that disagree about the bytes join each other (the Hello compares only the version and the map hash); a snapshot that outgrows `snapshot_bytes` fails
  `tests/net_budget.rs::the_worst_case_snapshot_and_the_bandwidth_it_implies_are_within_budget`.

## 4. Interest management and portals

A client is told about its own room and the rooms within `interest.hops` open portals of it (default 1, at most 16); anything standing in no zone is always relevant. `InterestMap` is built from
the scene's `zones` (rooms), `portals` (`between` two zones, `open` unless it says false) and `interest.hops`; `visible_players` filters the *other* players' records by room, `props_to_send`
the props. Your own record and your own `Feedback` are always sent. The room comes from position (`room_at`: x and z inside a zone's rectangle, the highest floor at most 0.5 m above the feet), never
from line of sight. `red_server` and `re2 --host` build the map at start when the scene has zones (`red_server --no-interest` turns it off).

- **Open**: `src/sim/interest.rs::parse`, `::relevant`, `::room_at`, `src/net/snapshots.rs::visible_players`, `src/net/server.rs::set_interest`.
- **Tuned by**: scene `zones[]` (`id`, `rect`, `y`), `portals[]` (`between`, `open`), `interest.hops`; `red_server --no-interest`.
- **How to see it**: `red_engine2 lint MAP` (code `interest`); `red_server` prints `interest management: N rooms, M portal hop(s)` (or `off (the map has no zones)`); `/remote/roster_others`,
  `/remote/in_view`, `/remote/hidden_by_interest` in the dump, and `roster others N  hidden by interest M` on the F3 overlay; `tests/net_interest.rs::clients_in_far_rooms_stop_receiving_props_and_players_and_bandwidth_drops`,
  `src/tools/lint.rs`'s `linked_zones_are_clean_and_disconnected_ones_or_too_few_hops_are_named`.
- **Fails silently when**: players can walk to each other and never see each other: a multiplayer map whose zones have no portals, a spawn zone no open portal reaches, two spawn zones more than
  `interest.hops` portals apart. Lint `interest` says so before play (Error: no portals, or spawn zones no chain of open portals joins; Warn: an isolated spawn zone, a pair beyond `hops`); at
  run time `RemoteStats::hidden_by_interest` and `SessionCounters::hidden_frames` count the players the roster says are in the round and the snapshots left out.

## 5. The interpolation buffer

`NetClient` feeds every snapshot to `RemoteWorld::apply`: a `History<PlayerPose>` per player id (24 samples) stamped with server time, and `ServerClock` estimates server time from arrivals,
trusting the quickest packets. `RemoteWorld::view` poses every player named in the newest snapshot (except us) at server time minus `INTERP_DELAY` (0.1 s, three snapshot intervals): between two
samples it blends position, yaw, pitch and speed, and takes flags and weapon from the newer; past the newest it carries a moving player along their last heading for at most
`MAX_EXTRAPOLATE` (0.25 s), then holds. A drawn position that would jump farther than `TELEPORT_DISTANCE` (4 m, a respawn) snaps; a shorter jump after an outage glides at no less than
`CATCH_UP_SPEED`, more for fast maps (`catch_up_speed`). A player missing from a snapshot is forgotten at once, so a newcomer in the slot does not glide in.

- **Open**: `src/net/interp.rs::apply`, `::view`, `::sample`, `::limit_catch_up`, `src/net/client.rs::view`, `src/net/session.rs::update_scene`.
- **Tuned by**: `INTERP_DELAY`, `MAX_EXTRAPOLATE`, `CATCH_UP_SPEED`, `TELEPORT_DISTANCE` in `src/net/interp.rs`; the scene's `player` block and `jump_pads`, which `NetSession::connect_with` hands to
  `NetClient::set_movement_profile`.
- **How to see it**: `red_engine2 net-test MAP --profile bad` (check "other players glide, they do not teleport": worst drawn jump at most 0.15 m);
  `tests/net_sim.rs::a_cruel_link_still_keeps_everyone_connected_and_nobody_teleports`; `src/net/interp.rs`'s `a_burst_of_lost_snapshots_carries_a_moving_player_on_instead_of_freezing_then_snapping`
  and `a_long_outage_ends_in_a_quick_glide_not_a_snap_but_a_teleport_still_snaps`.
- **Fails silently when**: snapshots stop and a remote player freezes, then snaps forward (found by `net-test`, fixed by the extrapolation cap and the glide); a fast game's players outrun
  the glide when the movement profile is not set. `RemoteStats::unposed` (`/remote/unposed`) counts players a snapshot named but the buffer cannot pose yet.

## 6. An avatar from the pool (ADR 0057)

Avatars are hidden scene objects made before the renderer exists, because it takes its meshes from the scene at creation. `NetSession::add_avatar_pool` (called by `start_game` before
`LiveRenderer::new`) builds one `ScenePool` per body from `avatar_plan`: 8 for every body a human can wear (any, or only the `player.humans_play_as` body) and, for bodies only bots wear, as many
as the scene's bots need and at least `BOT_BODY_POOL` (4) per fighting body when `bots.fill` is above 0. The objects are `net_<body>_<k>`. Each frame `apply_view` gives every player in the view an
avatar (`claim_avatar`): the one they wear if it is still of their body, else a free one of their body, else a stand-in of the same rig (person or rat), else none (`undrawn`).
**The bug**: the pool was sized for the humans' body only and the client skipped a player it had no avatar for, so a game that forced Human and fielded cowboys, wizards and robots shipped with
enemies that shot and could not be seen; every test had used a Human as the opponent.

- **Open**: `src/net/session.rs::avatar_plan`, `::build_avatar_pools`, `::claim_avatar`, `::apply_view`, `src/scene_pool.rs::claim`, `src/tools/game.rs::avatar_line`.
- **Tuned by**: `BOT_BODY_POOL`, `MAX_PLAYERS_PER_SNAPSHOT`; scene `player.humans_play_as`, `bots.fill`, `bots.roster[].character`.
- **How to see it**: dump `/remote/in_view`, `drawn`, `undrawn`, `undrawn_ids`, `standins`, `/remote/pool` (`capacity`, `in_use`), `/remote/bodies` (per body `capacity`, `in_use`, `high_water`,
  `failed_claims`), `/remote/counters/undrawn_frames` (`NetSession::counters`), `/remote/players/N/avatar`; `src/net/session.rs`'s
  `bots_in_bodies_the_humans_are_not_forced_to_are_drawn`, `a_body_the_pool_lacks_is_stood_in_for_rather_than_left_undrawn`, `the_pool_holds_every_body_somebody_can_wear`,
  `a_player_the_pool_cannot_dress_is_counted_and_warned_about_once_not_skipped_in_silence`.
- **Fails silently when**: a body has no avatar to wear. Four things catch it now: `red_engine2 game check` (`avatar_line`: `bot 'NAME' wears Cowboy and the client prepares no Cowboy avatar: it
  would be invisible`, test `src/tools/game.rs`'s `game_check_confirms_the_client_will_draw_every_body_the_bots_wear`); the counters `RemoteStats::undrawn`, `undrawn_ids`, `standins`,
  `SessionCounters::undrawn_frames`, `standin_frames` and the pool's `failed_claims`; one warning per player (`take_warnings`, printed as `warning: player N wears BODY and the avatar pool has
  nothing free for them (pool: ...): they are INVISIBLE ...`); and the exit code, since a warning is a failure of a headless run and `playtest` ends with `expect /remote/undrawn eq 0`.

## 7. The gun in its hand

The gun is not part of the avatar object. `avatar::animate` returns a `RemoteHand` (weapon, wrist position, yaw, pitch, bat pitch, recoil kick, muzzle flash), or `None` for a rat and for the dead.
`NetSession::remote_hands` passes them to `LiveRenderer::set_remote_hands` (from `App::draw`), which draws every `HeldPart` of that weapon (`build_all_held_parts`: the bat and each firearm's
`build_firearm_parts`; the first-person sleeve is skipped, a muzzle-flash piece only while `flash` is above 0) at `remote_hand_transform`, once per player. The weapon number is the snapshot's
`PlayerSnap::weapon`, so a ladder rung up swaps the held parts when that pose is drawn.

- **Open**: `src/avatar.rs::animate`, `src/viewer.rs::set_remote_hands`, `::remote_hand_transform`, `::build_all_held_parts`, `src/firearms.rs::build_firearm_parts`.
- **Tuned by**: `REMOTE_HANDS` (8, `src/viewer.rs`), `MUZZLE_FLASH_TIME` (`src/weapons.rs`), `firearms::grip_anchor`, `AIM_SHOULDER_X` and `AIM_ELBOW_DEG` (`src/avatar.rs`).
- **How to see it**: a script picture `{"shot": "gun", "camera": "follow"}` (behind the nearest other player); `cargo run --example weapon_poses -- out/weapon-poses` (every firearm through the
  live renderer) and `cargo run --example avatar_poses -- out/poses.png` (avatars with guns); `src/avatar.rs`'s `a_player_holding_a_gun_holds_it_out_and_the_wrist_follows_the_body`.
- **Fails silently when**: a remote body is drawn without its gun. Rats and the dead hold nothing by design; nothing counts a gun missing for any other reason, so look at a `follow` picture.
  A weapon number the client does not know is the bat (`Weapon::from_wire`).

## 8. Animation

`avatar::animate` poses the pooled object from the interpolated `PlayerPose`: position and facing, the walk cycle from `speed`, an idle sway, the head from `pitch`, the gun arm raised and
following the aim with a kick each time `shots` changes, the bat swing when `swinging` rises, the fall when `dead` and standing again on respawn. The local player's third-person body uses the
same constants (`src/bin/re2/avatar.rs::pose_human`), so everyone holds a weapon the same way.

- **Open**: `src/avatar.rs::animate`, `::recoil_kick`, `::swing_blend`, `src/bin/re2/avatar.rs::pose_human`.
- **Tuned by**: `WALK_CYCLES_PER_SEC_AT_WALK_SPEED`, `HIP_SWING_DEG`, `SHOULDER_SWING_DEG`, `FALL_SECS`, `RISE_SECS` in `src/avatar.rs`; `RECOIL_TIME` in `src/weapons.rs`.
- **How to see it**: pictures (`overview`, `follow`); `/remote/players` (`dead`); `src/avatar.rs`'s `the_dead_fall_over_and_lose_their_weapon_then_stand_when_they_respawn`,
  `the_bat_swings_through_its_arc_when_the_swing_flag_rises`, `walking_cycles_the_legs_and_a_rat_gets_no_weapon`; `cargo run --example avatar_poses -- out/poses.png`.
- **Fails silently when**: a shot fired in the snapshot where a player first appears is not animated (the first sighting of a counter is a baseline); a state the server has but `PlayerSnap`
  lacks (step 3) cannot be animated; a player who is undrawn (step 6) animates nothing.

## 9. Names, scoreboard, crosshair

There is no name above a head. A player's name (`Hello.name`, cut to `MAX_NAME` bytes by `sanitize_name`, `player` when empty; a bot's is `bots.roster[].name`) appears in the lobby table, the
results table and the in-round scoreboard (`sb{i}_name`, `sb{i}_score`, yours in gold), all from `Status.roster`, sent at 5 Hz and at once when the roster or the flow changes. With a firearm in hand the crosshair
turns red over a living, drawn remote body with no wall in front (`NetSession::player_in_sight`).

- **Open**: `src/ui/online.rs::hud_layout` and `::lobby_layout`, `src/net/server.rs::roster` and `::send_statuses`, `src/bin/re2/online.rs::online_view`, `src/net/protocol.rs::sanitize_name`,
  `src/net/session.rs::nearest_body_on_ray`.
- **Tuned by**: `MAX_NAME` (16) and `MAX_ROSTER` (8) in `src/net/protocol.rs`; `STATUS_EVERY_TICKS` in `src/net/server.rs`.
- **How to see it**: `red_engine2 ui-shot hud out/hud.png` (and `ui-check`, which audits the screens at several sizes); `/hud/lines` (`sb0_name`, `sb0_score`, ...), `/online/roster`,
  `/crosshair/state` in the dump.
- **Fails silently when**: no scoreboard is drawn until the first `Status` arrives (`online_view` is `None`); two players can share a name (only ids differ) and nothing above a head tells them apart.

## 10. Leaving, dying, respawning, hidden ids

A `Bye` or `client_timeout` (3 s) of silence ends the session: `drop_session` removes the body and parks the token. The next snapshot lacks the id, `RemoteWorld::apply` forgets the history,
`apply_view` releases the avatar and `ScenePool::release` hides it. Leaving interest range looks the same. A death sets `flags` bit 2: the avatar falls (`FALL_SECS`), holds nothing, is ignored by
`nearest_body_on_ray`, and stays for `combat.respawn_secs`; the respawn moves the body to a spawn (a jump beyond `TELEPORT_DISTANCE` is not glided) and it stands. Every frame
`src/bin/re2/frame.rs::hidden_ids` collects the rule-hidden objects of the server's `RuleState` (`NetSession::hidden_objects`), the unworn avatars (`hidden_avatar_ids`), the idle tracer boxes and,
in first person, your own body, and `LiveRenderer::set_hidden_objects` skips them in the colour and shadow passes: a mesh scaled to a speck is still a draw call.

- **Open**: `src/net/server.rs::drop_session`, `src/net/interp.rs::apply`, `src/net/session.rs::apply_view` and `::hidden_avatar_ids`, `src/scene_pool.rs::release` and `::hidden_ids`,
  `src/sim/interact.rs::respawn`, `src/viewer.rs::set_hidden_objects`.
- **Tuned by**: `ServerConfig::client_timeout` (3 s) and `resume_grace` (30 s), scene `combat.respawn_secs`, `combat.spawn`, `TELEPORT_DISTANCE`, `FALL_SECS`, `RISE_SECS`.
- **How to see it**: `/remote/pool` (`in_use` returns to 0 when everyone left) in the dump; `tests/net_e2e.rs::a_client_can_disconnect_and_reconnect_gracefully_and_by_timing_out`;
  `src/net/session.rs`'s `a_networked_session_draws_the_other_player_and_moved_props_into_the_scene` (the avatar appears, then is hidden) and
  `a_stand_in_is_counted_and_a_full_pool_of_the_right_body_counts_its_failed_claim` (the hidden list is exactly the unworn avatars).
- **Fails silently when**: "where did they go?" has two causes that look alike, a player who left and one who left your interest range (`/remote/hidden_by_interest` tells them apart, step 4).
  A pooled object left out of `set_hidden_objects` still draws twice a frame as a speck: a cost, not a bug (`LiveRenderer::hidden_mesh_count`).
