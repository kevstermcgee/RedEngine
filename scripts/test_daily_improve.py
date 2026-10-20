#!/usr/bin/env python3
"""Tests for scripts/daily_improve.py: what a daily pull request may change, what happens to an open one, the brief and the settings. No network,
no gh, no session: the pull requests and the inbox are given to the pure functions. Run directly: python3 scripts/test_daily_improve.py."""
import os
import string
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import daily_improve as di  # noqa: E402

MANIFEST = {"collections": {"games": [{"source": "examples/2d/snake.game2d.json", "destination": "snake"}, {"source": "examples/chrono-clash", "destination": "chrono-clash"},
                                      {"source": "examples/3d/drowsy-orchestra", "destination": "drowsy-orchestra"}]},
            "playables": [{"slug": "snake", "page": "pages/snake", "maintainer": "meastwood13"}, {"slug": "chrono-clash", "page": "pages/chrono-clash", "maintainer": "meastwood13"},
                          {"slug": "drowsy-orchestra"}]}
GREEN = [{"status": "COMPLETED", "conclusion": "SUCCESS"}, {"status": "COMPLETED", "conclusion": "SKIPPED"}]
RED = [{"status": "COMPLETED", "conclusion": "SUCCESS"}, {"status": "COMPLETED", "conclusion": "FAILURE"}]
RUNNING = [{"status": "IN_PROGRESS", "conclusion": ""}]


def pr(created="2026-10-20", checks=GREEN, files=("src/lib.rs",), draft=False, mergeable="MERGEABLE"):
    return {"number": 7, "createdAt": created + "T23:40:00Z", "statusCheckRollup": checks, "files": [{"path": f} for f in files], "isDraft": draft, "mergeable": mergeable}


class Refused(unittest.TestCase):
    def test_the_engine_s_ci_rules_versions_and_reviews_are_off_limits(self):
        bad = di.refused("engine", [".github/workflows/ci.yml", "scripts/daily_improve.md", "Cargo.toml", "crates/red_core/Cargo.toml", "Cargo.lock",
                                    "docs/player-reviews/reviews.jsonl", "src/tools/lint.rs", "docs/analysis/idea-forge/fixes.json"], MANIFEST)
        self.assertEqual(len(bad), 6, bad)
        self.assertFalse(any("lint.rs" in b or "fixes.json" in b for b in bad))

    def test_a_maintained_game_is_its_maintainer_s_and_an_ai_game_is_fair(self):
        bad = di.refused("engine", ["examples/2d/snake.game2d.json", "examples/2d/snake/build.js", "pages/chrono-clash/wiki.json", "examples/chrono-clash/speakeasy.json",
                                    "examples/3d/drowsy-orchestra/drowsy-orchestra.json", "examples/2d/snakes-and-ladders.game2d.json"], MANIFEST)
        self.assertEqual(len(bad), 4, bad)
        self.assertTrue(all("maintained game" in b for b in bad))

    def test_in_redenginegames_the_app_version_and_the_release_lists_are_off_limits(self):
        bad = di.refused("games", ["launcher/Cargo.toml", ".release-games.json", "projects/redline/maps/main.json", "games/snake/snake.game2d.json",
                                   "launcher/core/src/library.rs", "site/generate.py"])
        self.assertEqual(len(bad), 4, bad)


class Decide(unittest.TestCase):
    def test_green_merges_unless_merging_is_off(self):
        self.assertEqual(di.decide(pr(), "engine", True, "2026-10-21", MANIFEST)[0], "merge")
        self.assertEqual(di.decide(pr(), "engine", False, "2026-10-21", MANIFEST)[0], "hold")

    def test_running_checks_wait_and_no_checks_never_count_as_green(self):
        self.assertEqual(di.decide(pr(checks=RUNNING), "engine", True, "2026-10-21")[0], "wait")
        self.assertEqual(di.decide(pr(checks=[]), "engine", True, "2026-10-21")[0], "wait")

    def test_failed_or_conflicting_waits_for_a_maintainer_then_is_closed_when_stale(self):
        self.assertEqual(di.decide(pr(checks=RED), "engine", True, "2026-10-21")[0], "hold")
        self.assertEqual(di.decide(pr(checks=RED), "engine", True, "2026-10-24")[0], "close")
        self.assertEqual(di.decide(pr(mergeable="CONFLICTING"), "engine", True, "2026-10-21")[0], "hold")
        self.assertEqual(di.decide(pr(draft=True), "engine", True, "2026-10-21")[0], "hold")
        self.assertEqual(di.decide(pr(draft=True), "engine", True, "2026-10-23")[0], "close")

    def test_a_refused_path_is_never_merged_or_closed_by_the_improver(self):
        action, why = di.decide(pr(files=(".github/workflows/ci.yml",)), "engine", True, "2026-10-30", MANIFEST)
        self.assertEqual(action, "hold")
        self.assertIn("needs a maintainer", why)


