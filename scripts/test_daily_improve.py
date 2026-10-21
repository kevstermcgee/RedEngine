#!/usr/bin/env python3
"""Tests for scripts/daily_improve.py: merge/recovery decisions, file guards, the brief, settings, shim locking and real conflict repairs against
a temporary local Git remote. No network, gh or model session. Run directly: python3 scripts/test_daily_improve.py."""
import os
import json
import string
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

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
    return {"number": 7, "createdAt": created + "T23:40:00Z", "statusCheckRollup": checks, "files": [{"path": f} for f in files], "isDraft": draft,
            "mergeable": mergeable, "headRefOid": "a" * 40, "headRefName": "daily/2026-10-20"}


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
        self.assertEqual(di.decide(pr(checks=[]), "engine", True, "2026-10-20")[0], "wait")
        self.assertEqual(di.decide(pr(checks=[]), "games", True, "2026-10-21")[0], "hold")

    def test_failed_or_conflicting_work_is_preserved_for_recovery(self):
        self.assertEqual(di.decide(pr(checks=RED), "engine", True, "2026-10-21")[0], "hold")
        self.assertEqual(di.decide(pr(checks=RED), "engine", True, "2026-10-24")[0], "hold")
        self.assertEqual(di.decide(pr(mergeable="CONFLICTING"), "engine", True, "2026-10-21")[0], "hold")
        self.assertEqual(di.decide(pr(draft=True), "engine", True, "2026-10-21")[0], "hold")
        self.assertEqual(di.decide(pr(draft=True), "engine", True, "2026-10-23")[0], "hold")

    def test_a_refused_path_is_never_merged_or_closed_by_the_improver(self):
        action, why = di.decide(pr(files=(".github/workflows/ci.yml",)), "engine", True, "2026-10-30", MANIFEST)
        self.assertEqual(action, "hold")
        self.assertIn("needs a maintainer", why)

    def test_unknown_mergeability_never_merges_and_behind_updates_first(self):
        self.assertEqual(di.decide(pr(mergeable="UNKNOWN"), "engine", True, "2026-10-21")[0], "wait")
        self.assertEqual(di.decide(dict(pr(), mergeStateStatus="BEHIND"), "engine", True, "2026-10-21")[0], "update")

    def test_a_legacy_local_failure_draft_recovers_only_on_green_hosted_ci(self):
        body = "Local checks failed: affected failed. Opened by the daily improver. It will not merge itself."
        self.assertEqual(di.decide(dict(pr(draft=True), body=body), "engine", True, "2026-10-21")[0], "merge")
        self.assertEqual(di.decide(dict(pr(draft=True, checks=RED), body=body), "engine", True, "2026-10-21")[0], "hold")
        self.assertEqual(di.decide(pr(draft=True), "engine", True, "2026-10-21")[0], "hold")
        new_body = body.replace("It will not merge itself.", "It merges itself once every hosted check passes")
        self.assertEqual(di.decide(dict(pr(draft=True), body=new_body), "engine", True, "2026-10-21")[0], "hold")


