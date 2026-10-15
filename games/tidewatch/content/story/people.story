story tidewatch
// Who is on the island, what the story itself remembers, and what can be known. The scenes are in tidewatch.story.

character odile "Odile Marr"
  want: both boats in tonight, and the light never once dark
  need: to let someone else carry the light, and to forgive herself the night she did not
  flaw: counts everything and trusts no count but her own
  contradiction: preaches patience to the new hand and cannot stand still beside the brazier
  values: a promise is kept to the minute; she never lies about the sea
  boundaries: she will not leave the brazier, not even to save face
  relationships: bram, son of the man she failed (she owes the family and cannot say so); the player, a new hand she has not decided about
  voice: short declaratives and counts; weather and work words; meets warmth with a chore
  example: Two bricks. Not three. Three is how you get lazy.
  example: The sea does not care who is sorry.
  knows: gannet_lost
  secret: the night the Gannet sank she let the light go out, and she has never said it aloud
  present: scene:near_odile
  cooldown: 4s

character bram "Bram Teller"
  want: his father's boat, the Wren, tied up at this dock before the tide turns
  need: to stop waiting for someone else to fix things
  flaw: talks to fill every silence; asks where he should act
  contradiction: fearless on the radio to strangers, frightened of one old woman with a bucket of peat
  values: says what is true even when it costs him; takes nothing that is not offered
  boundaries: he will not leave the dock while the Wren is out
  relationships: odile, whom he blames and admires; his father on the Wren; the player, his hope for an ally
  voice: quick questions, half-finished; gets numbers wrong so someone will correct him
  example: Is it four bricks or five? Five. No. Four?
  example: Dad says the light is the only honest thing on this coast. Is that true, do you think?
  knows: gannet_lost
  beliefs: that Odile's eyes are failing (they are not)
  present: scene:near_bram
  cooldown: 4s

fact lent_fuel bool = false run "the player gave Bram two bricks from the shed for the dock lamp"
fact promised_bram bool = false run "the player told Bram they would help after the watch"
fact kept_promise bool = false run "the player came back after the watch and gave Bram the bricks"
fact trust_bram int = 0 run "how far Bram trusts the new hand"

info gannet_lost "Odile let the light go out the night the Gannet sank"
info fuel_lent "the new hand gave Bram two bricks from the shed"
  true if lent_fuel
info player_heard_gannet "the new hand knows about the Gannet"
info odile_failing "Odile's eyes are going"
  true if false

setup the_promise "The player promised Bram help after the watch"
  by talk_bram/later
  pays acknowledge promise_remembered
  pays resolve bram_after/give
  pays complicate bram_after/cannot

setup gannet_secret "Bram tells the player that Odile let the light die once"
  by talk_bram/gannet_told
  pays acknowledge talk_odile/gannet
  pays leave talk_bram/gannet_told

setup lent_two "The player gave Bram two bricks from the shed"
  by talk_bram/lend
  pays acknowledge lent_cheer
  pays complicate intro_second_lent
