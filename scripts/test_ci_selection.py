#!/usr/bin/env python3
"""Which hosted CI jobs a change starts, tested by running the workflow's own "what changed" step (run by tests/ci_selection.rs, or directly: python3 scripts/test_ci_selection.py).

The step in `.github/workflows/ci.yml` (`id: f`) is extracted as it is written, its `${{ ... }}` expressions are filled in, and it runs under bash in a throwaway repository whose second
commit changes exactly the files a test names. Nothing here re-implements the patterns, so editing the workflow edits what is tested.

The gap this pins (ADR 2026-10-09-ci-runs-the-python-tools-on-windows-when-they-change): a change to launchpad.py, idea_forge.py, proc_supervisor.py or a PowerShell script started no Windows
job, because the Windows leg only looked at what the Rust build and its tests touch.
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
WORKFLOW = os.path.join(os.path.dirname(HERE), ".github", "workflows", "ci.yml")
BASH = shutil.which("bash")
HAVE = bool(BASH and shutil.which("git"))


def step_script():
    """The `run:` block of the step `id: f`, de-indented, with the GitHub expressions replaced by shell variables."""
    lines = open(WORKFLOW, encoding="utf-8").read().splitlines()
    start = next(i for i, l in enumerate(lines) if l.strip() == "- id: f")
    head = next(i for i in range(start, len(lines)) if lines[i].strip() == "run: |")
    indent = len(lines[head + 1]) - len(lines[head + 1].lstrip())
    block = []
    for l in lines[head + 1:]:
        if l.strip() and len(l) - len(l.lstrip()) < indent:
            break
        block.append(l[indent:] if l.strip() else "")
    text = "\n".join(block)
    text = text.replace("${{ github.event_name }}", "pull_request").replace("${{ github.event.pull_request.base.sha }}", "$BASE").replace("${{ github.event.before }}", "$BASE")
    assert "${{" not in text, "the step uses an expression this test does not know how to fill in"
    return text


def git(repo, *args):
    p = subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false", "-c", "core.quotepath=true", *args],
                       capture_output=True, text=True)
    assert p.returncode == 0, p.stderr
    return p.stdout.strip()


@unittest.skipUnless(HAVE, "needs bash and git")
class WhichJobs(unittest.TestCase):
    repo = None

    def setUp(self):
        self.fresh()

    def fresh(self):
        """A new throwaway repository with one base commit (the previous one, if a test needs several, is removed)."""
        if self.repo:
            shutil.rmtree(self.repo, ignore_errors=True)
        self.repo = tempfile.mkdtemp(prefix="re2_ci_sel_")
        git(self.repo, "init", "-q")
        self.write("README.md", "base\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "base")
        self.base = git(self.repo, "rev-parse", "HEAD")

    def tearDown(self):
        shutil.rmtree(self.repo, ignore_errors=True)

    def write(self, rel, text="x\n"):
        path = os.path.join(self.repo, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8", newline="") as f:
            f.write(text)

    def jobs(self, *changed, base="real"):
        for rel in changed:
            self.write(rel, f"changed {rel}\n")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "change", "--allow-empty")
        env = dict(os.environ, BASE=self.base if base == "real" else base, GITHUB_OUTPUT="out.txt")
        p = subprocess.run([BASH, "-e", "-o", "pipefail", "-c", step_script()], cwd=self.repo, env=env, capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        with open(os.path.join(self.repo, "out.txt"), encoding="utf-8") as f:
            got = dict(l.strip().split("=", 1) for l in f if "=" in l)
        return {k: got[k] == "true" for k in ("native", "docker", "pytools", "games_only")}

    def assertJobs(self, changed, native, docker, pytools, games_only=False, **kw):
        self.assertEqual(self.jobs(*changed, **kw), {"native": native, "docker": docker, "pytools": pytools, "games_only": games_only}, f"for {changed}")

    def test_documentation_alone_starts_nothing(self):
        self.assertJobs(["docs/HOSTING.md", "README.md", "docs/adr/2026-10-09-x.md", "STATUS.md"], False, False, False)

    def test_every_python_tool_starts_the_python_job_and_not_the_rust_ones(self):
        for tool in ("scripts/launchpad.py", "scripts/red_resolve.py", "scripts/idea_forge.py", "scripts/proc_supervisor.py", "scripts/publish_games.py", "scripts/test_launchpad_git.py",
                     "scripts/test_proc_supervisor.py", "scripts/seed_target.py", "scripts/prune_target.py", "mcp_server.py", "benches/check.py", "tools/gen_karts.py", "bench/fresh-agent/run.py"):
            with self.subTest(tool):
                self.fresh()
                self.assertJobs([tool], native=tool.startswith("benches/"), docker=False, pytools=True)

    def test_shell_and_powershell_helpers_start_it_too(self):
        for helper in ("scripts/dev", "scripts/dev.ps1", "scripts/package_release.sh", "scripts/play_multiplayer.ps1", "scripts/bootstrap.sh", "games-publish.json"):
            with self.subTest(helper):
                self.fresh()
                self.assertEqual(self.jobs(helper)["pytools"], True)

    def test_a_python_file_anywhere_counts(self):
        self.assertEqual(self.jobs("examples/external/tool/helper.py")["pytools"], True)

    def test_a_name_git_would_quote_still_selects_the_job(self):
        # `git diff --name-only` prints "scripts/\303\274nder.py" in quotes by default: an anchored pattern never matches the opening quote.
        self.assertJobs(["scripts/ünder code.py"], False, False, True)

    def test_rust_only_changes_do_not_start_the_python_job(self):
        self.assertJobs(["src/net/relay.rs", "Cargo.toml"], True, True, False)

    def test_the_rust_tests_that_wrap_the_python_suites_reach_windows_through_the_native_job(self):
        self.assertJobs(["tests/launchpad.rs", "tests/idea_forge.rs"], True, False, False)

    def test_changing_ci_itself_starts_everything(self):
        self.assertJobs([".github/workflows/ci.yml"], True, True, True)

    def test_no_usable_base_runs_everything(self):
        for bad in ("", "0000000000000000000000000000000000000000", "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"):
            self.fresh()
            self.assertJobs(["docs/HOSTING.md"], True, True, True, base=bad)

    def test_a_pull_request_that_only_adds_generated_games_runs_neither_the_windows_leg_nor_the_image(self):
        self.assertJobs(["examples/2d/new-game.game2d.json", "docs/analysis/idea-forge/2026-10-10-2d-new-game.md"], False, False, False, games_only=True)
        self.fresh()
        self.assertJobs(["examples/3d/new-game/new-game.json", "examples/3d/new-game/notes.md", "docs/analysis/idea-forge/fixes.json"], False, False, False, games_only=True)

    def test_one_file_outside_the_games_makes_it_a_full_run(self):
        for other in ("src/lib.rs", "tests/games2d.rs", "examples/house.json", "examples/marcel/marcel.json", "recipes/x.json", "Cargo.lock", "docs/HOSTING.md", "assets/a.png", "examples/2dx/y.json"):
            with self.subTest(other):
                self.fresh()
                got = self.jobs("examples/2d/new-game.game2d.json", other)
                self.assertFalse(got["games_only"], f"games_only with {other}")
        self.fresh()
        self.assertJobs(["examples/2d/new-game.game2d.json", "src/net/relay.rs"], True, True, False)

    def test_a_change_that_touches_nothing_is_not_a_games_only_change(self):
        self.assertEqual(self.jobs()["games_only"], False)

    def test_a_games_change_with_an_unusable_base_runs_everything(self):
        self.assertJobs(["examples/2d/new-game.game2d.json"], True, True, True, base="")

    def test_a_game_with_a_python_helper_still_gets_the_python_job(self):
        self.assertEqual(self.jobs("examples/3d/x/make.py"), {"native": False, "docker": False, "pytools": True, "games_only": True})

    def test_a_large_change_list_is_still_read_whole(self):
        # Under pipefail `echo | grep -q` can fail when grep exits early; the helper reads a here-string. A few thousand files, the match last.
        names = [f"docs/analysis/f{i:05}.md" for i in range(3000)] + ["scripts/idea_forge.py"]
        self.assertEqual(self.jobs(*names)["pytools"], True)


if __name__ == "__main__":
    unittest.main(verbosity=1)