class Recovery(unittest.TestCase):
    def test_merges_are_pinned_to_the_inspected_head(self):
        with mock.patch.object(di, "gh", return_value="behind") as gh:
            di.act(pr(), "engine", "merge", "green")
        self.assertIn("--match-head-commit", gh.call_args.args)
        self.assertIn("a" * 40, gh.call_args.args)

    def test_legacy_draft_is_marked_ready_before_merging(self):
        with mock.patch.object(di, "gh", return_value="behind") as gh:
            di.act(pr(draft=True), "engine", "merge", "green")
        self.assertEqual(gh.call_args_list[-2].args[:2], ("pr", "ready"))
        self.assertEqual(gh.call_args_list[-1].args[:2], ("pr", "merge"))

    def test_main_moving_under_a_clean_pr_updates_instead_of_merging(self):
        with mock.patch.object(di, "gh", side_effect=["diverged", "updated"]) as gh:
            self.assertIn("waiting for checks", di.act(pr(), "engine", "merge", "green"))
        self.assertIn("expected_head_sha=" + "a" * 40, gh.call_args.args)
        self.assertFalse(any(c.args[:2] == ("pr", "merge") for c in gh.call_args_list))

    def test_unrecognized_ancestry_cannot_authorize_a_merge(self):
        with mock.patch.object(di, "gh", return_value="unknown"):
            with self.assertRaises(RuntimeError):
                di.act(pr(), "engine", "merge", "green")

    def test_listing_failure_is_not_treated_as_an_empty_queue(self):
        with mock.patch.object(di, "gh", side_effect=RuntimeError("offline")):
            with self.assertRaises(RuntimeError):
                di.open_daily_prs(di.REPOS["engine"])

    def test_all_file_pages_and_renamed_sources_are_checked(self):
        candidate = dict(pr(), baseRefName="main", isCrossRepository=False, author={"login": "kevstermcgee"}, changedFiles=2)
        fork = dict(candidate, isCrossRepository=True)
        other = dict(candidate, author={"login": "someone-else"})
        pages = '\n'.join(json.dumps(row) for row in [["src/ok.rs", None], ["moved.txt", ".github/workflows/ci.yml"]])
        with mock.patch.object(di, "gh", side_effect=[json.dumps([candidate, fork, other]), pages]) as gh:
            prs = di.open_daily_prs(di.REPOS["engine"])
        self.assertEqual(len(prs), 1)
        self.assertEqual(len(prs[0]["files"]), 3)
        self.assertIn("--paginate", gh.call_args.args)
        self.assertEqual(di.decide(prs[0], "engine", True, "2026-10-21")[0], "hold")

    def test_incomplete_file_lists_and_missing_ownership_stop_the_operation(self):
        candidate = dict(pr(), baseRefName="main", isCrossRepository=False, author={"login": "kevstermcgee"}, changedFiles=2)
        with mock.patch.object(di, "gh", side_effect=[json.dumps([candidate]), json.dumps(["src/ok.rs", None])]):
            with self.assertRaises(RuntimeError):
                di.open_daily_prs(di.REPOS["engine"])
        with mock.patch.object(di, "git", side_effect=RuntimeError("missing manifest")):
            with self.assertRaises(RuntimeError):
                di.manifest_on_main("checkout")

    def test_one_ci_retry_per_head_survives_restarts_and_ignores_other_heads(self):
        runs = {"workflow_runs": [{"id": 1, "head_sha": "a" * 40, "conclusion": "failure", "event": "pull_request"},
                                  {"id": 2, "head_sha": "b" * 40, "conclusion": "failure", "event": "pull_request"},
                                  {"id": 3, "head_sha": "a" * 40, "conclusion": "failure", "event": "workflow_dispatch", "path": ".github/workflows/releases.yml"}]}
        with tempfile.TemporaryDirectory() as state, mock.patch.object(di, "STATE", state), mock.patch.object(di, "gh", return_value=json.dumps(runs)) as gh:
            self.assertTrue(di.retry_checks(pr(checks=RED), "engine"))
            self.assertFalse(di.retry_checks(pr(checks=RED), "engine"))
            writes = [c for c in gh.call_args_list if "POST" in c.args]
            self.assertEqual(len(writes), 1)
            self.assertIn("/1/rerun-failed-jobs", writes[0].args[-1])

    def test_docs_without_a_pr_workflow_get_one_read_only_dispatch_per_head(self):
        with tempfile.TemporaryDirectory() as state, mock.patch.object(di, "STATE", state), mock.patch.object(di, "gh") as gh:
            self.assertTrue(di.dispatch_missing_checks(pr(checks=[], files=("README.md",)), "games"))
            self.assertFalse(di.dispatch_missing_checks(pr(checks=[], files=("README.md",)), "games"))
            self.assertFalse(di.dispatch_missing_checks(pr(checks=[]), "engine"))
            self.assertFalse(di.dispatch_missing_checks(pr(checks=RUNNING), "games"))
        gh.assert_called_once_with("workflow", "run", "checks.yml", "-R", di.REPOS["games"], "--ref", "daily/2026-10-20")

    def test_normal_workflows_have_five_minutes_to_appear(self):
        fresh = dict(pr(checks=[], files=("README.md",)), createdAt=di.datetime.datetime.now(di.datetime.timezone.utc).isoformat())
        with mock.patch.object(di, "gh") as gh:
            self.assertFalse(di.dispatch_missing_checks(fresh, "games"))
        gh.assert_not_called()

    def test_a_transient_dispatch_error_does_not_strand_the_pr_forever(self):
        with tempfile.TemporaryDirectory() as state, mock.patch.object(di, "STATE", state), mock.patch.object(di, "gh", side_effect=[RuntimeError("offline"), "queued"]) as gh:
            with self.assertRaises(RuntimeError):
                di.dispatch_missing_checks(pr(checks=[], files=("README.md",)), "games")
            self.assertTrue(di.dispatch_missing_checks(pr(checks=[], files=("README.md",)), "games"))
            self.assertFalse(di.dispatch_missing_checks(pr(checks=[], files=("README.md",)), "games"))
        self.assertEqual(gh.call_count, 2)

    def test_missing_code_or_launcher_checks_cannot_be_replaced_by_site_validation(self):
        with mock.patch.object(di, "gh") as gh:
            for path in ("launcher/core/src/library.rs", "launcher/README.md", "site/generate.py", "distribution/upload_game.py", "docs/GAME_METADATA.md"):
                self.assertFalse(di.dispatch_missing_checks(pr(checks=[], files=(path,)), "games"))
        gh.assert_not_called()

    def test_fallback_checks_can_retry_but_release_dispatches_cannot(self):
        runs = {"workflow_runs": [{"id": 4, "head_sha": "a" * 40, "conclusion": "failure", "event": "workflow_dispatch", "path": ".github/workflows/checks.yml"},
                                  {"id": 5, "head_sha": "a" * 40, "conclusion": "failure", "event": "workflow_dispatch", "path": ".github/workflows/releases.yml"}]}
        with tempfile.TemporaryDirectory() as state, mock.patch.object(di, "STATE", state), mock.patch.object(di, "gh", return_value=json.dumps(runs)) as gh:
            self.assertTrue(di.retry_checks(pr(checks=RED), "games"))
        posts = [c for c in gh.call_args_list if "POST" in c.args]
        self.assertEqual(len(posts), 1)
        self.assertTrue(posts[0].args[-1].endswith("/4/rerun-failed-jobs"))

    def test_dispatching_a_check_is_never_itself_permission_to_merge(self):
        with tempfile.TemporaryDirectory() as state, mock.patch.object(di, "STATE", state), mock.patch.object(di, "manifest_on_main", return_value=MANIFEST), \
                mock.patch.object(di, "open_daily_prs", side_effect=[[], [pr(checks=[], files=("README.md",))]]), mock.patch.object(di, "gh") as gh:
            lines = di.settle(di.DEFAULTS, "2026-10-21")
        self.assertIn("dispatched read-only hosted checks", lines[0])
        self.assertFalse(any(c.args[:2] == ("pr", "merge") for c in gh.call_args_list))

    def test_closed_pr_is_not_merged_from_a_stale_snapshot(self):
        with mock.patch.object(di, "manifest_on_main", return_value=MANIFEST), mock.patch.object(di, "open_daily_prs", side_effect=[[pr(checks=RUNNING)], [], []]), \
                mock.patch.object(di.time, "sleep"), mock.patch.object(di, "act") as act:
            self.assertEqual(di.settle(di.DEFAULTS, "2026-10-21", wait_min=1), [])
        act.assert_not_called()

    def test_merge_off_performs_no_recovery_writes(self):
        cfg = dict(di.DEFAULTS, NIGHTLY_PUSH_MAIN="0")
        with mock.patch.object(di, "manifest_on_main", return_value=MANIFEST), mock.patch.object(di, "open_daily_prs", side_effect=[[pr(checks=RED)], []]), \
                mock.patch.object(di, "retry_checks") as retry, mock.patch.object(di, "gh") as gh:
            lines = di.settle(cfg, "2026-10-30")
        self.assertIn("merging is off", lines[0])
        retry.assert_not_called()
        gh.assert_not_called()

    def test_stale_failed_work_is_preserved_for_repair(self):
        with mock.patch.object(di, "manifest_on_main", return_value=MANIFEST), mock.patch.object(di, "open_daily_prs", side_effect=[[pr(checks=RED)], []]), \
                mock.patch.object(di, "retry_checks", return_value=False), mock.patch.object(di, "gh") as gh:
            lines = di.settle(di.DEFAULTS, "2026-10-30")
        self.assertIn("automatic recovery pending", lines[0])
        gh.assert_not_called()

    def test_local_failure_still_opens_a_ready_pr_and_uses_hosted_ci(self):
        def git(wt, *args, **kwargs):
            return "1" if args[0] == "rev-list" else ""
        with tempfile.TemporaryDirectory() as d, mock.patch.object(di, "git", side_effect=git) as git_mock, \
                mock.patch.object(di, "changed_paths", return_value=["src/ok.rs"]), mock.patch.object(di, "local_checks", return_value=(False, "flaky local check")), \
                mock.patch.object(di, "gh", return_value="https://github.com/kevstermcgee/RedEngine/pull/123\n") as gh, mock.patch.object(di, "settle", return_value=["merged"]) as settle:
            result = di.land("engine", d, "daily/2026-10-21", os.path.join(d, "missing-report"), "2026-10-21", di.DEFAULTS, MANIFEST, os.path.join(d, "log"))
        self.assertNotIn("--draft", gh.call_args.args)
        self.assertIn("Hosted CI is authoritative", gh.call_args.args[gh.call_args.args.index("--body") + 1])
        self.assertFalse(any("--force" in c.args for c in git_mock.call_args_list))
        settle.assert_called_once()
        self.assertIn("merged", result)

    def test_refused_work_stays_a_draft_despite_green_checks(self):
        with tempfile.TemporaryDirectory() as d, mock.patch.object(di, "git", side_effect=["", "1", ""]), \
                mock.patch.object(di, "changed_paths", return_value=[".github/workflows/ci.yml"]), mock.patch.object(di, "local_checks") as checks, \
                mock.patch.object(di, "gh", return_value="https://github.com/kevstermcgee/RedEngine/pull/123\n") as gh, mock.patch.object(di, "settle") as settle:
            di.land("engine", d, "daily/2026-10-21", os.path.join(d, "missing"), "2026-10-21", di.DEFAULTS, MANIFEST, os.path.join(d, "log"))
        self.assertIn("--draft", gh.call_args.args)
        checks.assert_not_called()
        settle.assert_not_called()

    def test_repair_sessions_are_bounded_to_one_per_night_and_two_per_pr(self):
        with tempfile.TemporaryDirectory() as state, mock.patch.object(di, "STATE", state), mock.patch.object(di, "manifest_on_main", return_value=MANIFEST), \
                mock.patch.object(di, "open_daily_prs", return_value=[pr(checks=RED)]), mock.patch.object(di, "repair_pr", return_value="repaired") as repair:
            self.assertEqual(di.repair_one(di.DEFAULTS, "2026-10-21"), ["repaired"])
            self.assertEqual(di.repair_one(di.DEFAULTS, "2026-10-22"), ["repaired"])
            # There are two repositories, and each has its own PR number namespace.
            self.assertEqual(di.repair_one(di.DEFAULTS, "2026-10-23"), ["repaired"])
            self.assertEqual(di.repair_one(di.DEFAULTS, "2026-10-24"), ["repaired"])
            self.assertEqual(di.repair_one(di.DEFAULTS, "2026-10-25"), [])
            self.assertEqual(repair.call_count, 4)


