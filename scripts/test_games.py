#!/usr/bin/env python3
"""Tests for scripts/games.py (forking a published game, the family tree) and the publisher's fork checks (scripts/publish_games.py). No network and no
git: the released files, the page and the catalog are given to the pure functions; the publisher runs on a temporary repository. Run directly:
python3 scripts/test_games.py."""
import copy
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import games  # noqa: E402
import publish_games as pg  # noqa: E402

SNAKE_GAME = '{\n "game2d": 1,\n "id": "snake",\n "title": "Snake",\n "sprites": {"snake": {"rows": ["#"]}},\n "hud": [{"text": "Snake Classic"}]\n}\n'
GENERATOR = "// node examples/2d/snake/build.js examples/2d/snake.game2d.json\nconst game = { game2d: 1, id: 'snake', title: 'Snake' };\n" \
            "const out = process.argv[2] || __dirname + '/snake.game2d.json';\n"
WIKI = {"tagline": "Classic snake.", "overview": "o" * 220, "story": "s" * 160, "goal": "Fill the board.", "rules": ["a", "b", "c"],
        "controls": [{"input": "Arrows", "action": "Turn"}], "characters": [{"name": "Snake", "role": "you", "description": "d", "image": "c1.png"}],
        "gallery": [{"image": "g1.png", "caption": "x"}], "prompt": "Make snake."}
PAGE = {"site/wiki/snake/wiki.json": json.dumps(WIKI).encode(), "site/wiki/snake/c1.png": b"\x89PNG\r\n\x1a\n1", "site/thumbs/snake.png": b"\x89PNG\r\n\x1a\nT"}
SNAKE = {"slug": "snake", "name": "Snake", "versions": [{"version": 3, "repo_commit": "c3"}, {"version": 1, "repo_commit": "c1"}]}
SNAKE_PLAYABLE = {"slug": "snake", "name": "Snake", "kind": "2d", "entry": "games/snake/snake.game2d.json", "files": ["games/snake"],
                  "arguments": ["content/games/snake/snake.game2d.json"], "page": "pages/snake", "maintainer": "meastwood13"}


def fork_snake(**kw):
    return games.plan_fork(SNAKE, SNAKE["versions"][0], SNAKE_PLAYABLE, {"games/snake/snake.game2d.json": SNAKE_GAME.encode()}, PAGE,
                           "neon-snake", "Neon Snake", "kevstermcgee", **kw)


class Fork2D(unittest.TestCase):
    def test_the_game_gets_its_own_id_and_title_and_nothing_else_moves(self):
        out, collection, playable = fork_snake()
        game = out["examples/2d/neon-snake.game2d.json"].decode()
        self.assertEqual(json.loads(game)["id"], "neon-snake")
        self.assertEqual(json.loads(game)["title"], "Neon Snake")
        self.assertEqual(game.replace('"neon-snake"', '"snake"').replace('"Neon Snake"', '"Snake"'), SNAKE_GAME, "only whole strings equal to the id or title change")
        self.assertIn('"neon-snake": {"rows"', game, "a name equal to the id is renamed everywhere it appears whole, so what refers to it still does")
        self.assertIn('"Snake Classic"', game, "a longer string that contains the title is kept")
        self.assertEqual(collection, {"source": "examples/2d/neon-snake.game2d.json", "destination": "neon-snake"})
        self.assertEqual(playable, {"slug": "neon-snake", "name": "Neon Snake", "kind": "2d", "entry": "games/neon-snake/neon-snake.game2d.json",
                                    "files": ["games/neon-snake"], "arguments": ["content/games/neon-snake/neon-snake.game2d.json"],
                                    "page": "pages/neon-snake", "maintainer": "kevstermcgee", "forked_from": {"slug": "snake", "version": 3}})

    def test_its_generator_comes_along_renamed_the_same_way(self):
        out, _, _ = fork_snake(generator={"examples/2d/snake/build.js": GENERATOR.encode(), "examples/2d/snake/art/tiles.json": b'{"snake": 1}'})
        gen = out["examples/2d/neon-snake/build.js"].decode()
        self.assertIn("id: 'neon-snake', title: 'Neon Snake'", gen)
        self.assertIn("node examples/2d/neon-snake/build.js examples/2d/neon-snake.game2d.json", gen)
        self.assertIn("'/neon-snake.game2d.json'", gen)
        self.assertEqual(out["examples/2d/neon-snake/art/tiles.json"], b'{"neon-snake": 1}', "what the generator reads beside it is renamed the same way")

    def test_the_page_starts_from_the_original_with_what_must_change_marked(self):
        out, _, _ = fork_snake()
        wiki = json.loads(out["pages/neon-snake/wiki.json"])
        self.assertTrue(wiki["tagline"].startswith(games.TODO) and wiki["prompt"].startswith(games.TODO) and wiki["overview"].startswith(games.TODO))
        self.assertIn("Snake v3", wiki["overview"])
        self.assertEqual(out["pages/neon-snake/thumb.png"], PAGE["site/thumbs/snake.png"])
        self.assertEqual(out["pages/neon-snake/c1.png"], PAGE["site/wiki/snake/c1.png"])

    def test_an_ai_games_page_becomes_a_person_s_fork(self):
        page = dict(PAGE, **{"site/wiki/snake/wiki.json": json.dumps(dict(WIKI, why_made="w" * 120, prompt=None)).encode()})
        out, _, _ = games.plan_fork(SNAKE, SNAKE["versions"][0], SNAKE_PLAYABLE, {"games/snake/snake.game2d.json": SNAKE_GAME.encode()}, page,
                                    "neon-snake", "Neon Snake", "kevstermcgee")
        wiki = json.loads(out["pages/neon-snake/wiki.json"])
        self.assertNotIn("why_made", wiki)
        self.assertTrue(wiki["prompt"].startswith(games.TODO))


