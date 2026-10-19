# 2026-10-18. Player reviews arrive anonymously through a public relay and are kept in the repo
Status: accepted
Summary: Players review published games (score out of 10, what they enjoyed and did not) anonymously from the website and the app via a public ntfy.sh topic; `scripts/player_reviews.py`, run every half hour by a workflow, keeps the valid ones in `docs/player-reviews/` for the site, the app, the Idea Forge and the nightly maintainer.

## Context
The owner wants players to leave a review on each game's wiki page, and wants the reviews in the engine's repo so the engine and the AI game schedule improve from them. It must be anonymous: no account for players and no third-party account for the owner. Both sites are static (GitHub Pages, and the app's page), so a review needs somewhere to go first. Options weighed:
- GitHub issues: these need each player to have a GitHub account.
- Google Forms, Cloudflare Workers, Supabase: each needs an account for the owner.
- An endpoint on the owner's box: this needs a router port, a domain and TLS, because a github.io page cannot post to plain HTTP.
- A token embedded in the app: anyone could extract it.

ntfy.sh takes anonymous HTTPS posts with CORS from a browser and from the app, and keeps messages for 12 hours. Anyone can read its topics back.

## Decision
The site's wiki and the app's game page post one JSON message per review to the topic named by `player_reviews.RELAY` (and RedEngineGames `distribution/config.json`). `.github/workflows/player-reviews.yml` runs `player_reviews.pull` every half hour, well inside the 12 hours.

`accept` keeps a message only when all of these hold:
- it is a v1 review of a slug in the site's catalog;
- the score is an integer from 1 to 10, and there is a write-up;
- it has no links, it fits the length limits, and it is not a duplicate.

Text keeps only visible characters: Unicode categories Cc, Cf, Co, Cs and Cn go, except a newline (this drops the tag characters U+E0000 to U+E007F, which draw nothing but can spell text an agent would read). At most 5 reviews of a game are kept per day (UTC), since each is permanent in git history. `pull` appends the kept reviews to `docs/player-reviews/reviews.jsonl`. `build` then writes `public.json`, which the site and the app read from `raw.githubusercontent.com`, and `SUMMARY.md`, minus the ids in `hidden.txt`. The workflow commits to `main` with the workflow token, so no other workflow starts.

The reviews reach two places:
- `players_section` adds the best- and worst-liked games, with short quotes, to every Idea Forge brief.
- The nightly maintainer reads `SUMMARY.md`, which holds scores and counts only. It pushes to `main`, so no player's text reaches it whole; the text stays in `public.json` and `reviews.jsonl`.

## Consequences
- Players need nothing, and reviews appear on the pages within about half an hour.
- Spam is possible; the checks drop junk and links, and the owner hides a review with one line in `hidden.txt`.
- Reviews are public, as they would be in this public repo anyway.
- If ntfy.sh is down, a review fails to send and the page says so. If the workflow does not run for 12 hours, the reviews posted in that time are lost.
- Review text reaches the Idea Forge brief only: visible characters, quoted, cut to 160 characters and labelled as opinion. (Amended 2026-10-18, same day: visible-only text, scores-only `SUMMARY.md`, the daily cap.)
- To leave ntfy.sh, run ntfy on any reachable host and change the topic URL in `player_reviews.RELAY` and `distribution/config.json`: the message format stays the same.
- To undo, delete the workflow and the `players_section` call; the record stays.
