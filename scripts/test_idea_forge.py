#!/usr/bin/env python3
"""Tests for scripts/idea_forge.py (run by tests/idea_forge.rs, or directly: python3 scripts/test_idea_forge.py).

The model is replaced by a stub `claude` and the engine by a fake `red_engine2`, so the whole loop (idea -> worktree-like checkout -> agent -> notes -> score -> feedback -> ship guard) is observed
without a network, a GPU, Rust or a single token.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

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
    e = dict(os.environ)
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
                     "Do not `git push`", "Do not change engine code", "YOU plant seeds", "scripts/dev red describe 2d", "idea-fit", "docs/analysis/idea-forge/2026-10-08-electric-foundry.md"):
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
        self.assertIn("2 run(s), 3 findings", text)
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
p = os.path.join("docs", "analysis", "idea-forge", "{self.today()}-{slug}.md")
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
        text = rd(os.path.join(self.wt, "docs", "analysis", "idea-forge", f"{self.today()}-{slug}.md"))
        self.assertEqual(idea_forge.check_feedback_text(text), [])
        self.assertIn("Friction report", text)
        self.assertIn("Agent session: 1 assistant turns", text)
        self.assertIn("$0.50", text)
        self.assertIn("exceed the measured wall time", text, "12 claimed minutes in an instant run must be called out")
        runs = json.load(open(os.path.join(self.home, "state.json")))["runs"]
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
        self.assertEqual(sorted(files), sorted([f"examples/2d/{slug}.game2d.json", f"docs/analysis/idea-forge/{self.today()}-{slug}.md"]))

    def test_ship_refuses_unfinished_feedback(self):
        self.run_it()
        slug = self.idea["slug"]
        p = os.path.join(self.wt, "docs", "analysis", "idea-forge", f"{self.today()}-{slug}.md")
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


if __name__ == "__main__":
    unittest.main()