class Fork3D(unittest.TestCase):
    PLAYABLE = {"slug": "drowsy-marathon", "name": "Drowsy Marathon", "files": ["projects/drowsy-marathon"],
                "arguments": ["--host", "content/projects/drowsy-marathon/drowsy-marathon.json"]}
    FILES = {"projects/drowsy-marathon/drowsy-marathon.json": b'{"audio": {"music": "projects/drowsy-marathon/lullaby.json"}}',
             "projects/drowsy-marathon/lullaby.json": b"{}", "projects/drowsy-marathon/README.md": b"Run while sleepy.\n",
             "projects/drowsy-marathon/STATUS.md": b"old notes", "projects/drowsy-marathon/scripts/red": b"#!/bin/sh",
             "projects/drowsy-marathon/game.json": b'{"game": 1, "id": "drowsy-marathon", "name": "drowsy-marathon", "engine": {"path": "../../../RedEngine"}}'}

    def test_a_3d_game_folder_moves_whole_into_examples_with_its_paths(self):
        game = {"slug": "drowsy-marathon", "name": "Drowsy Marathon"}
        page = {"site/wiki/drowsy-marathon/wiki.json": json.dumps(WIKI).encode()}
        out, collection, playable = games.plan_fork(game, {"version": 2}, self.PLAYABLE, self.FILES, page, "lively-marathon", "Lively Marathon", "meastwood13")
        self.assertEqual(collection, {"source": "examples/lively-marathon", "destination": "lively-marathon"})
        self.assertEqual(playable["entry"], "games/lively-marathon/drowsy-marathon.json")
        self.assertEqual(playable["arguments"], ["--host", "content/games/lively-marathon/drowsy-marathon.json"])
        self.assertEqual(json.loads(out["examples/lively-marathon/drowsy-marathon.json"])["audio"]["music"], "games/lively-marathon/lullaby.json")
        self.assertTrue(out["examples/lively-marathon/README.md"].startswith(games.TODO.encode()), "the README's first paragraph is the site's description")
        self.assertNotIn("examples/lively-marathon/STATUS.md", out, "the original's own development notes stay with it")
        self.assertNotIn("examples/lively-marathon/scripts/red", out, "a shim that ran a sibling engine has no use inside the engine")
        project = json.loads(out["examples/lively-marathon/game.json"])
        self.assertEqual((project["id"], project["engine"]["path"]), ("lively-marathon", "../.."))

    def test_a_game_with_its_own_client_or_many_folders_is_refused_with_the_reason(self):
        with self.assertRaisesRegex(games.ForkError, "engine code"):
            games.plan_fork({"slug": "killchain", "name": "Killchain"}, {"version": 4}, dict(self.PLAYABLE, binary="killchain"), {}, {}, "kc2", "KC2", "x")
        with self.assertRaisesRegex(games.ForkError, "a fork copies one"):
            games.plan_fork({"slug": "d", "name": "D"}, {"version": 1}, dict(self.PLAYABLE, files=["a", "b"]), {}, {}, "d2", "D2", "x")


