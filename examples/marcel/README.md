# Marcel

A boy in an endless world of fields and forests. Each day fades to night and the sun rises again: the sky fills with stars, fireflies come out of the grass, an owl calls in the dark, and the music follows the sun. The menu says how many days you have lived.

```bash
re2 examples/marcel/marcel.json            # play (WASD move, Shift runs, mouse looks, Q switches to a view from behind, Esc pauses: music and sound can be turned off there, or with N)
red_engine2 frame examples/marcel/marcel.json out/m.png --hour 5.8      # a picture of any hour (0 to 24)
red_engine2 sky examples/marcel/marcel.json out/sky.png                   # a whole day as a contact sheet
red_engine2 procgen out/world.png --seed 7 --size 600                    # the world from above (add --biomes)
red_engine2 flora out/plants.png                                         # the twenty plants
red_engine2 audio report ambience.nature.wind ; red_engine2 audio picture bird.robin out/robin.png
re2 examples/marcel/marcel.json --headless --script s.json --dump d.json  # live a day in a moment and check what was heard (see `describe playtest`)
```

What it is made of (all of it the engine's, nothing here is code):

- `marcel.json`: `procgen` (the world, from a seed), `clock` (a 20-minute day that begins before sunrise), `player` (the boy, seen through his own eyes, fading in from black, peaceful), `audio` (nature without songbirds, plus four scores), `vars`/`persist`/`rules` (every sunrise you see is a day, kept between sessions), `ui` (the start card is the menu).
- `audio/marcel_{dawn,day,dusk,night}.json`: the four ambient scores, one per mood; the sun's height crossfades between them. Edit one and check it with `red_engine2 audio report examples/marcel/audio/marcel_dawn.json` (loudness, seam, spectrum) or `audio picture`.

The days you have lived are saved per game under `$XDG_CONFIG_HOME/red_engine2/games/` (`%APPDATA%` on Windows) in `vars.json`; delete it to start counting again. `RE2_MUSIC=0` or `RE2_AMBIENCE=0` silence the music or the countryside for a run.
