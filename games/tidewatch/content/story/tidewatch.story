story tidewatch version 1
uses bindings.json
map ../tidewatch.json
round tw:watch_started
// Tidewatch: two night watches on a tidal island. Odile keeps the signal brazier; Bram waits on the dock for his father's boat.
// The decision that matters is two bricks of peat; the rest is who knows what about the night the Gannet sank.

// ---- Bram, on the dock: the optional interaction ------------------------------------------------------------------------------

conversation talk_bram with bram
  interrupt 5

node hello
  bram: You're the new hand? Odile said you'd be quick. Are you quick?
  bram: The lamp's lit already! Was that you? It doesn't matter. It's lit.
    if tw:dock_lit
    -> lamp_lit
  bram: The Wren is out there. My dad's boat. Two lamps on this coast and one of them is ours.
    if tw:watch == 0
  bram: The light held, didn't it? I watched it from here. It held.
    if tw:watch >= 1 && tw:boats_late == 0
  bram: It went dark out there. I counted. Was it long? It felt long.
    if tw:boats_late >= 1
  -> ask

node ask
  bram: The dock lamp's empty. Two bricks would light it for the Wren. Just two?
  * lend: Take two. Light it.
    if tw:shed_fuel >= 2
    kind consequential
    do run tw:lend 2; set lent_fuel = true; add trust_bram 1
    -> lent
    else -> shed_thin
  * refuse: The brazier needs every brick.
    kind consequential
    do add trust_bram -1
    -> refused
  * later: After the watch. I promise.
    if tw:phase == 0 && !promised_bram
    kind consequential
    do set promised_bram = true
    -> later
  * why: Why are you scared of Odile?
    if !knows(player, gannet_lost)
    kind informational
    -> gannet

node lent
  bram: Two. Two is plenty. Thank you. Really.
  -> end

node refused
  bram: No. No, you're right, of course you are.
  -> end

node later
  bram: After. All right. I'll hold you to it, you know.
  -> end

node gannet
  bram: She let it go out once. The night the Gannet sank. Dad was on the sister boat; he watched it go dark.
    id gannet_told
    reveals gannet_lost
  bram: Dad says her eyes are going now. Maybe that's why she counts.
    claims odile_failing
  bram: Don't tell her I told you?
  -> ask

node shed_thin
  bram: Oh. The shed's thin, isn't it. Never mind. Never mind.
  -> end

node lamp_lit
  bram: Two lamps lit. Dad will see us from the rocks.
  -> end

conversation bram_after with bram
  if promised_bram && !kept_promise && tw:watch >= 1 && tw:phase != 1
  priority 1

node ask
  bram: You said after. It's after. Is it after?
  * give: It's after. Two bricks, as I said.
    if tw:shed_fuel >= 2
    do run tw:lend 2; set lent_fuel = true; set kept_promise = true; add trust_bram 2
    -> kept
    else -> cannot_reply
  * cannot: The shed's too thin. I can't. I'm sorry.
    do add trust_bram -1; set promised_bram = false
    -> cannot_reply

node kept
  bram: You came back. People don't. Mostly they don't.
    if trust_bram >= 2
  bram: You came back.
    if trust_bram < 2
  -> end

node cannot_reply
  bram: Right. The shed. Of course. No, it's fine.
  -> end

// ---- Odile, by the brazier ------------------------------------------------------------------------------------------------------

conversation talk_odile with odile
  if tw:phase != 1
  repeat

node start
  odile: Bell's there. Ring it when you're ready to work.
    if tw:watch == 0
  odile: Bell's there. Ring it for the next.
    if tw:watch >= 1 && tw:phase == 0
  odile: That's the night. Go and sleep.
    if tw:phase == 2
  odile: Well?
  * gannet: Bram told me about the Gannet.
    if knows(player, gannet_lost) && !knows(odile, player_heard_gannet)
    kind informational
    do learn odile player_heard_gannet
    -> gannet
  * lent: I gave Bram two bricks for the dock lamp.
    if lent_fuel && !knows(odile, fuel_lent)
    kind consequential
    do learn odile fuel_lent
    -> lent
  * ready: Nothing. I'm ready.
    -> end

node gannet
  odile: Did he.
  odile: Then you know why I count. Ring the bell.
  -> end

