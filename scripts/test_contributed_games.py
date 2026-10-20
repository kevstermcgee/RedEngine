#!/usr/bin/env python3
"""The rules that mark a contributor's game pull request safe to merge (scripts/contributed_games.py `verdict`): run directly, python3 scripts/test_contributed_games.py.

What may be marked safe: a new game or an update, by a listed contributor, touching only content inside games they maintain, with its site page. What must
always reach the owner: engine files, someone else's game, a game claimed from its maintainer, its own client, launch arguments beyond its files, a missing
page, a name or a source that is taken, non-content files, large files, symlinks and removals.
"""
import copy
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import contributed_games as cg  # noqa: E402

ME = "cousin"
MAIN = {
    "version": 1,
    "target_repository": "x/games",
    "collections": {
        "games": [
            {"source": "examples/prop_hunt_yard.json", "destination": "prop-hunt"},
            {"source": "examples/chrono-clash", "destination": "chrono-clash"},
        ],
        "demos": [{"source": "examples", "destination": "examples"}],
    },
    "playables": [
        {"slug": "prop-hunt-yard", "name": "Prop Hunt Yard", "entry": "games/prop-hunt/prop_hunt_yard.json", "files": ["games/prop-hunt"], "arguments": ["content/games/prop-hunt/prop_hunt_yard.json"]},
        {"slug": "chrono-clash", "name": "Chrono Clash", "entry": "games/chrono-clash/speakeasy.json", "files": ["games/chrono-clash"],
         "arguments": ["--host", "content/games/chrono-clash/speakeasy.json"], "page": "pages/chrono-clash", "maintainer": ME},
    ],
}
ON_MAIN = {"examples/prop_hunt_yard.json", "examples/chrono-clash", "pages/chrono-clash", "examples"}
SNAKE = {"slug": "snake", "name": "Snake", "kind": "2d", "entry": "games/snake/snake.game2d.json", "files": ["games/snake"],
         "arguments": ["content/games/snake/snake.game2d.json"], "page": "pages/snake", "maintainer": ME}


def with_snake(**changes):
    head = copy.deepcopy(MAIN)
    head["collections"]["games"].append({"source": "examples/2d/snake.game2d.json", "destination": "snake"})
    head["playables"].append({**SNAKE, **changes})
    return head


SNAKE_FILES = [("added", "examples/2d/snake.game2d.json", None), ("added", "pages/snake/wiki.json", None), ("added", "pages/snake/g1.png", None),
               ("modified", "games-publish.json", None)]


def run(head, changed, author=ME, main=MAIN, taken=(), tree=None):
    return cg.verdict(author, [ME], main, main, head, changed, tree or {}, set(taken), lambda p: p in ON_MAIN)


class SafeToMerge(unittest.TestCase):
    def test_a_new_game_with_its_page(self):
        self.assertEqual(run(with_snake(), SNAKE_FILES), ([], ["snake"]))

    def test_an_update_to_their_own_game_a_new_map_and_a_new_picture(self):
        changed = [("modified", "examples/chrono-clash/speakeasy.json", None), ("added", "examples/chrono-clash/colosseum.json", None),
                   ("modified", "examples/chrono-clash/README.md", None), ("added", "pages/chrono-clash/g6.jpg", None), ("modified", "pages/chrono-clash/wiki.json", None)]
        self.assertEqual(run(copy.deepcopy(MAIN), changed), ([], ["chrono-clash"]))

    def test_a_2d_games_folder_with_its_javascript_generator(self):
        changed = SNAKE_FILES + [("added", "examples/2d/snake/build.js", None), ("added", "examples/2d/snake/README.md", None)]
        self.assertEqual(run(with_snake(), changed), ([], ["snake"]))

    def test_changing_their_own_games_launch_arguments_and_name(self):
        head = copy.deepcopy(MAIN)
        head["playables"][1].update(name="Chrono Clash: Eras", arguments=["--host", "content/games/chrono-clash/colosseum.json"])
        self.assertEqual(run(head, [("modified", "games-publish.json", None)])[0], [])


