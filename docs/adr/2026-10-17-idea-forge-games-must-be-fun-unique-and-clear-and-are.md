# 2026-10-17. Idea Forge games must be fun unique and clear and are labelled AI-designed
Status: accepted
Summary: The Idea Forge brief ranks fun, uniqueness and clarity above fidelity to the random idea card, and publishing marks each game `"author": "ai"` so RedEngineGames can badge and filter it.

## Context
The owner judged the first sixteen games the schedule published (2026-10-08 to 2026-10-17) unfun, awkward, strange and really confusing. The brief told the agent to keep the card's YOU and BUT together as the mechanic and finish small, but said nothing about fun, originality or whether a stranger can understand the game, so odd cards became odd, unexplained games. The site also described games with one line (often just "X, built with RedEngine."), and gave no way to tell these from the human-designed games, or to filter them.

## Decision
`brief` now has a "Make it fun, unique and clear" section and says fun, uniqueness and clarity beat fidelity to the card: the agent may soften or drop the BUT, must skim `examples/` and not rebuild an existing mechanic, must show the goal, controls and progress on screen, teach one rule at a time, avoid hidden state, and check the game as a stranger using only what is on screen. Its last message also says why it is fun and how a first-time player learns it. `mark_authors` (called by `stage_games`) writes `"author": "ai"` for every game `publish` lists into RedEngineGames' `site/games-meta.json`; a slug with no author is human-designed there. Each game also needs a wiki page (`wiki_problems`, `build_wiki`): the agent writes overview, story, rules, controls, characters and gallery as data with `shot` specs plus `why_made` in its own words; `publish` draws the pictures (3+ distinct gallery pictures, one per character) and refuses to list a game whose page is missing or incomplete, because RedEngineGames' site build refuses it too.

## Consequences
Runs may depart from the literal card more often, and the idea-fit findings will say how. The brief is longer. The brief cannot guarantee a fun game: judge the next batch (the games published from 2026-10-18) and tighten it if needed. A game that never gets a valid page is never listed, so a careless run costs a game; the brief makes `wiki --check` a finish step. To undo, remove the section from `brief`; the `author` marking is independent and harmless.