node lent
  odile: Two. From the shed. During my watch.
  odile: He'll have his lamp, and you'll have the dark. Count better.
  -> end

// ---- barks: round intros, the work, the dark ----
// Priorities form a ladder: chatter 0, the lamp and the dock 2, hunger 3, help and relighting 4, a watch's end 4, round intros 5 (6 for the
// one that knows more), the light going out 6. A speaker's cooldown only holds back their lines of the same rung or lower.------------------------------------------------------------------------------

category chatter cooldown 12s

bark intro_first on tw:watch_started
  odile: First watch. Feed it before it asks. The sea won't wait for you to be sorry.
  if tw:watch == 1
  priority 5

bark intro_second_clean on tw:watch_started
  odile: One boat in. Do it again and I'll stop counting your steps.
  if tw:watch == 2 && tw:boats_late == 0
  priority 5

bark intro_second_dark on tw:watch_started
  odile: It went dark last watch. The Wren sat out there in it. Not again.
  if tw:watch == 2 && tw:boats_late >= 1
  priority 5

bark intro_second_lent on tw:watch_started
  odile: Second watch, and two bricks short for your kindness. Count better.
  if tw:watch == 2 && knows(odile, fuel_lent)
  priority 6

bark fed_full on tw:fed
  odile: Full. Don't stand there admiring it.
  if tw:brazier_fuel >= 4
  category chatter

bark fed_good on tw:fed
  odile: Good. Again.
  category chatter

bark fed_next on tw:fed
  odile: That's one. Now the next one.
  category chatter

bark low on tw:low
  odile: It's hungry. One brick left in it.
  if tw:secs_left > 3
  priority 3

bark low_again on tw:low
  odile: Last brick. Move.
  if tw:secs_left > 3
  priority 3

// In a watch's last seconds hunger is not worth a word: a deliberate silence, chosen when neither line above holds.
bark low_late on tw:low
  silence

bark out_odile on tw:out
  odile: It's out! Peat, now!
  if present(odile)
  priority 6

bark out_bram on tw:out
  bram: The light's gone! Is it gone? It's gone!
  if present(bram)
  priority 6

bark out_far on tw:out
  narrator: Behind you, the signal goes dark.
  priority 6

bark relit on tw:relit
  odile: There. Breathe later.
  priority 4
  cooldown 30s

bark lent_cheer on tw:lent
  bram: Two bricks! Look, Dad. Two!
  priority 2

bark lent_kept on tw:lent
  bram: There. Lit. See, Dad? People come back.
  if kept_promise
  priority 2

// Whoever is here when bricks go to the dock sees it happen (Odile, at the brazier, does not).
notice saw_lending on tw:lent
  do learn present fuel_lent

bark ended_safe on tw:watch_ended
  odile: Boat's in. Don't celebrate. Sweep.
  if tw:last_safe
  priority 4

bark ended_late on tw:watch_ended
  odile: Late. The Wren sat in the dark because of us. Remember the feeling.
  if !tw:last_safe
  priority 4

bark promise_remembered on tw:watch_ended
  bram: That's the watch! You said after. It's after!
  if promised_bram && !kept_promise && present(bram)
  priority 3

bark helped on tw:odile_helped
  odile: Mine. From my own pile. Don't make me do it twice.
  priority 4

bark radio_news
  narrator: The radio on the dock crackles: the Wren is past the rocks.

// Each of these events is answered by conditions that between them cover every case, which the linter cannot prove:
allow fallback tw:watch_started "watch 1 has its intro; watch 2 has one for a clean, a dark and a known-lending first watch"
allow fallback tw:watch_ended "tw:last_safe or its negation always holds"
allow repeat helped "Odile helps at most once a watch (the director's `help` group), so this is heard at most twice a night"

// ---- pacing: a small director over live state -------------------------------------------------------------------------------

director pacing
  every 15s
  recover 20s
  budget 2 per round
  tension tw:light_out * 3 + (tw:brazier_fuel <= 1) * 2 + (tw:secs_left < 15)
  rest if tw:phase != 1
  rest if tw:light_out
  intervene odile_brings_peat
    if tw:brazier_fuel <= 1 && tw:dark_secs >= 3 && tw:reserve > 0
    group help
    do run tw:odile_helps
  intervene radio
    if tw:secs_left > 15
    chance 40%
    group color
    do bark radio_news
