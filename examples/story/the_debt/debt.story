story debt version 1
uses ../bindings.json
// THE DEBT. Warm, a little wry. Structure: a relationship and a commitment, paid off (or not) hours later.
// Inventory first (workflow step 1): the game offers coins, a bread stall, the hour and a way to pay. The premise uses only those:
// you are short of coin, the baker is not short of opinions, and credit is a promise the clock will remember.

character wynn "Wynn Arden"
  want: to sell out before the noon bell
  need: to be trusted back by a town that once called her a cheat
  flaw: she keeps score, out loud
  contradiction: gives credit to strangers, never to friends
  values: an honest weight; she will not refuse a hungry child
  relationships: the player, a stranger with an empty purse
  voice: market patter turned dry; prices as punctuation
  // Examples are hers alone: perspective (a seller who was once called a thief), not a catchphrase.
  example: Two coins, and that's me being sentimental.
  example: I remember faces. Faces and debts.
  present: m:near_wynn
  cooldown: 5s

fact owes bool = false run "the player took a loaf on credit"
fact paid bool = false run "the player paid the debt"

setup credit "Wynn gave the player a loaf on the promise of payment before evening"
  by stall/credit
  pays resolve settle/pay
  pays complicate evening_dun
  pays acknowledge settle/thanks/l1

conversation stall with wynn
  if m:loaves > 0 && !owes

node offer
  wynn: Last of the rye. Two coins, and that's me being sentimental.
  * buy: Two coins. Here.
    if m:coins >= 2
    do run m:pay_wynn 2; run m:take_loaf
    -> sold
  // A meaningful refusal costs something real: here, only her opinion of you. That is still a consequence: a later line reads it.
  * credit: I'll pay you this evening.
    kind consequential
    do run m:take_loaf; set owes = true
    -> trusted
  * no: Not today.
    -> end

node sold
  wynn: Pleasure. Mind the crust, it bites back.
  -> end

node trusted
  // Subtext: she says yes to the loaf and no to the trust, in one breath.
  wynn: This evening. I'll be here. So will my memory.
  -> end

conversation settle with wynn
  if owes && !paid

node ask
  wynn: Back already? That's a first.
  * pay: The two coins I owe.
    if m:coins >= 2
    kind consequential
    do run m:pay_wynn 2; set paid = true
    -> thanks
  * later: Not yet.
    kind expressive
    -> end

node thanks
  // Earned warmth: only after the debt is paid does she drop the patter.
  wynn: Huh. People don't, you know. Come back tomorrow; I'll have rye.
  -> end

// The callback names what this player did. It is withheld if she cannot see them (presence) or the debt is paid.
bark evening_dun on m:hour_struck
  wynn: That's the six o'clock. Faces and debts, friend. Faces and debts.
  if owes && !paid && m:hour >= 18
  priority 3

bark morning on m:hour_struck
  silence
