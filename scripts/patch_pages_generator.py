#!/usr/bin/env python3
"""One-time integration: teach the RedEngineGames site generator (`site/generate.py` in that repository) to list the browser games
`red_engine2 publish --backend github-pages` commits under `webgames/`.

    python3 scripts/patch_pages_generator.py ../RedEngineGames/site/generate.py

It adds a "Play in your browser" section to the index, copies `webgames/` to `<site>/play/` and lists the games under `browser_games` in catalog.json.
Refuses to run twice. Review the diff in that repository and commit it there yourself (this tool never pushes). See docs/PUBLISHING_2D.md.
"""
import sys
p = sys.argv[1]
s = open(p, encoding='utf-8').read()
assert 'load_web_games' not in s, "already patched"
s = s.replace('''def human_size(n) -> str:''', '''WEB = REPO / "webgames"


def load_web_games() -> list[dict]:
    """Browser games published by `red_engine2 publish --backend github-pages`: webgames/catalog.json (schema red2d-catalog/1) beside webgames/games/<id>/."""
    catalog = WEB / "catalog.json"
    if not catalog.is_file():
        return []
    return json.loads(catalog.read_text(encoding="utf-8")).get("games", [])


def web_card(g: dict) -> str:
    page = f'play/{g["url"]}'
    inputs = " + ".join(g.get("input", []))
    return f\'\'\'
<article class="game">
  <a href="{e(page)}"><img class="thumb" src="play/{e(g["thumbnail"])}" alt="{e(g["title"])} screenshot" loading="lazy" width="640" height="360" style="image-rendering:pixelated"></a>
  <div class="body">
    <h3><a href="{e(page)}">{e(g["title"])}</a></h3>
    <p class="meta"><span>{e(g["presentation"].upper())} · {e(inputs)}</span><span>{e(g["build_timestamp"][:10])}</span></p>
    <p class="desc">{e(g["description"])}</p>
    <a class="dl" href="{e(page)}">Play in your browser</a>
    <div class="card-links"><a href="play/games/{e(g["id"])}/game.json">details</a></div>
  </div>
</article>\'\'\'


def web_section(web: list[dict]) -> str:
    if not web:
        return ""
    cards = "\\n".join(web_card(g) for g in web)
    return f\'\'\'
  <section class="web">
    <h2>Play in your browser</h2>
    <p class="fine">Nothing to install: these run in a web page and keep your best score in this browser. Each was played by a real headless browser before it was published (not by a human tester).</p>
    <section class="grid" id="webgames">
{cards}
    </section>
  </section>\'\'\'


def human_size(n) -> str:''', 1)
s = s.replace("def index_page(games: list[dict]) -> str:", "def index_page(games: list[dict], web: list[dict] | None = None) -> str:")
s = s.replace('''    cards = "\\n".join(card(g, i) for i, g in enumerate(games))
    body = f\'\'\'
  <section class="start">''', '''    cards = "\\n".join(card(g, i) for i, g in enumerate(games))
    body = f\'\'\'{web_section(web or [])}
  <section class="start">''', 1)
s = s.replace('''    (out / "index.html").write_text(index_page(games), encoding="utf-8")''', '''    web = load_web_games()
    if web:
        shutil.copytree(WEB, out / "play")
    (out / "index.html").write_text(index_page(games, web), encoding="utf-8")''', 1)
s = s.replace('''        {k: g[k] for k in ("slug", "name", "description", "added", "online", "versions", "earlier")} for g in games]}, indent=1), encoding="utf-8")''', '''        {k: g[k] for k in ("slug", "name", "description", "added", "online", "versions", "earlier")} for g in games],
        "browser_games": [{**g, "url": f'play/{g["url"]}'} for g in web]}, indent=1), encoding="utf-8")''', 1)
assert 'web_section(web or [])' in s and 'browser_games' in s and 'copytree(WEB' in s
open(p, 'w', encoding='utf-8').write(s)
print("patched", p)
