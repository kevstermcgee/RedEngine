#!/usr/bin/env python3
"""Tests for scripts/idea_forge.py (run by tests/idea_forge.rs, or directly: python3 scripts/test_idea_forge.py).

The model is replaced by a stub `claude` and the engine by a fake `red_engine2`, so the whole loop (idea -> worktree-like checkout -> agent -> notes -> score -> feedback -> ship guard) is observed
without a network, a GPU, Rust or a single token.
"""
import contextlib
import io
import json
import os
import random
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import idea_forge  # noqa: E402

SCRIPT = os.path.join(HERE, "idea_forge.py")
# `nightly` exports the backlog to a note in the real workspace that the owner's maintenance job reads. A test must NEVER write there (an early version did, and replaced a real
# 28-issue note with an empty test backlog): every test in this module exports to a scratch file instead.
SCRATCH_EXPORT = os.path.join(tempfile.mkdtemp(prefix="idea_forge_export_"), "idea-forge-feedback.md")
os.environ["IDEA_FORGE_EXPORT"] = SCRATCH_EXPORT
REAL_NOTE = os.path.join(os.path.dirname(os.path.dirname(HERE)), "idea-forge-feedback.md")
WINDOWS = os.name == "nt"
NEEDS_SH = unittest.skipIf(WINDOWS, "a fake executable here is a shell script")


# The night skips itself below `--min-free-gb` of free disk, so every test of `nightly` would otherwise depend on the disk of the machine that runs it (it failed on any machine with
# under 20 GB free: the night logged "skipped", and the tests then looked for an agent log that was never written). The disk is read in ONE place, `idea_forge.free_gb`, and no test
# sees the real one: `setUpModule` replaces it with plenty, and the one test of the low-disk skip replaces it with little.
PLENTY_GB = 100.0
# To prove the suite is hermetic, run it on a machine that is nearly full (`IDEA_FORGE_TEST_MACHINE_FREE_GB=5 python3 scripts/test_idea_forge.py`; tests/idea_forge.rs does): every
# way of asking Python for the free space then answers with that, and the suite must still pass.
_MACHINE_GB = os.environ.get("IDEA_FORGE_TEST_MACHINE_FREE_GB")
if _MACHINE_GB:
    _real_usage = shutil.disk_usage

    def _full_disk(path):
        u = _real_usage(path)
        free = int(float(_MACHINE_GB) * 2**30)
        return shutil._ntuple_diskusage(max(u.total, 2 * free), max(u.total, 2 * free) - free, free)

    shutil.disk_usage = _full_disk
_disk = mock.patch.object(idea_forge, "free_gb", lambda: PLENTY_GB)


def setUpModule():
    _disk.start()


def tearDownModule():
    _disk.stop()


def home():
    d = tempfile.mkdtemp(prefix="idea_forge_home_")
    os.environ["IDEA_FORGE_HOME"] = d
    return d


def wr(path, text, mode="w"):
    with open(path, mode) as f:
        f.write(text)


def rd(path):
    with open(path) as f:
        return f.read()


