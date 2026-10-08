#!/usr/bin/env python3
"""Tests for scripts/idea_forge.py (run by tests/idea_forge.rs, or directly: python3 scripts/test_idea_forge.py).

The model is replaced by a stub `claude` and the engine by a fake `red_engine2`, so the whole loop (idea -> worktree-like checkout -> agent -> notes -> score -> feedback -> ship guard) is observed
without a network, a GPU, Rust or a single token.
"""
import json
import os
import random
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import idea_forge  # noqa: E402

SCRIPT = os.path.join(HERE, "idea_forge.py")
WINDOWS = os.name == "nt"
NEEDS_SH = unittest.skipIf(WINDOWS, "a fake executable here is a shell script")


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


GOOD = {"id": "F1", "area": "format", "title": "no random", "severity": 3, "cost_min": 20, "evidence": "rules have no random expression", "workaround": "a die entity", "proposal": "add random(a,b)"}


def good_feedback(extra=None):
    rows = [GOOD] + (extra or [])
    return ("# run\n\n## The game\n\nA game.\n\n## Findings\n\n```json findings\n" + json.dumps(rows) + "\n```\n\n"
            "## Idea fit\n\nThe mechanic was expressible apart from the random part, which needed a workaround noted above.\n\n## What worked\n\nvalidate and verify were fast and precise.\n")


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
        lines = idea_forge.cron_lines(["09:30", "17:05"], 12, "some-model")
        self.assertEqual(len(lines), 2)
        self.assertTrue(lines[0].startswith("30 9 * * * cd "))
        self.assertTrue(lines[1].startswith("5 17 * * * cd "))
        for l in lines:
            for must in ("PATH=", "IDEA_FORGE_HOME=", "idea_forge.py", "daily --next --budget 12", "--model some-model", "daily.log", idea_forge.CRON_MARK):
                self.assertIn(must, l)
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
        cli("schedule", "--install", "--times", "10:00,18:00", env=env)
        cli("schedule", "--install", "--times", "10:00,18:00", env=env)
        lines = rd(store).splitlines()
        self.assertEqual(sum(1 for l in lines if idea_forge.CRON_MARK in l), 2, "installing twice must not duplicate")
        self.assertIn("0 3 * * * backup.sh", lines)
        printed = cli("schedule", "--times", "11:00", env=env).stdout
        self.assertIn("not installed", printed)
        self.assertEqual(len(rd(store).splitlines()), 3, "printing changes nothing")
        cli("schedule", "--uninstall", env=env)
        self.assertEqual(rd(store).splitlines(), ["0 3 * * * backup.sh"])


@NEEDS_SH
class DailyEndToEnd(unittest.TestCase):
    """A whole day in-process: a stub agent per game, a bare remote, a stub `gh`; worktrees are plain clones the tool then removes."""

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
        sh(os.path.join(self.base, "gh"), "#!/bin/sh\necho https://example.test/pull/$$\n")
        game2 = {"game2d": 1, "description": "A tiny game that proves the plumbing end to end.", "persist": ["b"], "sounds": {"a": {}}, "checks": {"scenarios": [{"smoke": True}, {}, {}]}}
        game3 = {"rules": [{"id": "r"}], "ui": {"c": 1}, "audio": {"a": 1}, "checks": {"sim": [{"name": "a"}, {"name": "b"}]}}
        stub = f"""#!{sys.executable}
import json, os, subprocess, sys
if os.environ.get("IDEA_FORGE_STUB_FAIL"):
    sys.exit(3)
run = os.environ["IDEA_FORGE_RUN"]
r = json.load(open(os.path.join(run, "run.json")))
slug, kind, date = r["slug"], r["kind"], r["date"]
cli = [sys.executable, {SCRIPT!r}]
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
print(json.dumps({{"type": "result", "total_cost_usd": 0.25, "duration_ms": 60000}}))
"""
        self.claude = sh(os.path.join(self.base, "claude"), stub)
        env = {"RED_ENGINE_EXE": self.engine, "PATH": self.base + os.pathsep + os.environ["PATH"], "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
        self.saved = {k: os.environ.get(k) for k in list(env) + ["IDEA_FORGE_STUB_FAIL"]}
        os.environ.update(env)
        self.addCleanup(self.restore)

        def fake_worktree(slug, kind="2d"):
            wt = os.path.join(self.base, f"wt-{kind}-{slug}")
            subprocess.run(["git", "clone", "-q", self.remote, wt], check=True, capture_output=True)
            git(wt, "checkout", "-q", "-b", f"idea-{kind}-{slug}")
            return wt, f"idea-{kind}-{slug}"

        patches = [mock.patch.object(idea_forge, "make_worktree", fake_worktree), mock.patch.object(idea_forge, "remove_worktree", lambda wt: self.removed.append(wt))]
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

    def test_an_agent_that_makes_nothing_is_retried_once_then_given_up_on(self):
        os.environ["IDEA_FORGE_STUB_FAIL"] = "1"
        self.assertEqual(self.daily("--order", "2d,3d", "--next"), 1)
        slot = self.day()["slots"]["2d"]
        self.assertEqual((slot["status"], slot["attempts"]), ("failed", 1))
        self.assertEqual(self.daily("--next"), 1)
        self.assertEqual(self.day()["slots"]["2d"]["attempts"], 2)
        self.assertEqual(self.daily("--next"), 1, "the next slot (3d) is tried; 2d is not retried a third time")
        self.assertEqual(self.day()["slots"]["3d"]["attempts"], 1)
        self.assertEqual(self.day()["slots"]["2d"]["attempts"], 2)
        self.assertEqual(self.removed, [], "a failed run keeps its worktree for a human")

    def test_no_ship_builds_and_keeps_the_worktree(self):
        self.assertEqual(self.daily("--order", "2d,3d", "--next", "--no-ship"), 0)
        self.assertEqual(self.removed, [])
        self.assertIn("not shipped", self.day()["slots"]["2d"]["detail"])


if __name__ == "__main__":
    unittest.main()
