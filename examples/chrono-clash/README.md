# Chrono Clash

A time-travelling arena shooter in the spirit of the classic console deathmatch games: every map is a different era, and the fighters are drifters from all of them.

**Speakeasy, Chicago 1932** (`speakeasy.json`) is the first arena: a two-level bootlegger warehouse. A bar sits in the middle, crates and pillars give cover, catwalk mezzanines run along both ends (stairs up, drop down), and a time-portal jump pad launches you onto the crate tower.

- Deathmatch, first to 20 eliminations, 5 minute rounds, no ready-up
- SMG start, 40 round magazine, health regenerates after 5 s, respawn at the spawn farthest from enemies
- Five bots with a nav graph: Tommy Vance (human, rusher), Dusty Rhodes (cowboy, sniper), Unit 7 (robot), Zorbo (alien, acrobat), Merlin (wizard)

```bash
re2 --host examples/chrono-clash/speakeasy.json                          # play against the bots
re2 --host --bot-skill easy examples/chrono-clash/speakeasy.json         # gentler bots
re2 --playtest --secs 40 --shots 8 --out out/cc examples/chrono-clash/speakeasy.json   # headless check: contact sheet + report
```

Status: authored on Windows without the dev CLI. It loads and a 40 s headless playtest passes (bots route, fight and score, no failures), but `lint`, `nav` and `walk` have not been run yet.
