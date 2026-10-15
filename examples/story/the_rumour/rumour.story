story rumour version 1
uses ../bindings.json
// THE RUMOUR. Tense, small. Structure: a revelation that is not true, and what knowing it does to the people who hear it.
// The craft point: a claim is not a fact. Hale believes what he says; it is false; the player only comes to *believe* it, and Odo's
// reaction depends on whether he knows the player heard it, not on what the world says.

character hale "Hale Fenwick"
  want: Odo's corner of the square for his own cart
  need: to stop blaming others for his empty mornings
  flaw: he says the worst thing he can think of and calls it honesty
  contradiction: hates gossip, deals in nothing else
  values: loyal to the fishing families; never cheats a weight himself
  relationships: odo, a rival he once apprenticed under; the player, an audience
  voice: confiding, low, every sentence a nudge
  example: Between us? Weigh his sacks yourself.
  example: I'm only saying what everyone thinks and won't.
  present: m:near_hale

extra odo "Odo"
  role: the miller, old, on a flour cart at the corner
  voice: slow, polite, very tired
  present: m:near_odo

info watered_flour "Odo waters his flour to make the weight"
  true if false
info heard_rumour "the player has heard Hale's rumour"

conversation gossip with hale
node lean
  hale: Between us? Weigh Odo's sacks yourself. Wet flour weighs heavy.
    claims watered_flour
  // Honest stakes: asking changes only what he says next (he has no proof to reveal), so it is expressive, and `story check` says so.
  * ask: How do you know?
    kind expressive
    -> proof
  * leave: I'll weigh my own sacks.
    kind expressive
    -> end

node proof
  // A direct answer where subtext would be dishonest: he has none, and the line admits it by dodging.
  hale: Know? I don't need to know. I've got a nose.
  -> end

// Whoever stands at the fish stall when the player buys flour later sees it: Hale learns the player went to Odo anyway.
notice saw_flour on m:flour_bought
  if believes(player, watered_flour)
  do learn present heard_rumour

conversation cart with odo
  repeat
node greet
  odo: Flour's dry today. Always is.
  // Withheld unless Odo knows the player heard the rumour: he does not answer gossip he has not heard of.
  odo: Hale's been talking, hasn't he. He was my apprentice. I taught him to talk.
    if knows(odo, heard_rumour)
  // The choices ride on a line that is always said: on a conditional line they would vanish with it.
  odo: What'll it be?
  * buy: One sack.
    if m:coins >= 3
    do run m:buy_flour 3
    -> end
  * tell: Hale says you water it.
    if believes(player, watered_flour) && !knows(odo, heard_rumour)
    kind consequential
    do learn odo heard_rumour
    -> hurt
  * go: Another day.
    -> end

node hurt
  odo: Does he.
  // Silence as an answer: no line, the pause is the scene. The player's next words are theirs.
  odo: ...
  -> greet