def sh(path, text):
    wr(path, text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
    return path


def cli(*args, cwd=None, env=None, check=True):
    e = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")   # CI runners have no git identity
    e.update(env or {})
    p = subprocess.run([sys.executable, SCRIPT, *args], capture_output=True, text=True, cwd=cwd, env=e)
    if check and p.returncode:
        raise AssertionError(f"{args} -> {p.returncode}\n{p.stdout}\n{p.stderr}")
    return p


GOOD = {"id": "F1", "key": "no-random-expression", "area": "format", "title": "no random", "severity": 3, "cost_min": 20, "evidence": "rules have no random expression", "workaround": "a die entity", "proposal": "add random(a,b)"}


def good_feedback(extra=None):
    rows = [GOOD] + (extra or [])
    return ("# run\n\n## The game\n\nA game.\n\n## Findings\n\n```json findings\n" + json.dumps(rows) + "\n```\n\n"
            "## Idea fit\n\nThe mechanic was expressible apart from the random part, which needed a workaround noted above.\n\n## What worked\n\nvalidate and verify were fast and precise.\n")


class Isolation(unittest.TestCase):
    def test_the_tests_never_write_the_real_feedback_note(self):
        self.assertEqual(os.environ["IDEA_FORGE_EXPORT"], SCRATCH_EXPORT)
        self.assertEqual(idea_forge.export_path(), SCRATCH_EXPORT)
        self.assertNotEqual(os.path.abspath(idea_forge.export_path()), os.path.abspath(REAL_NOTE))
        self.assertTrue(os.path.abspath(SCRATCH_EXPORT).startswith(os.path.abspath(tempfile.gettempdir())))


class Ideas(unittest.TestCase):
    def test_the_walk_never_repeats_across_calls_and_machines(self):
        home()
        first = idea_forge.next_codes(20000)
        second = idea_forge.next_codes(20000)
        self.assertEqual(len(set(first + second)), 40000, "an idea was handed out twice")
        other = tempfile.mkdtemp()
        os.environ["IDEA_FORGE_HOME"] = other
        self.assertNotEqual(idea_forge.next_codes(5), first[:5], "two machines must not walk the same sequence")

    def test_an_idea_decodes_to_the_same_text_as_the_game_builder(self):
        forge = idea_forge._forge_module()
        i = idea_forge.make_idea(123456789)
        decoded = forge.decode(123456789)
        for part in (i["you"], i["but"], i["pushback"], i["goal"], i["story"], i["title"].upper()):
            self.assertIn(part, decoded)
        self.assertEqual(i["slug"], "electric-foundry")
        self.assertEqual(idea_forge.make_idea(123456789, story=False)["story"], "")

    def test_the_idea_command_reproduces_a_code(self):
        home()
        a, b = cli("idea", "--code", "42").stdout, cli("idea", "--code", "42").stdout
        self.assertEqual(a, b)
        self.assertIn("0000000042", a)


class Brief(unittest.TestCase):
    def test_the_brief_carries_everything_an_agent_needs(self):
        text = idea_forge.brief(idea_forge.make_idea(123456789), "2026-10-08")
        for must in ("electric-foundry", "0123456789", "examples/2d/electric-foundry.game2d.json", "scripts/idea_forge.py note", "feedback --init", "feedback --check",
                     "Do not `git push`", "Do not change engine code", "YOU plant seeds", "scripts/dev red describe 2d", "idea-fit", "docs/analysis/idea-forge/2026-10-08-2d-electric-foundry.md"):
            self.assertIn(must, text)
        for area in idea_forge.AREAS:
            self.assertIn(area, text)
        self.assertNotIn("Story (optional", idea_forge.brief(idea_forge.make_idea(1, story=False), "2026-10-08"))


class Feedback(unittest.TestCase):
    def test_findings_are_checked(self):
        self.assertEqual(idea_forge.check_findings([GOOD]), [])
        bad = idea_forge.check_findings([dict(GOOD, area="nonsense"), dict(GOOD, id="F2", severity=9, proposal=""), dict(GOOD, id="F2", evidence=" ")])
        text = " | ".join(bad)
        self.assertTrue(any("`key` must be" in b for b in idea_forge.check_findings([dict(GOOD, key="Not Kebab")])))
        self.assertTrue(any("missing `key`" in b for b in idea_forge.check_findings([{k: v for k, v in GOOD.items() if k != "key"}])))
        for fragment in ("area `nonsense`", "severity must be", "say what the engine should change", "`evidence` is empty", "duplicate id"):
            self.assertIn(fragment, text)
        self.assertEqual(idea_forge.check_findings([dict(GOOD, area="worked", proposal="", workaround="")]), [], "a `worked` finding needs no proposal or workaround")
        self.assertTrue(any("placeholder" in b for b in idea_forge.check_findings([dict(GOOD, workaround="see game data")])))

    def test_a_feedback_file_must_be_finished(self):
        self.assertEqual(idea_forge.check_feedback_text(good_feedback()), [])
        self.assertTrue(any("TODO" in b for b in idea_forge.check_feedback_text(good_feedback().replace("A game.", "TODO"))))
        self.assertTrue(any("no ```json findings" in b for b in idea_forge.check_feedback_text("# x\n## Findings\n## Idea fit\n## What worked\n")))
        self.assertTrue(any("no findings" in b for b in idea_forge.check_feedback_text(good_feedback().replace(json.dumps([GOOD]), "[]"))))
        self.assertTrue(any("missing section" in b for b in idea_forge.check_feedback_text(good_feedback().replace("## What worked", "## Other"))))

    def test_a_drafted_file_is_not_shippable_until_the_todos_are_filled(self):
        run = {"idea": idea_forge.make_idea(7), "slug": "x", "date": "2026-10-08", "model": "m", "engine_revision": "abc"}
        draft = idea_forge.draft_feedback(run, [{"area": "format", "text": "no random", "cost_min": 9, "fix": "add random"}], [{"name": "validate passes", "ok": True, "detail": ""}], "2026-10-08")
        self.assertEqual(idea_forge.read_findings(draft)[0]["proposal"], "add random")
        self.assertTrue(idea_forge.check_feedback_text(draft))

    def test_the_digest_ranks_findings_across_runs(self):
        d = tempfile.mkdtemp()
        for name, rows in (("2026-10-08-a", [GOOD, dict(GOOD, id="F2", area="worked", title="verify is fast", severity=1, cost_min=0)]),
                           ("2026-10-09-b", [dict(GOOD, area="discovery", title="search found nothing", severity=2, cost_min=7)])):
            wr(os.path.join(d, name + ".md"), good_feedback().replace(json.dumps([GOOD]), json.dumps(rows)))
        text = idea_forge.digest_text(idea_forge.collect_findings(d))
        self.assertIn("2 run(s) (2 2D, 0 3D), 3 findings", text)
        self.assertLess(text.index("no random"), text.index("search found nothing"), "severity 3 ranks first")
        self.assertIn("## Keep", text)
        self.assertIn("verify is fast", text)


class Guard(unittest.TestCase):
    def test_only_the_game_and_its_feedback_may_change(self):
        self.assertTrue(idea_forge.allowed_path("examples/2d/x.game2d.json", "x"))
        self.assertTrue(idea_forge.allowed_path("docs/analysis/idea-forge/2026-10-08-x.md", "x"))
        for p in ("src/lib.rs", "crates/red2d/src/game.rs", "examples/2d/y.game2d.json", "scripts/dev", "docs/GLOSSARY.md"):
            self.assertFalse(idea_forge.allowed_path(p, "x"), p)


def git(cwd, *a):
    subprocess.run(["git", "-C", cwd, "-c", "user.name=t", "-c", "user.email=t@t", *a], check=True, capture_output=True)


@NEEDS_SH
class EndToEnd(unittest.TestCase):
    """`run` with a stub agent in a throwaway clone, then `ship` (without pushing)."""

    def setUp(self):
        self.home = home()
        base = tempfile.mkdtemp(prefix="idea_forge_e2e_")
        remote = os.path.join(base, "remote.git")
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", remote], check=True)
        self.wt = os.path.join(base, "wt")
        subprocess.run(["git", "clone", "-q", remote, self.wt], check=True, capture_output=True)
        os.makedirs(os.path.join(self.wt, "examples", "2d"))
        os.makedirs(os.path.join(self.wt, "src"))
        wr(os.path.join(self.wt, "src", "lib.rs"), "// engine\n")
        git(self.wt, "checkout", "-q", "-b", "main")
        git(self.wt, "add", "-A")
        git(self.wt, "commit", "-q", "-m", "base")
        git(self.wt, "push", "-q", "-u", "origin", "main")
        git(self.wt, "checkout", "-q", "-b", "idea-test")
        # a fake engine: validate and verify pass
        self.engine = sh(os.path.join(base, "red_engine2"), "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = verify ] && echo '9 passed, 0 failed in 0.1 s'; done\nexit 0\n")
        self.idea = idea_forge.make_idea(123456789)
        slug = self.idea["slug"]
        game = {"game2d": 1, "id": slug, "description": "A tiny game that proves the plumbing end to end.", "persist": ["best"], "sounds": {"a": {}},
                "checks": {"scenarios": [{"name": "a", "smoke": True}, {"name": "b"}, {"name": "c"}]}}
        # a stub `claude`: reads the brief, logs a note through the real CLI, writes the game and the feedback, emits stream-json
        stub = f"""#!{sys.executable}
import json, os, subprocess, sys
run = os.environ["IDEA_FORGE_RUN"]
cli = [sys.executable, {SCRIPT!r}]
subprocess.run(cli + ["note", "no random expression", "--area", "format", "--cost-min", "12", "--fix", "add random(a,b)"], check=True)
open("examples/2d/{slug}.game2d.json", "w").write({json.dumps(game)!r})
open(os.path.join(run, "trace.jsonl"), "w").write(json.dumps({{"argv": ["describe", "2d"], "exit": 0, "t": 1}}) + "\\n" + json.dumps({{"argv": ["verify", "g"], "exit": 0, "t": 61}}) + "\\n")
subprocess.run(cli + ["feedback", "--init"], check=True)
p = os.path.join("docs", "analysis", "idea-forge", "{self.today()}-2d-{slug}.md")
t = open(p).read().replace("TODO: the engine change", "add random(a,b)").replace("TODO: the command and the message, or the number", "rules have no random").replace("TODO: how you got past it (or `none`)", "a die entity")
for todo, text in (("TODO: the design paragraph (the mechanic in one sentence, the core loop, win and lose) and what you cut.", "A game that proves the plumbing end to end for real."),
                   ("TODO: which parts of the card the engine could express, which it could not, and the capability that was missing.", "Everything was expressible except randomness, noted above."),
                   ("TODO: what the engine did well that should be kept (commands, messages, speed).", "validate and verify were fast and precise and named every fix.")):
    t = t.replace(todo, text)
open(p, "w").write(t)
print(json.dumps({{"type": "assistant", "message": {{"content": [{{"type": "tool_use", "name": "Bash", "input": {{"command": "verify"}}}}]}}}}))
print(json.dumps({{"type": "result", "total_cost_usd": 0.5, "duration_ms": 120000}}))
"""
        self.claude = sh(os.path.join(base, "claude"), stub)
        self.env = {"RED_ENGINE_EXE": self.engine}

    def today(self):
        import datetime
        return datetime.date.today().isoformat()

    def run_it(self):
        os.environ["RED_ENGINE_EXE"] = self.engine
        return cli("run", "--code", "123456789", "--workdir", self.wt, "--claude", self.claude, "--quiet", env=self.env)

    def test_a_run_produces_a_scored_game_and_valid_feedback(self):
        p = self.run_it()
        self.assertIn("pass: feedback file", p.stdout)
        self.assertIn("pass: verify passes", p.stdout)
        self.assertIn(SCRIPT, rd(os.path.join(self.wt, ".idea-forge", "brief.md")), "the agent must be told the launching checkout's script by absolute path")
        slug = self.idea["slug"]
        text = rd(os.path.join(self.wt, "docs", "analysis", "idea-forge", f"{self.today()}-2d-{slug}.md"))
        self.assertEqual(idea_forge.check_feedback_text(text), [])
        self.assertIn("Friction report", text)
        self.assertIn("Agent session: 1 assistant turns", text)
        self.assertIn("$0.50", text)
        self.assertIn("exceed the measured wall time", text, "12 claimed minutes in an instant run must be called out")
        runs = json.loads(rd(os.path.join(self.home, "state.json")))["runs"]
        self.assertEqual((runs[-1]["status"], runs[-1]["feedback_ok"], runs[-1]["notes"]), ("built", True, 1))
        self.assertIn(slug, cli("ledger").stdout)

    @NEEDS_SH
    def test_an_agent_that_goes_silent_is_stopped_at_its_timeout_and_the_run_is_still_scored_and_recorded(self):
        hang = sh(os.path.join(os.path.dirname(self.claude), "claude-hang"), f"#!{sys.executable}\nimport time\ntime.sleep(600)\n")
        t0 = time.monotonic()
        p = cli("run", "--code", "123456789", "--workdir", self.wt, "--claude", hang, "--quiet", "--timeout-min", "0.03", env=self.env, check=False)
        self.assertLess(time.monotonic() - t0, 90, "a silent agent in quiet mode used to run for its whole 600 s")
        self.assertIn("agent timeout", p.stderr)
        run = json.loads(rd(os.path.join(self.wt, ".idea-forge", "run.json")))
        self.assertEqual(run["agent_status"], "timeout")
        end = json.loads(rd(os.path.join(self.wt, ".idea-forge", "agent_end.json")))
        self.assertEqual(end["status"], "timeout")
        self.assertAlmostEqual(end["timeout_s"], 1.8)
        self.assertEqual(json.loads(rd(os.path.join(self.home, "state.json")))["runs"][-1]["agent"], "timeout", "the ledger says why the run is incomplete")

    def test_ship_commits_only_the_game_and_the_feedback_and_refuses_engine_edits(self):
        self.run_it()
        slug = self.idea["slug"]
        wr(os.path.join(self.wt, "src", "lib.rs"), "// a patch the agent should not make\n", "a")
        p = cli("ship", slug, "--no-push", check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("src/lib.rs", p.stderr)
        subprocess.run(["git", "-C", self.wt, "checkout", "--", "src/lib.rs"], check=True)
        cli("ship", slug, "--no-push")
        files = subprocess.run(["git", "-C", self.wt, "show", "--name-only", "--format=", "HEAD"], capture_output=True, text=True).stdout.split()
        self.assertEqual(sorted(files), sorted([f"examples/2d/{slug}.game2d.json", f"docs/analysis/idea-forge/{self.today()}-2d-{slug}.md"]))

    def test_ship_refuses_unfinished_feedback(self):
        self.run_it()
        slug = self.idea["slug"]
        p = os.path.join(self.wt, "docs", "analysis", "idea-forge", f"{self.today()}-2d-{slug}.md")
        wr(p, "\nTODO later\n", "a")
        r = cli("ship", slug, "--no-push", check=False)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("feedback is not ready", r.stderr)


class Notes(unittest.TestCase):
    def test_notes_need_a_run_and_a_known_area(self):
        d = tempfile.mkdtemp()
        p = cli("note", "x", "--area", "docs", cwd=d, env={"IDEA_FORGE_RUN": ""}, check=False)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("not inside an Idea Forge run", p.stderr)
        p = cli("note", "x", "--area", "bogus", check=False)
        self.assertNotEqual(p.returncode, 0)


class Kinds(unittest.TestCase):
    def test_a_3d_game_owns_its_folder_and_a_2d_game_one_file(self):
        self.assertTrue(idea_forge.allowed_path("examples/3d/x/x.json", "x", "3d"))
        self.assertTrue(idea_forge.allowed_path("examples/3d/x/audio/dusk.json", "x", "3d"))
        self.assertFalse(idea_forge.allowed_path("examples/3d/y/y.json", "x", "3d"))
        self.assertFalse(idea_forge.allowed_path("examples/2d/x.game2d.json", "x", "3d"))
        self.assertFalse(idea_forge.allowed_path("examples/2d/x/extra.json", "x", "2d"), "a 2D game is one file")
        self.assertEqual(idea_forge.feedback_name("2026-10-09", "3d", "x"), "2026-10-09-3d-x.md")
        self.assertEqual(idea_forge.feedback_name({"date": "2026-10-07", "slug": "x"}), "2026-10-07-x.md", "runs from before kinds keep their name")

    def test_the_3d_brief_asks_for_a_first_person_scene_and_the_2d_brief_for_a_game_file(self):
        i = idea_forge.make_idea(123456789)
        b3, b2 = idea_forge.brief(i, "2026-10-09", kind="3d"), idea_forge.brief(i, "2026-10-09", kind="2d")
        for must in ("examples/3d/electric-foundry/electric-foundry.json", "first-person", "checks.sim", "`lint`", "2026-10-09-3d-electric-foundry.md", "Do not change engine code", "native 3D game"):
            self.assertIn(must, b3)
        self.assertNotIn("game2d", b3)
        for must in ("examples/2d/electric-foundry.game2d.json", "2026-10-09-2d-electric-foundry.md", "native 2D game"):
            self.assertIn(must, b2)
        self.assertNotIn("first-person", b2)

    @NEEDS_SH
    def test_a_3d_game_is_scored_with_lint_and_its_playthroughs(self):
        root = tempfile.mkdtemp()
        os.makedirs(os.path.join(root, "examples", "3d", "x"))
        engine = sh(os.path.join(root, "red_engine2"), "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = verify ] && echo 'm.json: 11 check(s), 0 failed, 2.9s'; done\nexit 0\n")
        os.environ["RED_ENGINE_EXE"] = engine
        self.addCleanup(os.environ.pop, "RED_ENGINE_EXE", None)
        self.assertFalse(idea_forge.score(root, "x", "3d")[0]["ok"], "no game file yet")
        wr(os.path.join(root, "examples", "3d", "x", "x.json"), json.dumps({"rules": [{"id": "r"}], "ui": {"cards": 1}, "audio": {"a": 1}, "checks": {"sim": [{"name": "a"}, {"name": "b"}]}}))
        items = idea_forge.score(root, "x", "3d")
        self.assertEqual([i["ok"] for i in items], [True] * 7, items)
        self.assertEqual(items[1]["name"], "lint passes")
        self.assertIn("11 check(s)", items[2]["detail"])


class Backlog(unittest.TestCase):
    def rows(self, spec):
        """spec: [(run, key, severity, minutes)] -> rows as rows_from would make them."""
        return [{"run": r, "date": r[:10], "id": f"F{i}", "key": k, "area": "format", "title": f"title of {k}", "severity": sv, "cost_min": m, "evidence": "e", "proposal": f"fix {k}"}
                for i, (r, k, sv, m) in enumerate(spec, 1)]

    def test_findings_group_by_key_and_rank_by_severity_then_runs_then_minutes(self):
        groups = idea_forge.build_backlog(self.rows([("2026-10-07-a", "slow-start", 2, 9), ("2026-10-08-b", "slow-start", 2, 5), ("2026-10-08-b", "no-random", 3, 1), ("2026-10-08-c", "tiny", 1, 30)]), [])
        self.assertEqual([g["key"] for g in groups], ["no-random", "slow-start", "tiny"])
        self.assertEqual((groups[1]["minutes"], len(groups[1]["runs"])), (14, 2))

    def test_a_merged_fix_closes_a_key_until_it_is_reported_again(self):
        fixes = [{"key": "no-random", "date": "2026-10-08", "summary": "added random()", "covers": ["2026-10-07-a:F2"]}]
        rows = self.rows([("2026-10-07-a", "other", 2, 1), ("2026-10-08-b", "no-random", 3, 1)])
        rows[0]["id"] = "F2"
        g = {x["key"]: x for x in idea_forge.build_backlog(rows, fixes)}
        self.assertEqual(g["no-random"]["status"], "addressed")
        self.assertEqual(g["other"]["status"], "addressed", "a legacy finding is covered by `covers`")
        again = {x["key"]: x for x in idea_forge.build_backlog(rows + self.rows([("2026-10-09-c", "no-random", 2, 4)]), fixes)}
        self.assertEqual(again["no-random"]["status"], "recurring", "reported after the fix: a regression the improver must see")
        self.assertIn("reported again", idea_forge.backlog_text(list(again.values())))

    def test_a_partial_fix_keeps_the_key_in_the_queue_with_what_remains(self):
        fixes = [{"key": "cold-build", "date": "2026-10-08", "summary": "start now says a build takes minutes", "partial": True, "remaining": "preflight still rebuilds in another profile"}]
        g = idea_forge.build_backlog(self.rows([("2026-10-08-b", "cold-build", 2, 5)]), fixes)[0]
        self.assertEqual(g["status"], "partial")
        text = idea_forge.backlog_text([g])
        self.assertIn("PARTIAL fix", text)
        self.assertIn("preflight still rebuilds", text)
        self.assertIn("`cold-build`", idea_forge.known_issues_section([g]))
        self.assertIn("partly fixed", idea_forge.known_issues_section([g]))
        self.assertEqual(idea_forge.build_backlog(self.rows([("2026-10-08-b", "cold-build", 2, 5)]), [dict(fixes[0], partial=False)])[0]["status"], "addressed")

    def test_an_open_fix_pr_takes_a_key_out_of_the_queue(self):
        groups = idea_forge.build_backlog(self.rows([("2026-10-08-b", "no-random", 3, 1)]), [], {"no-random": {"pr": "https://x/1", "status": "open"}})
        self.assertEqual(groups[0]["status"], "in-progress")
        self.assertEqual(idea_forge.build_backlog(self.rows([("2026-10-08-b", "no-random", 3, 1)]), [], {"no-random": {"status": "closed"}})[0]["status"], "rejected")
        self.assertIn("nothing open", idea_forge.backlog_text(groups, statuses=("open", "recurring")))

    def test_the_brief_lists_known_issues_so_agents_add_evidence_instead_of_rediscovering(self):
        groups = idea_forge.build_backlog(self.rows([("2026-10-08-b", "no-random", 3, 1)]), [])
        text = idea_forge.brief(idea_forge.make_idea(5), "2026-10-09", known=idea_forge.known_issues_section(groups))
        self.assertIn("Known issues", text)
        self.assertIn("`no-random`", text)
        self.assertIn("--key", text)
        self.assertNotIn("Known issues", idea_forge.brief(idea_forge.make_idea(5), "2026-10-09"))

    def test_autofill_makes_any_feedback_valid_and_says_what_it_did(self):
        root = tempfile.mkdtemp()
        run_dir = os.path.join(root, ".idea-forge")
        os.makedirs(run_dir)
        run = {"idea": idea_forge.make_idea(7), "slug": "x", "kind": "2d", "date": "2026-10-08", "model": "m", "engine_revision": "abc", "score": [{"name": "validate passes", "ok": False, "detail": ""}]}
        wr(os.path.join(run_dir, "issues.jsonl"), json.dumps({"area": "format", "text": "no random expression", "cost_min": 3, "fix": "", "key": ""}) + "\n")
        path = idea_forge.autofill_feedback(run_dir, run, root)   # no file at all: drafted from the notes
        text = rd(path)
        self.assertEqual(idea_forge.check_feedback_text(text), [])
        self.assertIn(idea_forge.AUTO_BANNER, text)
        row = idea_forge.read_findings(text)[0]
        self.assertEqual((row["key"], row["proposal"], row["workaround"]), ("no-random-expression", "(none given)", "(none recorded)"))
        wr(path, rd(path).replace("(not completed by the agent: the run ended before this was written)", "TODO: later", 1))
        idea_forge.autofill_feedback(run_dir, run, root)   # a half-written file with a TODO is repaired too, and again is a no-op for the banner
        self.assertEqual(idea_forge.check_feedback_text(rd(path)), [])
        self.assertEqual(rd(path).count(idea_forge.AUTO_BANNER), 1)


class Daily(unittest.TestCase):
    def test_the_order_is_a_shuffle_and_both_orders_happen(self):
        seen = {tuple(idea_forge.pick_order(random.Random(n))) for n in range(40)}
        self.assertEqual(seen, {("2d", "3d"), ("3d", "2d")})

    def test_a_day_is_fixed_once_chosen_and_failed_slots_stop_after_two_attempts(self):
        st = {}
        day = idea_forge.day_state(st, "2026-10-09", ["3d", "2d"])
        self.assertEqual(idea_forge.day_state(st, "2026-10-09", ["2d", "3d"])["order"], ["3d", "2d"], "a day never reshuffles")
        self.assertEqual(idea_forge.pending_kinds(day), ["3d", "2d"])
        day["slots"]["3d"].update(status="done")
        day["slots"]["2d"].update(status="failed", attempts=1)
        self.assertEqual(idea_forge.pending_kinds(day), ["2d"], "a failed slot is retried once")
        day["slots"]["2d"]["attempts"] = idea_forge.MAX_ATTEMPTS
        self.assertEqual(idea_forge.pending_kinds(day), [])
        day["slots"]["2d"].update(status="review", attempts=1)
        self.assertEqual(idea_forge.pending_kinds(day), [], "a built game awaiting review is not redone")

    def test_a_second_daily_run_is_refused_while_the_first_is_alive(self):
        home()
        with idea_forge.Lock():
            with self.assertRaises(SystemExit) as e:
                with idea_forge.Lock():
                    pass
            self.assertIn("another daily run is active", str(e.exception))
        with idea_forge.Lock():
            pass   # released, and a stale pid file does not block either
        wr(os.path.join(os.environ["IDEA_FORGE_HOME"], "daily.lock"), "999999999")
        with idea_forge.Lock():
            pass

    def test_the_dry_run_prints_the_order_and_fixes_it(self):
        home()
        first = cli("daily", "--dry-run", "--date", "2026-10-09").stdout
        second = cli("daily", "--dry-run", "--date", "2026-10-09").stdout
        self.assertEqual(first, second)
        self.assertIn("would run:", first)
        forced = cli("daily", "--dry-run", "--date", "2026-10-10", "--order", "3d,2d", "--next").stdout
        self.assertIn("order 3d, 2d", forced)
        self.assertIn("would run: 3d\n", forced + "\n")
        self.assertNotEqual(cli("daily", "--order", "3d", "--dry-run", check=False).returncode, 0)


class Schedule(unittest.TestCase):
    def test_cron_lines_carry_what_cron_lacks(self):
        home()
        lines = idea_forge.cron_lines(["06:00", "17:05"], 12, "some-model", 20, "nightly", True)
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("0 6 * * * cd "))
        self.assertTrue(lines[1].startswith("5 17 * * * cd "))
        for l in lines:
            for must in ("PATH=", "IDEA_FORGE_HOME=", "PYTHONUNBUFFERED=1", "idea_forge.py", "nightly --budget 12 --improve --improve-budget 20", "--model some-model", "nightly.log", idea_forge.CRON_MARK):
                self.assertIn(must, l)
        self.assertNotIn("--improve", idea_forge.cron_lines(["06:00"], 12)[0], "the improvement agent is opt-in")
        self.assertIn("daily --next --budget 7", idea_forge.cron_lines(["06:00"], 7, None, None, "daily")[0])   # paths are quoted on Windows, so match the arguments, not "idea_forge.py daily"
        for bad in ("25:00", "9", "09:61", "noon"):
            with self.assertRaises(SystemExit):
                idea_forge.cron_lines([bad], 5)

    @NEEDS_SH
    def test_install_is_idempotent_keeps_other_lines_and_uninstall_removes_only_ours(self):
        home()
        d = tempfile.mkdtemp()
        store = os.path.join(d, "crontab.txt")
        wr(store, "0 3 * * * backup.sh\n")
        sh(os.path.join(d, "crontab"), f"#!/bin/sh\nif [ \"$1\" = -l ]; then cat {store}; else cat > {store}; fi\n")
        env = {"PATH": d + os.pathsep + os.environ["PATH"]}
        cli("schedule", "--backend", "cron", "--install", "--times", "10:00,18:00", env=env)
        cli("schedule", "--backend", "cron", "--install", "--times", "10:00,18:00", env=env)
        lines = rd(store).splitlines()
        self.assertEqual(sum(1 for l in lines if idea_forge.CRON_MARK in l), 2, "installing twice must not duplicate")
        self.assertIn("0 3 * * * backup.sh", lines)
        printed = cli("schedule", "--backend", "cron", "--times", "11:00", env=env).stdout
        self.assertIn("not installed", printed)
        self.assertEqual(len(rd(store).splitlines()), 3, "printing changes nothing")
        cli("schedule", "--backend", "cron", "--uninstall", env=env)
        self.assertEqual(rd(store).splitlines(), ["0 3 * * * backup.sh"])


@NEEDS_SH
class E2EBase(unittest.TestCase):
    """In-process fixtures: a stub agent, a bare remote, a stub `gh`; worktrees are plain clones the tool then removes."""

    def setUp(self):
        self.home = home()
        self.base = tempfile.mkdtemp(prefix="idea_forge_daily_")
        self.remote = os.path.join(self.base, "remote.git")
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", self.remote], check=True)
        seed = os.path.join(self.base, "seed")
        subprocess.run(["git", "clone", "-q", self.remote, seed], check=True, capture_output=True)
        os.makedirs(os.path.join(seed, "src"))
        wr(os.path.join(seed, "src", "lib.rs"), "// engine\n")
        git(seed, "checkout", "-q", "-b", "main")
        git(seed, "add", "-A")
        git(seed, "commit", "-q", "-m", "base")
        git(seed, "push", "-q", "-u", "origin", "main")
        self.removed = []
        self.engine = sh(os.path.join(self.base, "red_engine2"), "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = verify ] && echo '9 passed, 0 failed in 0.1 s'; done\nexit 0\n")
        self.gh_dir = os.path.join(self.base, "gh_state")
        os.makedirs(self.gh_dir)
        sh(os.path.join(self.base, "gh"), """#!/bin/sh
if [ "$1 $2" = "pr view" ]; then
  f="$GH_STUB_DIR/$(basename "$3").json"
  if [ -f "$f" ]; then cat "$f"; else echo "{\\"state\\": \\"${IDEA_FORGE_STUB_PR_STATE:-OPEN}\\", \\"mergeable\\": \\"MERGEABLE\\", \\"statusCheckRollup\\": [], \\"files\\": []}"; fi
elif [ "$1 $2" = "pr merge" ]; then
  if [ -f "$GH_STUB_DIR/merge_fails" ]; then echo "merge refused" >&2; exit 1; fi
  echo "$3" >> "$GH_STUB_DIR/merged.log"
else echo https://example.test/pull/$$; fi
""")
        game2 = {"game2d": 1, "description": "A tiny game that proves the plumbing end to end.", "persist": ["b"], "sounds": {"a": {}}, "checks": {"scenarios": [{"smoke": True}, {}, {}]}}
        game3 = {"rules": [{"id": "r"}], "ui": {"c": 1}, "audio": {"a": 1}, "checks": {"sim": [{"name": "a"}, {"name": "b"}]}}
        stub = f"""#!{sys.executable}
import json, os, subprocess, sys
if os.environ.get("IDEA_FORGE_STUB_FAIL") and "fix" not in os.environ.get("IDEA_FORGE_STUB_FAIL"):
    sys.exit(3)
run = os.environ["IDEA_FORGE_RUN"]
r = json.load(open(os.path.join(run, "run.json")))
slug, kind, date = r["slug"], r["kind"], r["date"]
cli = [sys.executable, {SCRIPT!r}]
import time
log = os.environ.get("IDEA_FORGE_STUB_LOG")
if log:
    with open(log, "a") as f:
        f.write("start %s %f\\n" % (kind, time.time()))
if kind == "fix":
    mode = os.environ.get("IDEA_FORGE_STUB_FIX", "good")
    if mode != "nothing":
        key = r["candidates"][0]
        with open("src/lib.rs", "a") as f:
            f.write("// fix for %s\\n" % key)
        if mode != "notest":
            os.makedirs("tests", exist_ok=True)
            with open("tests/fix_%s.rs" % key.replace("-", "_"), "w") as f:
                f.write("#[test] fn t() {{}}\\n")
        if mode == "forbidden":
            os.makedirs(".github", exist_ok=True)
            with open(".github/x.yml", "w") as f:
                f.write("x")
        if mode != "nofix":
            os.makedirs("docs/analysis/idea-forge", exist_ok=True)
            with open("docs/analysis/idea-forge/fixes.json", "w") as f:
                f.write(json.dumps([{{"key": key, "covers": [], "date": date, "summary": "fixed " + key}}]))
    if log:
        with open(log, "a") as f:
            f.write("end fix %f\\n" % time.time())
    print(json.dumps({{"type": "result", "result": "fixed", "total_cost_usd": 0.4, "duration_ms": 1000}}))
    sys.exit(0)
subprocess.run(cli + ["note", "no random expression", "--area", "format", "--cost-min", "1", "--fix", "add random(a,b)"], check=True)
if kind == "2d":
    path = "examples/2d/%s.game2d.json" % slug
    game = {json.dumps(game2)!r}
else:
    os.makedirs("examples/3d/%s" % slug, exist_ok=True)
    path = "examples/3d/%s/%s.json" % (slug, slug)
    game = {json.dumps(game3)!r}
os.makedirs(os.path.dirname(path), exist_ok=True)
with open(path, "w") as f:
    f.write(game)
subprocess.run(cli + ["feedback", "--init"], check=True)
p = os.path.join("docs", "analysis", "idea-forge", "%s-%s-%s.md" % (date, kind, slug))
t = open(p).read().replace("TODO: the engine change", "add random(a,b)").replace("TODO: the command and the message, or the number", "rules have no random").replace("TODO: how you got past it (or `none`)", "a die entity")
for todo, text in (("TODO: the design paragraph (the mechanic in one sentence, the core loop, win and lose) and what you cut.", "A game that proves the plumbing end to end for real."),
                   ("TODO: which parts of the card the engine could express, which it could not, and the capability that was missing.", "Everything was expressible except randomness, noted above."),
                   ("TODO: what the engine did well that should be kept (commands, messages, speed).", "validate and verify were fast and precise and named every fix.")):
    t = t.replace(todo, text)
open(p, "w").write(t)
if log:
    with open(log, "a") as f:
        f.write("end %s %f\\n" % (kind, time.time()))
print(json.dumps({{"type": "result", "total_cost_usd": 0.25, "duration_ms": 60000}}))
"""
        self.claude = sh(os.path.join(self.base, "claude"), stub)
        env = {"RED_ENGINE_EXE": self.engine, "PATH": self.base + os.pathsep + os.environ["PATH"], "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
        env["IDEA_FORGE_GATES"] = '[["true"]]'
        env["GH_STUB_DIR"] = self.gh_dir
        env["IDEA_FORGE_AUTO_MERGE"] = "1"
        self.log = os.path.join(self.base, "stub.log")
        env["IDEA_FORGE_STUB_LOG"] = self.log
        self.saved = {k: os.environ.get(k) for k in list(env) + ["IDEA_FORGE_STUB_FAIL", "IDEA_FORGE_STUB_FIX"]}
        os.environ.update(env)
        self.addCleanup(self.restore)

        def fake_worktree(slug, kind="2d"):
            wt = os.path.join(self.base, f"wt-{kind}-{slug}")
            subprocess.run(["git", "clone", "-q", self.remote, wt], check=True, capture_output=True)
            git(wt, "checkout", "-q", "-b", f"idea-{kind}-{slug}")
            return wt, f"idea-{kind}-{slug}"

        def fake_fix_worktree(name):
            wt = os.path.join(self.base, f"wt-{name}")
            subprocess.run(["git", "clone", "-q", self.remote, wt], check=True, capture_output=True)
            git(wt, "checkout", "-q", "-b", name)
            return wt, name

        self.fixes_on_main = {}
        patches = [mock.patch.object(idea_forge, "refresh_checkout", lambda root=None: "not updated: test"),mock.patch.object(idea_forge, "make_worktree", fake_worktree), mock.patch.object(idea_forge, "make_fix_worktree", fake_fix_worktree),
                   mock.patch.object(idea_forge, "remove_worktree", lambda wt: self.removed.append(wt)),
                   mock.patch.object(idea_forge, "repo_texts", lambda ref="origin/main": dict(self.fixes_on_main))]
        for p_ in patches:
            p_.start()
            self.addCleanup(p_.stop)

    def restore(self):
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v

    def daily(self, *extra):
        try:
            idea_forge.main(["daily", "--claude", self.claude, "--quiet", "--date", "2026-10-09", *extra])
            return 0
        except SystemExit as e:
            return e.code or 0

    def day(self):
        return json.loads(rd(os.path.join(self.home, "state.json")))["days"]["2026-10-09"]

    def state(self):
        return idea_forge.load_state()

    def seed_feedback(self, rows, name="2026-10-08-2d-seed"):
        """A run's feedback in the local store, as `ship` leaves it: findings with keys."""
        findings = [dict({"id": f"F{i}", "area": "format", "title": f"title {k}", "severity": sv, "cost_min": 3, "evidence": "e", "workaround": "w", "proposal": f"fix {k}", "key": k}) for i, (k, sv) in enumerate(rows, 1)]
        wr(os.path.join(idea_forge.store_dir(), name + ".md"), good_feedback().replace(json.dumps([GOOD]), json.dumps(findings)))

    def improve(self, *extra):
        try:
            idea_forge.main(["improve", "--claude", self.claude, "--quiet", *extra])
            return 0
        except SystemExit as e:
            return e.code or 0


@NEEDS_SH
class DailyEndToEnd(E2EBase):
    """A whole day in-process, one game at a time."""

    def test_a_day_makes_one_game_of_each_kind_in_the_chosen_order_and_ships_both(self):
        self.assertEqual(self.daily("--order", "3d,2d"), 0)
        day = self.day()
        self.assertEqual(day["order"], ["3d", "2d"])
        self.assertTrue(all(day["slots"][k]["status"] == "done" and day["slots"][k]["detail"].startswith("https://example.test/pull/") for k in ("2d", "3d")), day)
        runs = json.loads(rd(os.path.join(self.home, "state.json")))["runs"]
        self.assertEqual([r["kind"] for r in runs], ["3d", "2d"], "in the day's order")
        self.assertEqual({r["status"] for r in runs}, {"shipped"})
        self.assertEqual(len(self.removed), 2, "each shipped worktree is removed")
        for r in runs:
            self.assertTrue(os.path.isfile(os.path.join(self.home, "runs", f"{r['date']}-{r['kind']}-{r['slug']}", "issues.jsonl")), "the notes are archived before the worktree goes")
        # what each PR contains: the 3D game's whole folder, or the one 2D file, plus the feedback
        for r in runs:
            files = subprocess.run(["git", "-C", os.path.join(self.base, f"wt-{r['kind']}-{r['slug']}"), "show", "--name-only", "--format=", "HEAD"], capture_output=True, text=True).stdout.split()
            game = f"examples/3d/{r['slug']}/{r['slug']}.json" if r["kind"] == "3d" else f"examples/2d/{r['slug']}.game2d.json"
            self.assertEqual(sorted(files), sorted([game, f"docs/analysis/idea-forge/{r['date']}-{r['kind']}-{r['slug']}.md"]))

    def test_a_finished_day_is_a_no_op_and_next_runs_one_game(self):
        self.assertEqual(self.daily("--order", "2d,3d", "--next"), 0)
        self.assertEqual([k for k, v in self.day()["slots"].items() if v["status"] == "done"], ["2d"])
        self.assertEqual(self.daily("--next"), 0)
        self.assertEqual(sorted(k for k, v in self.day()["slots"].items() if v["status"] == "done"), ["2d", "3d"])
        before = len(json.loads(rd(os.path.join(self.home, "state.json")))["runs"])
        self.assertEqual(self.daily(), 0)
        self.assertEqual(len(json.loads(rd(os.path.join(self.home, "state.json")))["runs"]), before, "nothing left to do")

    def test_an_agent_that_makes_nothing_is_retried_once_and_its_feedback_still_reaches_the_repo(self):
        os.environ["IDEA_FORGE_STUB_FAIL"] = "1"
        self.assertEqual(self.daily("--order", "2d,3d", "--next"), 1)
        slot = self.day()["slots"]["2d"]
        self.assertEqual((slot["status"], slot["attempts"]), ("failed", 1))
        self.assertIn("feedback shipped: https://example.test/pull/", slot["detail"])
        self.assertIn("feedback auto-completed", slot["detail"])
        self.assertEqual(self.daily("--next"), 1)
        self.assertEqual(self.day()["slots"]["2d"]["attempts"], 2)
        self.assertEqual(self.daily("--next"), 1, "the next slot (3d) is tried; 2d is not retried a third time")
        self.assertEqual(self.day()["slots"]["3d"]["attempts"], 1)
        self.assertEqual(self.day()["slots"]["2d"]["attempts"], 2)
        runs = json.loads(rd(os.path.join(self.home, "state.json")))["runs"]
        self.assertEqual(len(runs), 3)
        for r in runs:
            wt = os.path.join(self.base, f"wt-{r['kind']}-{r['slug']}")
            files = subprocess.run(["git", "-C", wt, "show", "--name-only", "--format=", "HEAD"], capture_output=True, text=True).stdout.split()
            self.assertEqual(files, [f"docs/analysis/idea-forge/{r['date']}-{r['kind']}-{r['slug']}.md"], "a run that built nothing ships only its feedback")
            text = rd(os.path.join(wt, "docs", "analysis", "idea-forge", f"{r['date']}-{r['kind']}-{r['slug']}.md"))
            self.assertEqual(idea_forge.check_feedback_text(text), [], "the completed file is valid")
            self.assertIn(idea_forge.AUTO_BANNER, text)
            self.assertIn("run-ended-without-findings", text)
            self.assertTrue(os.path.isfile(os.path.join(self.home, "feedback", f"{r['date']}-{r['kind']}-{r['slug']}.md")), "kept locally for the backlog")
        self.assertEqual(len(self.removed), 3, "shipped worktrees are removed")

    def test_no_ship_builds_and_keeps_the_worktree(self):
        self.assertEqual(self.daily("--order", "2d,3d", "--next", "--no-ship"), 0)
        self.assertEqual(self.removed, [])
        self.assertIn("not shipped", self.day()["slots"]["2d"]["detail"])


@NEEDS_SH
class ImproveEndToEnd(E2EBase):
    """The nightly engine-improvement agent: it fixes ONE backlog key, and nothing reaches a PR unless the CLI's own checks say it is sound."""

    def test_a_tested_and_recorded_fix_becomes_a_pr_and_takes_its_key_out_of_the_queue(self):
        self.seed_feedback([("no-random-expression", 3), ("slow-start", 1)])
        self.assertEqual(self.improve(), 0)
        imp = self.state()["improvements"]
        self.assertEqual(list(imp), ["no-random-expression"], "the worst-ranked key, and only that one")
        self.assertTrue(imp["no-random-expression"]["pr"].startswith("https://example.test/pull/"))
        wt = [d for d in os.listdir(self.base) if d.startswith("wt-engine-fix-")][0]
        files = subprocess.run(["git", "-C", os.path.join(self.base, wt), "show", "--name-only", "--format=", "HEAD"], capture_output=True, text=True).stdout.split()
        self.assertEqual(sorted(files), ["docs/analysis/idea-forge/fixes.json", "src/lib.rs", "tests/fix_no_random_expression.rs"], "no run directory leaks into the PR")
        self.assertEqual(len(self.removed), 1)
        keys = {g["key"]: g["status"] for g in idea_forge.current_backlog()}
        self.assertEqual(keys, {"no-random-expression": "in-progress", "slow-start": "open"})
        self.assertEqual(self.improve(), 0)
        self.assertEqual(sorted(self.state()["improvements"]), ["no-random-expression", "slow-start"], "tomorrow takes the next key, not the one whose PR is open")

    def test_a_closed_fix_pr_is_not_retried_and_a_merged_one_is_watched(self):
        self.seed_feedback([("a-problem", 3)])
        self.assertEqual(self.improve(), 0)
        os.environ["IDEA_FORGE_STUB_PR_STATE"] = "CLOSED"
        self.addCleanup(os.environ.pop, "IDEA_FORGE_STUB_PR_STATE", None)
        idea_forge.refresh_improvements()
        self.assertEqual(self.state()["improvements"]["a-problem"]["status"], "closed")
        self.assertEqual([g["status"] for g in idea_forge.current_backlog()], ["rejected"])
        self.assertEqual(self.improve(), 0, "nothing open: it does not retry a rejected fix")
        self.assertEqual(len(self.state()["improvements"]), 1)

    def test_a_key_reported_again_after_its_fix_is_a_regression_the_improver_sees(self):
        self.fixes_on_main["fixes.json"] = json.dumps([{"key": "no-random-expression", "date": "2026-10-07", "summary": "added random()"}])
        self.seed_feedback([("no-random-expression", 2)], "2026-10-08-2d-later")
        g = idea_forge.current_backlog()[0]
        self.assertEqual((g["key"], g["status"]), ("no-random-expression", "recurring"))
        self.assertIn("added random()", idea_forge.improve_brief([g], "2026-10-09", "cli"))

    def test_every_unsound_change_is_refused_before_any_pr_and_keeps_the_worktree(self):
        self.seed_feedback([("a-problem", 3)])
        for mode, expect in (("forbidden", "off limits"), ("notest", "no regression test"), ("nofix", "no new entry")):
            os.environ["IDEA_FORGE_STUB_FIX"] = mode
            self.assertEqual(self.improve(), 1, mode)
            self.assertEqual(self.state().get("improvements", {}), {}, f"{mode}: no PR may be recorded")
            self.assertEqual(self.removed, [], f"{mode}: the worktree is kept for a human")
            shutil.rmtree(os.path.join(self.base, [d for d in os.listdir(self.base) if d.startswith("wt-engine-fix-")][0]))
        os.environ["IDEA_FORGE_STUB_FIX"] = "good"
        os.environ["IDEA_FORGE_GATES"] = '[["false"]]'
        self.assertEqual(self.improve(), 1, "the CLI's own gates failing stops it, whatever the agent claims")
        self.assertEqual(self.state().get("improvements", {}), {})

    def test_an_agent_that_finds_nothing_safe_to_fix_changes_nothing_and_is_not_an_error(self):
        self.seed_feedback([("a-problem", 3)])
        os.environ["IDEA_FORGE_STUB_FIX"] = "nothing"
        self.assertEqual(self.improve(), 0)
        self.assertEqual(self.state().get("improvements", {}), {})

    def test_an_empty_backlog_does_not_even_make_a_worktree(self):
        self.assertEqual(self.improve(), 0)
        self.assertEqual([d for d in os.listdir(self.base) if d.startswith("wt-")], [])

    def test_no_ship_commits_on_the_branch_and_opens_nothing(self):
        self.seed_feedback([("a-problem", 3)])
        self.assertEqual(self.improve("--no-ship"), 0)
        self.assertEqual(self.state().get("improvements", {}), {})
        self.assertEqual(self.removed, [])


@NEEDS_SH
class NightlyEndToEnd(E2EBase):
    """One night: both games, then one engine improvement, strictly one agent at a time."""

    def nightly(self, *extra):
        try:
            idea_forge.main(["nightly", "--claude", self.claude, "--quiet", "--date", "2026-10-09", *extra])
            return 0
        except SystemExit as e:
            return e.code or 0

    def test_the_night_runs_games_then_an_improvement_one_at_a_time_and_feeds_the_backlog(self):
        self.assertEqual(self.nightly("--improve"), 0)
        events = [l.split() for l in rd(self.log).splitlines()]
        self.assertEqual([(e[0], e[1]) for e in events], [("start", self.day()["order"][0]), ("end", self.day()["order"][0]), ("start", self.day()["order"][1]),
                                                           ("end", self.day()["order"][1]), ("start", "fix"), ("end", "fix")], "agents strictly one after another, the fix last")
        times = [float(e[-1]) for e in events]
        self.assertEqual(times, sorted(times))
        night = self.state()["nights"]["2026-10-09"]
        self.assertEqual((night["games_failed"], night["improve"]), (0, "shipped"))
        # the games' own findings were the backlog the improver worked from
        self.assertEqual(list(self.state()["improvements"]), ["no-random-expression"])

    def test_a_failed_game_still_feeds_the_improver_and_the_night_goes_on(self):
        os.environ["IDEA_FORGE_STUB_FAIL"] = "games"   # games fail; the fix stub is unaffected by this value
        self.assertEqual(self.nightly("--order", "2d,3d"), 0)
        night = self.state()["nights"]["2026-10-09"]
        self.assertEqual(night["games_failed"], 2)
        self.assertEqual(night["improve"], "shipped" if self.state().get("improvements") else night["improve"])
        self.assertIn("run-ended-without-findings", " ".join(g["key"] for g in idea_forge.current_backlog()), "the auto-completed feedback of a run that built nothing is in the backlog")

    def test_the_dry_run_changes_nothing_but_fixes_the_order(self):
        self.assertEqual(self.nightly("--dry-run"), 0)
        self.assertFalse(os.path.exists(self.log))
        self.assertIn("2026-10-09", self.state()["days"])


def check(name, conclusion="SUCCESS", status="COMPLETED"):
    return {"name": name, "status": status, "conclusion": conclusion}


GREEN = [check("full build (ubuntu-24.04)"), check("rustfmt"), check("container image", "SKIPPED")]


@NEEDS_SH
class Settle(E2EBase):
    """The merge policy: the tool merges ONLY its own PRs, ONLY after every check has passed, and re-checks what the PR changes from GitHub's own file list."""

    def setUp(self):
        super().setUp()
        patcher = mock.patch.object(idea_forge, "SETTLE_POLL_SEC", 0)
        patcher.start()
        self.addCleanup(patcher.stop)

    def url(self, n):
        return f"https://example.test/pull/{n}"

    def gh_pr(self, n, state="OPEN", mergeable="MERGEABLE", checks=GREEN, files=()):
        wr(os.path.join(self.gh_dir, f"{n}.json"), json.dumps({"state": state, "mergeable": mergeable, "statusCheckRollup": list(checks), "files": [{"path": f} for f in files]}))

    def track_game(self, n, slug="x", kind="2d", feedback_only=False, **extra):
        st = self.state()
        st["runs"].append(dict({"code": n, "slug": slug, "kind": kind, "date": "2026-10-09", "status": "shipped", "pr": self.url(n), "feedback_only": feedback_only}, **extra))
        wr(os.path.join(self.home, "state.json"), json.dumps(st))

    def track_fix(self, n, key="a-problem"):
        st = self.state()
        st.setdefault("improvements", {})[key] = {"pr": self.url(n), "status": "open", "branch": "b", "date": "2026-10-09"}
        wr(os.path.join(self.home, "state.json"), json.dumps(st))

    def merged(self):
        p = os.path.join(self.gh_dir, "merged.log")
        return rd(p).split() if os.path.exists(p) else []

    def game_files(self, slug="x", kind="2d"):
        game = f"examples/2d/{slug}.game2d.json" if kind == "2d" else f"examples/3d/{slug}/{slug}.json"
        return [game, f"docs/analysis/idea-forge/2026-10-09-{kind}-{slug}.md"]

    def test_a_pr_whose_checks_all_passed_is_merged_and_recorded(self):
        self.track_game(1)
        self.gh_pr(1, files=self.game_files())
        res = idea_forge.settle(0)
        self.assertEqual([r[1] for r in res], ["merged"])
        self.assertEqual(self.merged(), [self.url(1)])
        self.assertEqual(self.state()["runs"][0]["merge"], "merged")
        self.assertEqual(idea_forge.settle(0), [], "a merged PR is not looked at again")

    def test_a_3d_game_may_change_its_whole_folder_and_a_feedback_only_pr_only_the_feedback(self):
        self.track_game(2, "y", "3d")
        self.gh_pr(2, files=["examples/3d/y/y.json", "examples/3d/y/audio/score.json", "docs/analysis/idea-forge/2026-10-09-3d-y.md"])
        self.track_game(3, "z", "2d", feedback_only=True)
        self.gh_pr(3, files=self.game_files("z"))
        verdicts = {r[0]["url"]: r[1] for r in idea_forge.settle(0)}
        self.assertEqual(verdicts, {self.url(2): "merged", self.url(3): "refused"}, "a feedback-only PR that carries a game is not merged")

    def test_a_failed_pending_or_missing_check_is_never_merged(self):
        for n, checks, want in ((10, GREEN + [check("full build (windows-latest)", "FAILURE")], "ci-failed"), (11, GREEN + [check("windows", None, "IN_PROGRESS")], "pending"),
                                (12, [], "pending"), (13, [check("only skipped", "SKIPPED")], "pending"), (14, GREEN + [check("x", "CANCELLED")], "ci-failed")):
            self.track_game(n)
            self.gh_pr(n, checks=checks, files=self.game_files())
            self.assertEqual(idea_forge.settle_one({"url": self.url(n), "type": "game", "slug": "x", "kind": "2d", "feedback_only": False})[0], want, n)
        self.assertEqual(self.merged(), [])
        recorded = {r["code"]: r.get("merge") for r in self.state()["runs"]}
        self.assertEqual((recorded[10], recorded[14], recorded[11]), ("ci-failed", "ci-failed", None))

    def test_a_status_context_is_judged_like_a_check_run(self):
        self.assertEqual(idea_forge.checks_verdict([{"state": "SUCCESS"}]), "passed")
        self.assertEqual(idea_forge.checks_verdict([{"state": "PENDING"}]), "pending")
        self.assertEqual(idea_forge.checks_verdict([{"state": "FAILURE"}, {"state": "SUCCESS"}]), "failed")

    def test_a_conflict_or_an_unknown_mergeability_is_not_merged(self):
        self.track_game(20)
        self.gh_pr(20, mergeable="CONFLICTING", files=self.game_files())
        self.track_game(21)
        self.gh_pr(21, mergeable="UNKNOWN", files=self.game_files())
        verdicts = {r[0]["url"]: r[1] for r in idea_forge.settle(0)}
        self.assertEqual(verdicts, {self.url(20): "conflict", self.url(21): "pending"})
        self.assertEqual(self.merged(), [])

    def test_a_pr_that_changes_anything_off_limits_is_refused_whatever_its_checks_say(self):
        self.track_game(30)
        self.gh_pr(30, files=self.game_files() + ["src/lib.rs"])
        self.track_fix(31, "k1")
        self.gh_pr(31, files=["src/lib.rs", "tests/t.rs", ".github/workflows/ci.yml"])
        self.track_fix(32, "k2")
        self.gh_pr(32, files=["Cargo.toml", "src/lib.rs"])
        verdicts = {r[0]["url"]: r[1] for r in idea_forge.settle(0)}
        self.assertEqual(set(verdicts.values()), {"refused"})
        self.assertEqual(self.merged(), [])

    def test_a_tested_engine_fix_pr_is_merged_and_its_key_becomes_merged(self):
        self.track_fix(40)
        self.gh_pr(40, files=["src/lib.rs", "tests/fix_a_problem.rs", "docs/analysis/idea-forge/fixes.json", "scripts/launchpad.py"])
        self.assertEqual([r[1] for r in idea_forge.settle(0)], ["merged"])
        self.assertEqual(self.state()["improvements"]["a-problem"]["status"], "merged")

    def test_settle_waits_for_running_checks_then_merges(self):
        self.track_game(50)
        self.gh_pr(50, checks=GREEN + [check("windows", None, "IN_PROGRESS")], files=self.game_files())
        calls = []

        def finish(_secs):
            calls.append(1)
            self.gh_pr(50, files=self.game_files())

        with mock.patch.object(idea_forge.time, "sleep", finish):
            res = idea_forge.settle(5)
        self.assertEqual([r[1] for r in res], ["merged"])
        self.assertEqual(len(calls), 1)
        self.assertEqual(idea_forge.settle(0), [])

    def test_a_final_verdict_is_reported_once_even_while_another_pr_is_still_pending(self):
        """Seen live: while one PR's checks ran, every poll re-printed another PR's `ci-failed` (14 identical lines in a night's log)."""
        self.track_game(52)
        self.gh_pr(52, checks=GREEN + [check("windows", "FAILURE")], files=self.game_files())
        self.track_game(53, "y")
        self.gh_pr(53, checks=[check("windows", None, "IN_PROGRESS")], files=self.game_files("y"))
        polls = []

        def poll(_secs):
            polls.append(1)
            if len(polls) == 3:
                self.gh_pr(53, files=self.game_files("y"))

        with mock.patch.object(idea_forge.time, "sleep", poll), mock.patch("builtins.print") as printed:
            res = idea_forge.settle(5)
        lines = [c.args[0] for c in printed.call_args_list if c.args and str(c.args[0]).startswith("settle:")]
        self.assertEqual(sum(1 for l in lines if "ci-failed" in l), 1, lines)
        self.assertEqual(sorted(r[1] for r in res), ["ci-failed", "merged"])
        self.assertEqual(len(polls), 3, "it kept asking about the pending PR only until it finished")
        self.assertEqual(self.merged(), [self.url(53)])

    def test_settle_gives_up_waiting_at_the_deadline_and_keeps_the_pr_tracked(self):
        self.track_game(51)
        self.gh_pr(51, checks=[check("windows", None, "IN_PROGRESS")], files=self.game_files())
        with mock.patch.object(idea_forge.time, "sleep", lambda _s: None):
            res = idea_forge.settle(0)
        self.assertEqual([r[1] for r in res], ["pending"])
        self.assertEqual([pr["url"] for pr in idea_forge.tracked_prs()], [self.url(51)], "still tracked: the next run settles it")

    def test_a_closed_or_externally_merged_pr_is_recorded_not_merged_again(self):
        self.track_game(60)
        self.gh_pr(60, state="MERGED", files=self.game_files())
        self.track_fix(61)
        self.gh_pr(61, state="CLOSED")
        verdicts = {r[0]["url"]: r[1] for r in idea_forge.settle(0)}
        self.assertEqual(verdicts, {self.url(60): "merged", self.url(61): "closed"})
        self.assertEqual(self.merged(), [], "gh pr merge is not called for a PR that is not open")
        self.assertEqual(self.state()["improvements"]["a-problem"]["status"], "closed")

    def test_only_the_tools_own_prs_are_ever_touched(self):
        self.gh_pr(70, files=self.game_files())   # an open green PR nobody tracked
        self.assertEqual(idea_forge.settle(0), [])
        self.assertEqual(self.merged(), [])

    def test_the_off_switches_hold_every_pr(self):
        self.track_game(80)
        self.gh_pr(80, files=self.game_files())
        self.assertEqual(json.loads(cli("config", "--auto-merge", "off").stdout), {"auto_merge": False})
        self.assertEqual([r[1] for r in idea_forge.settle(0)], ["held"])
        self.assertEqual(self.merged(), [])
        self.assertEqual(json.loads(cli("config").stdout), {"auto_merge": False}, "persisted")
        self.assertEqual(json.loads(cli("config", "--auto-merge", "on").stdout), {"auto_merge": True})
        os.environ["IDEA_FORGE_AUTO_MERGE"] = "0"
        self.assertEqual([r[1] for r in idea_forge.settle(0)], ["held"], "the environment switch wins")
        os.environ["IDEA_FORGE_AUTO_MERGE"] = "1"
        self.track_game(81, hold=True)
        self.gh_pr(81, files=self.game_files())
        self.assertEqual([r[0]["url"] for r in idea_forge.settle(0)], [self.url(80)], "a held PR is not even tracked")
        self.assertEqual(self.merged(), [self.url(80)])

    def test_a_refused_merge_is_recorded_and_not_reported_as_merged(self):
        self.track_game(90)
        self.gh_pr(90, files=self.game_files())
        wr(os.path.join(self.gh_dir, "merge_fails"), "1")
        self.assertEqual([r[1] for r in idea_forge.settle(0)], ["merge-error"])
        self.assertEqual(self.state()["runs"][0]["merge"], "merge-error")

    def test_the_night_settles_before_and_after_and_no_merge_turns_it_off_for_the_run(self):
        calls = []
        with mock.patch.object(idea_forge, "settle", lambda wait=0: calls.append(wait) or []):
            try:
                idea_forge.main(["nightly", "--claude", self.claude, "--quiet", "--date", "2026-10-09", "--skip-games", "--settle-timeout-min", "7"])
            except SystemExit:
                pass
        self.assertEqual(calls, [0, 7], "yesterday's finished PRs first, then tonight's after waiting")
        os.environ.pop("IDEA_FORGE_AUTO_MERGE", None)
        with mock.patch.object(idea_forge, "settle", lambda wait=0: []):
            try:
                idea_forge.main(["nightly", "--claude", self.claude, "--quiet", "--date", "2026-10-09", "--skip-games", "--no-merge"])
            except SystemExit:
                pass
        self.assertEqual(os.environ.get("IDEA_FORGE_AUTO_MERGE"), "0")
        os.environ["IDEA_FORGE_AUTO_MERGE"] = "1"


@NEEDS_SH
class Refresh(unittest.TestCase):
    """The night starts from yesterday's main: a dedicated checkout fast-forwards, anything else is left alone."""

    def setUp(self):
        self.base = tempfile.mkdtemp(prefix="idea_forge_refresh_")
        self.remote = os.path.join(self.base, "remote.git")
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", self.remote], check=True)
        self.work = os.path.join(self.base, "work")
        subprocess.run(["git", "clone", "-q", self.remote, self.work], check=True, capture_output=True)
        git(self.work, "checkout", "-q", "-b", "main")
        wr(os.path.join(self.work, "f.txt"), "1\n")
        git(self.work, "add", "-A")
        git(self.work, "commit", "-q", "-m", "one")
        git(self.work, "push", "-q", "-u", "origin", "main")
        self.dedicated = os.path.join(self.base, "factory")
        subprocess.run(["git", "clone", "-q", self.remote, self.dedicated], check=True, capture_output=True)
        wr(os.path.join(self.work, "f.txt"), "2\n")
        git(self.work, "commit", "-qam", "two")
        git(self.work, "push", "-q")

    def test_a_clean_checkout_tracking_main_is_fast_forwarded(self):
        self.assertEqual(idea_forge.refresh_checkout(self.dedicated), "updated to origin/main")
        self.assertEqual(rd(os.path.join(self.dedicated, "f.txt")), "2\n")

    def test_a_dirty_checkout_a_detached_head_and_a_branch_off_main_are_left_alone(self):
        wr(os.path.join(self.dedicated, "f.txt"), "mine\n")
        self.assertIn("uncommitted", idea_forge.refresh_checkout(self.dedicated))
        self.assertEqual(rd(os.path.join(self.dedicated, "f.txt")), "mine\n")
        git(self.dedicated, "checkout", "-q", "--", "f.txt")
        git(self.dedicated, "checkout", "-q", "-b", "feature")
        self.assertIn("does not track origin/main", idea_forge.refresh_checkout(self.dedicated))
        git(self.dedicated, "checkout", "-q", "--detach")
        self.assertIn("detached", idea_forge.refresh_checkout(self.dedicated))
        self.assertEqual(rd(os.path.join(self.dedicated, "f.txt")), "1\n", "nothing was touched")

    def test_a_diverged_checkout_is_not_rewritten(self):
        wr(os.path.join(self.dedicated, "g.txt"), "local\n")
        git(self.dedicated, "add", "-A")
        git(self.dedicated, "commit", "-q", "-m", "local only")
        self.assertIn("cannot fast-forward", idea_forge.refresh_checkout(self.dedicated))
        self.assertTrue(os.path.isfile(os.path.join(self.dedicated, "g.txt")))


@NEEDS_SH
class Integration(E2EBase):
    """The owner's existing nightly maintenance job reads `*feedback*.md` notes in the workspace; Idea Forge writes its backlog there, and only when it changed."""

    def setUp(self):
        super().setUp()
        self.export = os.path.join(self.base, "idea-forge-feedback.md")
        os.environ["IDEA_FORGE_EXPORT"] = self.export
        self.addCleanup(os.environ.__setitem__, "IDEA_FORGE_EXPORT", SCRATCH_EXPORT)   # back to the module's scratch path, never unset (unset = the real note)

    def test_the_backlog_is_exported_for_the_nightly_job_and_untouched_when_nothing_changed(self):
        self.seed_feedback([("no-random-expression", 3), ("slow-start", 1)])
        self.assertTrue(idea_forge.export_feedback_note().startswith("updated"))
        text = rd(self.export)
        self.assertTrue(text.startswith("# Idea Forge feedback for the engine"))
        self.assertIn("fixes.json", text)
        self.assertLess(text.index("no-random-expression"), text.index("slow-start"))
        mtime = os.stat(self.export).st_mtime_ns
        self.assertTrue(idea_forge.export_feedback_note().startswith("unchanged"))
        self.assertEqual(os.stat(self.export).st_mtime_ns, mtime, "an unchanged backlog must not wake the nightly job's feedback gate")
        self.seed_feedback([("another-problem", 2)], "2026-10-09-2d-more")
        self.assertTrue(idea_forge.export_feedback_note().startswith("updated"))
        self.assertIn("another-problem", rd(self.export))

    def test_a_fixed_issue_leaves_the_export_and_a_recurring_one_is_flagged(self):
        self.fixes_on_main["fixes.json"] = json.dumps([{"key": "slow-start", "date": "2026-10-08", "summary": "fixed"}])
        self.seed_feedback([("no-random-expression", 3), ("slow-start", 1)])
        idea_forge.export_feedback_note()
        self.assertNotIn("`slow-start`", rd(self.export))
        self.seed_feedback([("slow-start", 1)], "2026-10-10-2d-again")
        idea_forge.export_feedback_note()
        self.assertIn("`slow-start` [recurring]", rd(self.export))

    def test_the_night_exports_after_the_games_and_does_not_run_the_improver_by_default(self):
        try:
            idea_forge.main(["nightly", "--claude", self.claude, "--quiet", "--date", "2026-10-09", "--order", "2d,3d"])
        except SystemExit:
            pass
        events = [l.split()[:2] for l in rd(self.log).splitlines()]
        self.assertEqual(events, [["start", "2d"], ["end", "2d"], ["start", "3d"], ["end", "3d"]], "no engine-improvement agent unless asked")
        self.assertEqual(self.state()["nights"]["2026-10-09"]["improve"], "off")
        self.assertIn("no-random-expression", rd(self.export), "the games' own findings reached the file the nightly job watches")

    def test_a_night_with_too_little_disk_is_skipped_and_says_so(self):
        def night(free, *extra):
            """One night on a machine with `free` GB free; returns (exit code, what it printed)."""
            for path in (self.log, os.path.join(self.home, "state.json")):
                if os.path.exists(path):
                    os.remove(path)
            out = io.StringIO()
            code = None
            with mock.patch.object(idea_forge, "free_gb", lambda: free), contextlib.redirect_stdout(out):
                try:
                    idea_forge.main(["nightly", "--claude", self.claude, "--quiet", "--date", "2026-10-09", "--order", "2d,3d", *extra])
                except SystemExit as e:
                    code = e.code
            return code, out.getvalue()

        code, said = night(3.0)
        self.assertEqual(code, 0, "a skipped night is not a failure")
        self.assertIn("only 3 GB free (< 20 GB)", said)
        self.assertEqual(self.state()["nights"]["2026-10-09"]["skipped"], "3 GB free")
        self.assertFalse(os.path.exists(self.log), "no agent was started")
        night(20.0)   # exactly the threshold is enough
        self.assertNotIn("skipped", self.state()["nights"]["2026-10-09"])
        self.assertTrue(os.path.exists(self.log), "the night ran its agents")
        night(19.9)
        self.assertEqual(self.state()["nights"]["2026-10-09"]["skipped"], "20 GB free", "the message rounds, the comparison does not")
        self.assertFalse(os.path.exists(self.log))
        night(3.0, "--min-free-gb", "2")   # the threshold is the caller's
        self.assertNotIn("skipped", self.state()["nights"]["2026-10-09"])
        code, said = night(3.0, "--dry-run")
        self.assertNotIn("skipped", said, "a dry run only prints the plan, whatever the disk")


@NEEDS_SH
class SystemdSchedule(unittest.TestCase):
    """The owner's other jobs are systemd user timers; this one is too."""

    def test_the_units_run_at_low_priority_with_the_path_baked_in(self):
        home()
        service, timer = idea_forge.systemd_units(["06:00", "18:30"], 15, "some-model", 25, True)
        for must in ("Type=oneshot", "Nice=10", "IOSchedulingClass=best-effort", "TimeoutStartSec=10h", "Environment=PATH=", "Environment=IDEA_FORGE_HOME=", "Environment=PYTHONUNBUFFERED=1", "nightly --budget 15 --improve --improve-budget 25 --model some-model", "WorkingDirectory="):
            self.assertIn(must, service)
        self.assertEqual(timer.count("OnCalendar="), 2)
        self.assertIn("OnCalendar=*-*-* 06:00:00", timer)
        self.assertIn("OnCalendar=*-*-* 18:30:00", timer)
        self.assertIn("Persistent=false", timer)
        self.assertNotIn("--improve", idea_forge.systemd_units(["06:00"], 15)[0])
        with self.assertRaises(SystemExit):
            idea_forge.systemd_units(["25:00"], 15)

    def test_install_enables_the_timer_and_uninstall_removes_only_ours(self):
        home()
        d = tempfile.mkdtemp()
        calls = os.path.join(d, "systemctl.log")
        sh(os.path.join(d, "systemctl"), f"#!/bin/sh\necho \"$@\" >> {calls}\nexit 0\n")
        env = {"PATH": d + os.pathsep + os.environ["PATH"], "HOME": d}
        other = os.path.join(d, ".config", "systemd", "user")
        os.makedirs(other)
        wr(os.path.join(other, "redengine-nightly.timer"), "theirs")
        cli("schedule", "--backend", "systemd", "--times", "06:00", "--install", env=env)
        self.assertTrue(os.path.isfile(os.path.join(other, "idea-forge.timer")))
        self.assertIn("OnCalendar=*-*-* 06:00:00", rd(os.path.join(other, "idea-forge.timer")))
        self.assertIn("ExecStart=", rd(os.path.join(other, "idea-forge.service")))
        log = rd(calls)
        self.assertIn("--user daemon-reload", log)
        self.assertIn("--user enable --now idea-forge.timer", log)
        printed = cli("schedule", "--backend", "systemd", "--times", "07:00", env=env).stdout
        self.assertIn("not installed", printed)
        self.assertIn("06:00", rd(os.path.join(other, "idea-forge.timer")), "printing changes nothing")
        cli("schedule", "--backend", "systemd", "--uninstall", env=env)
        self.assertFalse(os.path.exists(os.path.join(other, "idea-forge.timer")))
        self.assertFalse(os.path.exists(os.path.join(other, "idea-forge.service")))
        self.assertEqual(rd(os.path.join(other, "redengine-nightly.timer")), "theirs", "another job's units are never touched")
        self.assertIn("--user disable --now idea-forge.timer", rd(calls))


if __name__ == "__main__":
    unittest.main()
