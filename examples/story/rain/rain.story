story rain version 1
uses ../bindings.json
round m:rain_started
// RAIN. Quiet. Structure: systemic aftermath with no conversation at all; the square tells its own story through weather and silence.
// The craft point: environmental storytelling. Nobody explains the town; the town reacts, and sometimes nothing is said on purpose.

extra wynn "Wynn"
  role: the baker, under her awning
  voice: brisk, practical
  present: m:near_wynn

category weather cooldown 30s

bark first_drops on m:rain_started
  narrator: The first drops darken the cobbles. Stalls start to shutter, one by one.
  if !m:awning_open
  once
  priority 2

bark wynn_rain on m:rain_started
  wynn: Bread and rain. Worst marriage in the world.
  category weather

// Most rain starts are not worth a word: when nothing more specific holds, the square is quiet on purpose.
bark hush on m:rain_started
  silence
  priority -1

bark rain_stops on m:rain_stopped
  narrator: Steam lifts off the square. Someone laughs, too loudly, relieved.
  category weather

bark rain_stops_quiet on m:rain_stopped
  silence
  priority -1

// A small director over live state: if the rain lingers and the awning is in, the town opens it (a permitted, unattended action that
// takes nothing from the player). It rests at night and gives up after its budget: the variety is finite and the log says so.
director sky
  every 20s
  recover 60s
  budget 1 per round
  tension m:raining * 2 + (m:hour >= 20)
  rest if m:hour < 7 || m:hour > 21
  intervene awning
    if m:raining && !m:awning_open
    chance 50%
    do run m:awning
