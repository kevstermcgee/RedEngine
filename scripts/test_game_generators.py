#!/usr/bin/env python3
"""Tests for scripts/game_generators.py (run by `scripts/ci.sh pytools`, or directly: python3 scripts/test_game_generators.py).

Each test builds a throwaway checkout with an `examples/2d` or `examples/3d` game and a generator beside it; the generators write bytes, so the
result is the same on Windows (no newline translation).
"""
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import game_generators  # noqa: E402

WRITE_3D = "from pathlib import Path\n(Path(__file__).resolve().parent / 'x.json').write_bytes(b'{\"objects\": []}\\n')\n"


class Generators(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="game_gen_"))

    def put(self, rel, data):
        p = self.root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data.encode() if isinstance(data, str) else data)
        return p

    def check(self, kind="3d", slug="x"):
        pair = game_generators.for_game(self.root, kind, slug)
        self.assertIsNotNone(pair)
        return game_generators.check_one(*pair, root=self.root)

    def test_a_generator_that_rewrites_its_game_passes(self):
        self.put("examples/3d/x/x.json", b'{"objects": []}\n')
        self.put("examples/3d/x/build.py", WRITE_3D)
        ok, _secs, problems = self.check()
        self.assertTrue(ok, problems)

    def test_a_2d_generator_beside_its_game_file_may_import_its_helpers(self):
        self.put("examples/2d/hb.game2d.json", b'{"game2d": 1}\n')
        self.put("examples/2d/hb/levels.py", "DATA = b'{\"game2d\": 1}\\n'\n")
        self.put("examples/2d/hb/gen.py", "import levels\n")   # build.py wins over gen.py: gen.py is a helper here
        self.put("examples/2d/hb/build.py", "from pathlib import Path\nimport levels\n(Path(__file__).resolve().parent.parent / 'hb.game2d.json').write_bytes(levels.DATA)\n")
        self.assertEqual(game_generators.for_game(self.root, "2d", "hb")[0].name, "build.py")
        ok, _secs, problems = self.check("2d", "hb")
        self.assertTrue(ok, problems)

    def test_a_generator_whose_output_differs_names_the_file_and_the_line(self):
        self.put("examples/3d/x/x.json", b'{"objects": [1]}\n')
        self.put("examples/3d/x/build.py", WRITE_3D)
        ok, _secs, problems = self.check()
        self.assertFalse(ok)
        self.assertIn("examples/3d/x/build.py: writes a different examples/3d/x/x.json than the committed one (first difference at line 1)", problems[0])

    def test_a_generator_that_writes_nothing_or_elsewhere_fails(self):
        self.put("examples/3d/x/x.json", b"{}\n")
        self.put("examples/3d/x/build.py", "print('hello')\n")
        ok, _secs, problems = self.check()
        self.assertFalse(ok)
        self.assertIn("did not write examples/3d/x/x.json", problems[0])
        self.assertIn("Path(__file__).parent", problems[0], "the fix: a path relative to the script (Quiet Compass wrote to an absolute path in a deleted worktree)")

    def test_a_file_it_writes_but_nobody_committed_fails(self):
        self.put("examples/3d/x/x.json", b'{"objects": []}\n')
        self.put("examples/3d/x/build.py", WRITE_3D + "(Path(__file__).resolve().parent / 'extra.json').write_bytes(b'{}')\n")
        ok, _secs, problems = self.check()
        self.assertFalse(ok)
        self.assertEqual(problems, ["examples/3d/x/build.py: writes examples/3d/x/extra.json, which is not committed (commit it, or stop writing it)"])

    def test_a_generator_that_fails_reports_its_last_lines(self):
        self.put("examples/3d/x/x.json", b"{}\n")
        self.put("examples/3d/x/build.py", "raise SystemExit('no levels')\n")
        ok, _secs, problems = self.check()
        self.assertFalse(ok)
        self.assertIn("exited 1", problems[0])
        self.assertIn("no levels", problems[0])

    def test_hash_order_is_fixed_so_a_set_writes_the_same_way_every_run(self):
        words = "{'kiwi', 'fig', 'plum', 'date', 'lime', 'pear'}"
        self.put("examples/3d/x/build.py", f"from pathlib import Path\n(Path(__file__).resolve().parent / 'x.json').write_bytes(','.join({words}).encode())\n")
        once = subprocess.run([sys.executable, "-c", f"print(','.join({words}), end='')"], env=dict(os.environ, PYTHONHASHSEED="0"), capture_output=True, text=True).stdout
        self.put("examples/3d/x/x.json", once.encode())
        ok, _secs, problems = self.check()
        self.assertTrue(ok, problems)

    def test_a_game_with_no_generator_has_none_and_the_cli_says_so(self):
        self.put("examples/3d/y/y.json", b"{}\n")
        self.assertIsNone(game_generators.for_game(self.root, "3d", "y"))
        self.assertEqual(game_generators.find(self.root), [])


if __name__ == "__main__":
    unittest.main()