class Manifest(unittest.TestCase):
    TEXT = ('{\n  "version": 1,\n  "collections": {\n    "games": [\n      {"source": "examples/a", "destination": "a"},\n      {"source": "examples/b", "destination": "b"}\n'
            '    ],\n    "demos": [\n      {"source": "examples", "destination": "examples"}\n    ]\n  },\n  "playables": [\n    {\n      "slug": "a",\n'
            '      "name": "A",\n      "files": ["games/a"]\n    }\n  ]\n}\n')

    def test_the_fork_is_two_new_entries_and_nothing_else(self):
        playable = {"slug": "c", "name": "C", "files": ["games/c"], "arguments": ["content/games/c/c.json"], "forked_from": {"slug": "a", "version": 2}}
        text = games.add_to_manifest(self.TEXT, {"source": "examples/c", "destination": "c"}, playable)
        self.assertIn('      {"source": "examples/b", "destination": "b"},\n      {"source": "examples/c", "destination": "c"}\n    ],', text)
        self.assertIn('      "forked_from": {"slug": "a", "version": 2}\n    }\n  ]\n}\n', text)
        self.assertEqual(json.loads(text)["playables"][-1], playable)
        self.assertEqual(self.TEXT.count("\n") + 1 + len(playable) + 2, text.count("\n"), "only the new lines")


class Tree(unittest.TestCase):
    CATALOG = [{"slug": "snake", "name": "Snake", "versions": [{"version": 3}], "maintainer": "meastwood13"},
               {"slug": "neon-snake", "name": "Neon Snake", "versions": [{"version": 1, "forked_from": {"slug": "snake", "version": 3}}]},
               {"slug": "night-snake", "name": "Night Snake", "forked_from": {"slug": "neon-snake", "version": 1}, "versions": [{"version": 2}]},
               {"slug": "wormhole", "name": "Wormhole", "versions": [{"version": 2}]}]

    def test_a_family_reads_from_the_first_ancestor_down(self):
        lines = games.tree_lines(self.CATALOG, "night-snake")
        self.assertEqual(lines, ["Snake `snake` v3 by @meastwood13", "  └ Neon Snake `neon-snake` v1 (from snake v3)", "    └ Night Snake `night-snake` v2 (from neon-snake v1)"])

    def test_the_whole_tree_shows_families_and_counts_the_rest(self):
        lines = games.tree_lines(self.CATALOG)
        self.assertEqual(lines[0], "Snake `snake` v3 by @meastwood13")
        self.assertEqual(lines[-1], "(1 more game(s) with no forks yet: `games.py list`)")


class Publisher(unittest.TestCase):
    def repo(self, playable_extra, page_text):
        root = Path(tempfile.mkdtemp(prefix="games_fork_pub_"))
        (root / "examples").mkdir()
        (root / "examples" / "c.json").write_text("{}\n")
        page = root / "pages" / "c"
        page.mkdir(parents=True)
        (page / "wiki.json").write_text(page_text)
        manifest = {"version": 1, "source_repository": "x/e", "target_repository": "x/g",
                    "collections": {c: [{"source": "examples/c.json", "destination": c}] for c in pg.CATEGORIES},
                    "playables": [{"slug": "c", "name": "C", "entry": "games/games/c.json", "files": ["games/games"], "arguments": [], "page": "pages/c", **playable_extra}]}
        (root / "games-publish.json").write_text(json.dumps(manifest))
        return root

    def test_forked_from_names_a_game_and_a_version(self):
        for bad in ({"slug": "c", "version": 1}, {"slug": "a"}, {"slug": "a", "version": 0}, {"slug": "A b", "version": 1}, "a@1", {"slug": "a", "version": True}):
            with self.assertRaisesRegex(pg.PublishError, "forked_from"):
                pg.load_manifest(self.repo({"forked_from": bad}, "{}"))
        pg.load_manifest(self.repo({"forked_from": {"slug": "a", "version": 2}}, "{}"))

    def test_an_unfinished_fork_is_never_published(self):
        root = self.repo({"forked_from": {"slug": "a", "version": 2}}, json.dumps({"tagline": games.TODO + ": one sentence"}))
        with self.assertRaisesRegex(pg.PublishError, r"site/wiki/c/wiki.json still says TODO\(fork\)"):
            pg.check(root, "rev")
        (root / "pages" / "c" / "wiki.json").write_text(json.dumps({"tagline": "Done."}))
        pg.check(root, "rev")

    def test_a_parent_no_one_publishes_is_refused_when_the_games_repo_is_known(self):
        games_repo = Path(tempfile.mkdtemp(prefix="games_repo_"))
        (games_repo / ".release-games.json").write_text(json.dumps({"data_playables": [{"slug": "a"}]}))
        catalog = {"playables": [{"slug": "c", "forked_from": {"slug": "zz", "version": 1}}]}
        self.assertEqual(pg.fork_problems(games_repo, catalog, games_repo), ["c: forked_from `zz`, which neither this manifest nor RedEngineGames publishes"])
        catalog["playables"][0]["forked_from"]["slug"] = "a"
        self.assertEqual(pg.fork_problems(games_repo, catalog, games_repo), [])


if __name__ == "__main__":
    unittest.main()