class RepairIntegration(unittest.TestCase):
    """Exercise actual merges and guarded pushes against a local bare remote, with only the agent and GitHub reads stubbed."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = self.tmp.name
        self.remote = os.path.join(self.base, "remote.git")
        self.clone = os.path.join(self.base, "clone")
        self.env_patch = mock.patch.dict(os.environ, {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"})
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)
        self.git("init", "--bare", "--initial-branch=main", self.remote)
        self.git("clone", self.remote, self.clone)
        self.git("-C", self.clone, "config", "user.name", "Nightly test")
        self.git("-C", self.clone, "config", "user.email", "test@example.test")
        self.write("src/fix.txt", "original\n")
        self.commit("base")
        self.git("-C", self.clone, "push", "origin", "main")
        self.git("-C", self.clone, "checkout", "-b", "daily/2026-10-20")
        self.write("src/fix.txt", "nightly change\n")
        self.commit("daily fix")
        self.head = self.git("-C", self.clone, "rev-parse", "HEAD")
        self.git("-C", self.clone, "push", "origin", "daily/2026-10-20")
        self.git("-C", self.clone, "checkout", "main")
        self.write("src/fix.txt", "main change\n")
        self.commit("main moved")
        self.main = self.git("-C", self.clone, "rev-parse", "HEAD")
        self.git("-C", self.clone, "push", "origin", "main")

    def git(self, *args):
        p = subprocess.run(["git", *args], capture_output=True, text=True, timeout=10)
        self.assertEqual(p.returncode, 0, p.stderr)
        return p.stdout.strip()

    def write(self, relative, text, wt=None):
        path = os.path.join(wt or self.clone, relative)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)

    def commit(self, message, wt=None):
        self.git("-C", wt or self.clone, "add", ".")
        self.git("-C", wt or self.clone, "commit", "-m", message)

    def agent(self, argv, wt, *args, **kwargs):
        with open(os.path.join(wt, "src", "fix.txt"), encoding="utf-8") as f:
            self.assertIn("<<<<<<<", f.read())
        self.write("src/fix.txt", "main change\nnightly change\n", wt)
        if getattr(self, "forbidden", False):
            self.write(".github/workflows/ci.yml", "weakened", wt)
        self.commit("resolve conflict without losing either change", wt)
        return {"status": "completed"}

    def repair(self, state="OPEN", sha=None, agent=None):
        with mock.patch.object(di, "ROOT", self.clone), mock.patch.object(di, "STATE", self.base), mock.patch.object(di, "REPORTS", self.base), \
                mock.patch.object(di.idea_forge, "stream_agent", side_effect=agent or self.agent), \
                mock.patch.object(di, "gh", return_value=json.dumps({"state": state, "headRefOid": sha or self.head})):
            return di.repair_pr(dict(pr(mergeable="CONFLICTING"), headRefOid=self.head), "engine", di.DEFAULTS, "2026-10-21", MANIFEST)

    def remote_head(self):
        return self.git("--git-dir", self.remote, "rev-parse", "refs/heads/daily/2026-10-20")

    def test_conflict_repair_preserves_both_histories_and_waits_for_new_ci(self):
        result = self.repair()
        head = self.remote_head()
        self.assertNotEqual(head, self.head)
        for ancestor in (self.main, self.head):
            self.assertEqual(self.git("--git-dir", self.remote, "merge-base", ancestor, head), ancestor)
        self.assertEqual(self.git("--git-dir", self.remote, "show", head + ":src/fix.txt"), "main change\nnightly change")
        self.assertIn("only after hosted checks pass", result)

    def test_forbidden_repair_does_not_push(self):
        self.forbidden = True
        self.assertIn("repair refused", self.repair())
        self.assertEqual(self.remote_head(), self.head)

    def test_closed_or_concurrently_updated_pr_is_left_untouched(self):
        for state, sha in (("CLOSED", self.head), ("OPEN", "b" * 40)):
            self.assertIn("left untouched", self.repair(state=state, sha=sha))
            self.assertEqual(self.remote_head(), self.head)

    def test_failed_session_cannot_push_partial_repairs(self):
        self.assertIn("repair incomplete", self.repair(agent=lambda *a, **k: {"status": "timeout"}))
        self.assertEqual(self.remote_head(), self.head)


@unittest.skipIf(os.name == "nt", "Linux systemd shim and flock")
class Shim(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = self.tmp.name
        self.bin = os.path.join(self.home, ".local", "bin")
        os.makedirs(self.bin)
        os.makedirs(os.path.join(self.home, "workspace", "RedEngine-nightly"))
        self.log = os.path.join(self.home, "commands")
        self.env = dict(os.environ, HOME=self.home, SHIM_LOG=self.log)
        for name, text in (("git", '#!/bin/sh\necho "git $*" >> "$SHIM_LOG"\nexit "${FAKE_GIT_CODE:-0}"\n'),
                           ("python3", '#!/bin/sh\necho "python $*" >> "$SHIM_LOG"\n')):
            path = os.path.join(self.bin, name)
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            os.chmod(path, 0o755)
        self.shim = os.path.join(self.bin, "redengine-nightly")
        with open(self.shim, "w", encoding="utf-8") as f:
            f.write(di.SHIM.format(backup="previous"))

    def run_shim(self, *args):
        return subprocess.run(["bash", self.shim, *args], env=self.env, capture_output=True, text=True, timeout=10)

    def test_lock_is_taken_before_any_git_command(self):
        import fcntl
        state = os.path.join(self.home, ".local", "state", "redengine-nightly")
        os.makedirs(state)
        with open(os.path.join(state, "run.lock"), "a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = self.run_shim()
        self.assertEqual(result.returncode, 0)
        self.assertIn("another daily command", result.stdout)
        self.assertFalse(os.path.exists(self.log))

    def test_python_reuses_the_inherited_lock_and_direct_invocations_cannot_take_it(self):
        import fcntl
        state = os.path.join(self.home, "state")
        os.makedirs(state)
        with open(os.path.join(state, "run.lock"), "a") as lock, mock.patch.object(di, "STATE", state):
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with mock.patch.dict(os.environ, {"RED_NIGHTLY_LOCK_FD": str(lock.fileno())}):
                with di.run_lock() as acquired:
                    self.assertTrue(acquired)
            with mock.patch.dict(os.environ, {"RED_NIGHTLY_LOCK_FD": "-1"}):
                with di.run_lock() as acquired:
                    self.assertFalse(acquired)

    def test_dry_run_does_not_create_reports_state_or_a_lock(self):
        with mock.patch.object(di, "cmd_prompt", return_value=0) as prompt, mock.patch.object(di, "run_lock") as lock, mock.patch.object(di, "Day") as day:
            self.assertEqual(di.main(["run", "--dry-run"]), 0)
        prompt.assert_called_once()
        lock.assert_not_called()
        day.assert_not_called()

    def test_fetch_failure_stops_before_reset_or_agent(self):
        self.env["FAKE_GIT_CODE"] = "7"
        self.assertEqual(self.run_shim().returncode, 7)
        with open(self.log, encoding="utf-8") as f:
            lines = f.readlines()
        self.assertEqual(len(lines), 1)
        self.assertIn("fetch", lines[0])

    def test_settle_and_legacy_flags_reach_the_right_subcommand(self):
        for args, expected in (((), "run"), (("--force",), "run --force"), (("settle",), "settle")):
            self.assertEqual(self.run_shim(*args).returncode, 0)
            with open(self.log, encoding="utf-8") as f:
                last = f.readlines()[-1].strip()
            self.assertTrue(last.endswith("daily_improve.py " + expected), last)

    def test_install_adds_catchup_timer_and_recurring_settle(self):
        with mock.patch.object(di, "HOME", self.home), mock.patch.object(di.subprocess, "run") as run:
            di.cmd_install(None)
        units = os.path.join(self.home, ".config", "systemd", "user")
        with open(os.path.join(units, "redengine-nightly-settle.timer"), encoding="utf-8") as f:
            self.assertIn("OnUnitInactiveSec=15m", f.read())
        with open(os.path.join(units, "redengine-nightly.timer.d", "reliability.conf"), encoding="utf-8") as f:
            self.assertIn("Persistent=true", f.read())
        self.assertEqual(run.call_args.args[0][-1], "redengine-nightly-settle.timer")


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