class Brief(unittest.TestCase):
    INBOX = {"since": "2026-10-19", "maintainers": ["kevstermcgee", "meastwood13"], "unread": [], "backlog": [], "issues": [], "prs": [], "players": [],
             "engine_notes": [], "documents": []}

    def test_every_placeholder_is_filled_and_the_inbox_is_in_it(self):
        prs = {"engine": [dict(pr(checks=RED), title="Daily improver 2026-10-19: lint warns", headRefName="daily/2026-10-19")], "games": []}
        text = di.brief(self.INBOX, prs, engine_wt="/w/e", games_wt="/w/g", branch="daily/2026-10-20", report="/r/2026-10-20.md", scratch="/s", date="2026-10-20")
        self.assertNotIn("{", "".join(t[1] or "" for t in string.Formatter().parse(text)), "no placeholder left")
        self.assertIn("@kevstermcgee, @meastwood13", text)
        self.assertIn("# Feedback inbox (new since 2026-10-19", text)
        self.assertIn("kevstermcgee/RedEngine#7 (daily/2026-10-19, checks failed): Daily improver 2026-10-19: lint warns", text)
        self.assertIn('{"key": "<key>", "date": "2026-10-20"', text)
        self.assertIn("/w/g", text)

    def test_the_oldest_open_keys_are_named_for_tonight(self):
        backlog = [{"key": "new-one", "status": "open", "runs": ["2026-10-19-x"], "title": "t1"},
                   {"key": "old-fixed", "status": "addressed", "runs": ["2026-10-01-x"], "title": "t2"},
                   {"key": "old-open", "status": "recurring", "runs": ["2026-10-12-y", "2026-10-03-x"], "title": "t3"},
                   {"key": "mid", "status": "partial", "runs": ["2026-10-08-x"], "title": "t4"}]
        self.assertEqual(di.oldest_keys(backlog), "`old-open` (first reported 2026-10-03, recurring): t3; `mid` (first reported 2026-10-08, partial): t4")
        self.assertEqual(di.oldest_keys([]), "(no open key)")

    def test_the_title_comes_from_the_report_s_first_line(self):
        d = tempfile.mkdtemp()
        path = os.path.join(d, "r.md")
        with open(path, "w", encoding="utf-8") as f:
            f.write("\n# Lint warns when a swing rule cannot fire in a peaceful scene\n\nbody\n")
        self.assertEqual(di.report_title(path, "2026-10-20"), "Daily improver 2026-10-20: Lint warns when a swing rule cannot fire in a peaceful scene")
        self.assertEqual(di.report_title(os.path.join(d, "missing.md"), "2026-10-20"), "Daily improver 2026-10-20")


class Settings(unittest.TestCase):
    def test_the_env_file_then_the_environment_over_the_defaults(self):
        d = tempfile.mkdtemp()
        path = os.path.join(d, "env")
        with open(path, "w", encoding="utf-8") as f:
            f.write("# Settings\nNIGHTLY_MAX_USD=30          # cap\nNIGHTLY_PUSH_MAIN=0\n#NIGHTLY_MODEL=opus\n")
        cfg = di.load_config(path, env={"NIGHTLY_WAIT_MIN": "45"})
        self.assertEqual((cfg["NIGHTLY_MAX_USD"], cfg["NIGHTLY_PUSH_MAIN"], cfg["NIGHTLY_MODEL"], cfg["NIGHTLY_WAIT_MIN"], cfg["NIGHTLY_CLAUDE_TIMEOUT"]),
                         ("30", "0", "", "45", "3h"))

    def test_durations(self):
        self.assertEqual([di.seconds(x) for x in ("3h", "100m", "90s", "15")], [10800, 6000, 90, 15])
        with self.assertRaises(ValueError):
            di.seconds("soon")


if __name__ == "__main__":
    unittest.main()