class NeedsTheOwner(unittest.TestCase):
    def problems(self, *a, **k):
        problems, _ = run(*a, **k)
        self.assertTrue(problems, "must reach the owner")
        return " | ".join(problems)

    def test_someone_not_on_the_list(self):
        self.assertIn("is not in", self.problems(with_snake(), SNAKE_FILES, author="stranger"))

    def test_an_engine_file(self):
        self.assertIn("`src/play2d.rs` is outside", self.problems(with_snake(), SNAKE_FILES + [("modified", "src/play2d.rs", None)]))

    def test_ci_or_the_contributor_list(self):
        self.assertIn("outside", self.problems(copy.deepcopy(MAIN), [("modified", ".github/contributors.json", None)]))

    def test_someone_elses_game(self):
        head = copy.deepcopy(MAIN)
        head["playables"][0]["name"] = "Mine now"
        self.assertIn("playable you do not maintain", self.problems(head, [("modified", "games-publish.json", None)]))
        self.assertIn("outside", self.problems(copy.deepcopy(MAIN), [("modified", "examples/prop_hunt_yard.json", None)]))

    def test_claiming_a_game_someone_else_maintains(self):
        head = copy.deepcopy(MAIN)
        head["playables"][0]["maintainer"] = ME
        self.assertIn("maintained by the owner", self.problems(head, [("modified", "games-publish.json", None), ("modified", "examples/prop_hunt_yard.json", None)]))

    def test_a_game_with_its_own_client(self):
        self.assertIn("engine code", self.problems(with_snake(binary="snake"), SNAKE_FILES))

    def test_launch_arguments_beyond_its_own_files(self):
        self.assertIn("not allowed", self.problems(with_snake(arguments=["--connect", "1.2.3.4:5", "content/games/snake/snake.game2d.json"]), SNAKE_FILES))
        self.assertIn("not allowed", self.problems(with_snake(arguments=["content/games/snake/../prop-hunt/x.json"]), SNAKE_FILES))

    def test_no_page(self):
        head = with_snake()
        del head["playables"][-1]["page"]
        self.assertIn("needs its page", self.problems(head, SNAKE_FILES))

    def test_a_taken_name_or_an_existing_source(self):
        self.assertIn("already used", self.problems(with_snake(), SNAKE_FILES, taken={"snake"}))
        head = copy.deepcopy(MAIN)
        head["collections"]["games"].append({"source": "examples/prop_hunt_yard.json", "destination": "mine"})
        head["playables"].append({**SNAKE, "slug": "mine", "kind": "3d", "entry": "games/mine/prop_hunt_yard.json", "files": ["games/mine"],
                                  "arguments": ["content/games/mine/prop_hunt_yard.json"], "page": "pages/mine"})
        self.assertIn("already exists on main", self.problems(head, [("modified", "games-publish.json", None), ("modified", "examples/prop_hunt_yard.json", None)]))

    def test_a_source_outside_a_game_folder(self):
        head = copy.deepcopy(MAIN)
        head["collections"]["games"].append({"source": "examples/external", "destination": "ext"})
        head["playables"].append({**SNAKE, "slug": "ext", "kind": "3d", "entry": "games/ext/x.json", "files": ["games/ext"], "arguments": [], "page": "pages/ext"})
        self.assertIn("must be", self.problems(head, [("modified", "games-publish.json", None)]))

    def test_moving_the_published_source(self):
        head = copy.deepcopy(MAIN)
        head["collections"]["games"][1]["source"] = "examples/chrono"
        self.assertIn("moving its source", self.problems(head, [("modified", "games-publish.json", None)]))

    def test_files_that_are_not_content(self):
        for path in ("examples/chrono-clash/sky.wgsl", "examples/chrono-clash/build.py", "pages/chrono-clash/run.exe", "pages/chrono-clash/x.js"):
            self.assertIn("only JSON, Markdown, PNG and JPEG", self.problems(copy.deepcopy(MAIN), [("added", path, None)]))
        self.assertIn("only JSON", self.problems(with_snake(), SNAKE_FILES + [("added", "examples/2d/snake/build.py", None)]), "CI runs Python generators")
        self.assertIn("outside", self.problems(with_snake(), SNAKE_FILES + [("added", "examples/2d/other/build.js", None)]))

    def test_a_large_file_or_a_symlink(self):
        big = {"examples/chrono-clash/huge.json": ("100644", 6 * 1024 * 1024)}
        self.assertIn("MB", self.problems(copy.deepcopy(MAIN), [("added", "examples/chrono-clash/huge.json", None)], tree=big))
        link = {"examples/chrono-clash/link.json": ("120000", 10)}
        self.assertIn("symlink", self.problems(copy.deepcopy(MAIN), [("added", "examples/chrono-clash/link.json", None)], tree=link))

    def test_a_rename_out_of_their_game(self):
        self.assertIn("outside", self.problems(copy.deepcopy(MAIN), [("renamed", "examples/chrono-clash/a.json", "examples/prop_hunt_yard.json")]))

    def test_the_rest_of_the_manifest(self):
        head = copy.deepcopy(MAIN)
        head["target_repository"] = "evil/games"
        self.assertIn("more than your games", self.problems(head, [("modified", "games-publish.json", None)]))
        head = copy.deepcopy(MAIN)
        head["collections"]["demos"] = []
        self.assertIn("more than your games", self.problems(head, [("modified", "games-publish.json", None)]))

    def test_removing_their_game(self):
        head = copy.deepcopy(MAIN)
        head["playables"].pop(1)
        head["collections"]["games"].pop(1)
        self.assertIn("removing `chrono-clash`", self.problems(head, [("modified", "games-publish.json", None)]))


if __name__ == "__main__":
    unittest.main(verbosity=1)
