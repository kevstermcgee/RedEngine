#!/usr/bin/env python3
"""Source identity and task recovery of the launchpad against REAL git repositories (run by tests/launchpad.rs, or directly: python3 scripts/test_launchpad_git.py).

`scripts/test_launchpad.py` uses throwaway directories that are not repositories; the defects found in October 2026 live exactly where git's output is read, so these tests
make real repositories and make every kind of change in them. The original defect: `git status --porcelain` text was `.strip()`ped, which ate the leading space of the first line,
and a fixed-width slice then turned ` M src/lib.rs` into `rc/lib.rs`; the hash of a file that could not be found is the same whatever its content was, so two different edits
of one file recorded the same identity.

Nothing here needs Rust, a GPU or the network, only `git`.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import launchpad  # noqa: E402
import red_resolve  # noqa: E402
import test_launchpad as base  # noqa: E402

WINDOWS = os.name == "nt"
HAVE_GIT = shutil.which("git") is not None
NO_GIT = unittest.skipUnless(HAVE_GIT, "git is not installed")
POSIX_ONLY = unittest.skipIf(WINDOWS, "this file name cannot exist on Windows")


def git(root, *args, check=True):
    cmd = ["git", "-C", root, "-c", "user.name=tester", "-c", "user.email=tester@example.invalid", "-c", "commit.gpgsign=false", "-c", "core.autocrlf=false", *args]
    p = subprocess.run(cmd, capture_output=True, text=True, env={**os.environ, "GIT_CONFIG_NOSYSTEM": "1"})
    if check and p.returncode != 0:
        raise AssertionError(f"git {' '.join(args)} failed: {p.stderr}")
    return p.stdout


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def save_json(path, data):
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f)


def base_read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def commit_all(root, message="change"):
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", message)
    return git(root, "rev-parse", "HEAD").strip()


def init_repo(root):
    git(root, "init", "-q")
    base.write(os.path.join(root, ".gitignore"), "out/\ntarget/\n__pycache__/\n")
    return commit_all(root, "baseline")


class GitCase(unittest.TestCase):
    def setUp(self):
        if not HAVE_GIT:
            self.skipTest("git is not installed")
        self.tmp = tempfile.mkdtemp(prefix="re2_lpgit_")
        self.root = os.path.join(self.tmp, "repo")
        os.makedirs(self.root)
        base.write(os.path.join(self.root, "src", "lib.rs"), "pub fn one() {}\n")
        base.write(os.path.join(self.root, "src", "net", "server.rs"), "fn serve() {}\n")
        base.write(os.path.join(self.root, "docs", "notes.md"), "notes\n")
        self.head = init_repo(self.root)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def path(self, rel):
        return os.path.join(self.root, *rel.split("/"))

    def edit(self, rel, text):
        base.write(self.path(rel), text)

    def changed(self):
        return sorted(launchpad.git_changed_files(self.root))

    def inputs(self, track=()):
        return launchpad.engine_inputs(self.root, track=track)


class TheOriginalCorruption(GitCase):
    def test_an_unstaged_edit_keeps_its_whole_path(self):
        self.edit("src/lib.rs", "pub fn two() {}\n")
        self.assertIn(" M src/lib.rs", git(self.root, "status", "--porcelain"), "the porcelain line that used to lose its first characters")
        self.assertEqual(self.changed(), ["src/lib.rs"], "was 'rc/lib.rs': a stripped leading space and a fixed-width slice")
        self.assertEqual(sorted(self.inputs()["files"]), ["src/lib.rs"])

    def test_the_old_reading_of_the_same_output_is_what_broke_it(self):
        self.edit("src/lib.rs", "pub fn two() {}\n")
        stripped = red_resolve._git(self.root, "status", "--porcelain")  # the text the old code sliced
        self.assertEqual(stripped, "M src/lib.rs", "stripping removes the leading space of an unstaged change")
        self.assertEqual(stripped[3:], "rc/lib.rs", "...which is exactly the corrupted path")

    def test_different_contents_of_one_file_are_different_identities(self):
        self.edit("src/lib.rs", "pub fn two() {}\n")
        a = self.inputs()
        self.edit("src/lib.rs", "pub fn three() {}\n")
        b = self.inputs()
        self.assertNotEqual(a["hash"], b["hash"], "two different edits of one file recorded the same identity")
        self.assertNotEqual(a["files"]["src/lib.rs"], b["files"]["src/lib.rs"])
        self.edit("src/lib.rs", "pub fn two() {}\n")
        self.assertEqual(self.inputs()["hash"], a["hash"], "the same content is the same identity again")

    def test_the_first_and_the_only_changed_file_is_read_like_every_other(self):
        for rel in ("src/lib.rs", "docs/notes.md", "src/net/server.rs"):
            git(self.root, "checkout", "-q", "--", ".")
            self.edit(rel, "// edited\n")
            self.assertEqual(self.changed(), [rel], rel)
            self.assertTrue(launchpad.file_sha(self.path(rel)) in self.inputs()["files"].values())


class EveryKindOfChange(GitCase):
    def test_staged_unstaged_untracked_deleted_and_renamed_are_all_there(self):
        self.edit("src/lib.rs", "pub fn staged() {}\n")
        git(self.root, "add", "src/lib.rs")
        self.edit("src/net/server.rs", "fn unstaged() {}\n")
        self.edit("brand_new.rs", "// new\n")
        os.remove(self.path("docs/notes.md"))
        got = self.inputs()["files"]
        self.assertEqual(sorted(got), ["brand_new.rs", "docs/notes.md", "src/lib.rs", "src/net/server.rs"])
        self.assertEqual(got["docs/notes.md"], "deleted")
        self.assertEqual(got["src/lib.rs"], launchpad.file_sha(self.path("src/lib.rs")))
        self.assertEqual(got["brand_new.rs"], launchpad.file_sha(self.path("brand_new.rs")))

    def test_a_staged_change_that_is_then_edited_again_is_its_latest_content(self):
        self.edit("src/lib.rs", "pub fn staged() {}\n")
        git(self.root, "add", "src/lib.rs")
        staged = self.inputs()
        self.edit("src/lib.rs", "pub fn staged_then_edited() {}\n")
        self.assertEqual(git(self.root, "status", "--porcelain").splitlines()[0][:2], "MM")
        self.assertNotEqual(self.inputs()["hash"], staged["hash"])

    def test_a_staged_deletion_and_an_unstaged_one_are_both_deleted(self):
        git(self.root, "rm", "-q", "docs/notes.md")
        os.remove(self.path("src/lib.rs"))
        got = self.inputs()["files"]
        self.assertEqual((got["docs/notes.md"], got["src/lib.rs"]), ("deleted", "deleted"))

    def test_a_rename_records_the_new_content_and_that_the_old_name_is_gone(self):
        git(self.root, "mv", "src/lib.rs", "src/renamed lib.rs")
        got = self.inputs()["files"]
        self.assertEqual(got["src/lib.rs"], "deleted")
        self.assertEqual(got["src/renamed lib.rs"], launchpad.file_sha(self.path("src/renamed lib.rs")))
        self.assertEqual(self.changed(), ["src/lib.rs", "src/renamed lib.rs"])
        # edited after the rename: still one rename entry, new content
        before = self.inputs()["hash"]
        self.edit("src/renamed lib.rs", "pub fn renamed_and_edited() {}\n")
        self.assertNotEqual(self.inputs()["hash"], before)

    def test_a_new_directory_lists_its_files_and_not_just_the_directory(self):
        for n in range(3):
            self.edit(f"fresh/deep/f{n}.rs", f"// {n}\n")
        self.assertEqual(sorted(self.inputs()["files"]), [f"fresh/deep/f{n}.rs" for n in range(3)], "git's default collapses a new directory into 'fresh/'")
        before = self.inputs()["hash"]
        self.edit("fresh/deep/f1.rs", "// changed inside a directory git would have collapsed\n")
        self.assertNotEqual(self.inputs()["hash"], before)

    def test_ignored_files_are_not_part_of_the_identity(self):
        self.edit("out/log.txt", "noise\n")
        self.edit("target/debug/x", "noise\n")
        self.assertEqual(self.inputs()["files"], {})

    def test_nothing_changed_is_an_empty_identity_and_it_is_stable(self):
        a, b = self.inputs(), self.inputs()
        self.assertEqual((a["files"], a["hash"], a["settled"]), ({}, b["hash"], True))

    def test_a_symlink_is_its_target_not_the_file_it_points_at(self):
        if WINDOWS:
            self.skipTest("symlinks need privileges on Windows")
        os.symlink("lib.rs", self.path("src/link.rs"))
        a = self.inputs()["files"]["src/link.rs"]
        os.remove(self.path("src/link.rs"))
        os.symlink("net/server.rs", self.path("src/link.rs"))
        self.assertTrue(a.startswith("symlink:"))
        self.assertNotEqual(a, self.inputs()["files"]["src/link.rs"])

    def test_more_than_400_changed_files_are_all_in_the_identity(self):
        for n in range(450):
            self.edit(f"many/f{n:03}.txt", f"{n}\n")
        files = self.inputs()["files"]
        self.assertEqual(len(files), 450, "the old identity silently dropped everything after the 400th file")
        before = self.inputs()["hash"]
        self.edit("many/f449.txt", "changed\n")
        self.assertNotEqual(self.inputs()["hash"], before, "an edit to the last one is seen")

    def test_a_file_too_big_to_read_is_still_part_of_the_identity(self):
        self.edit("big.bin", "x" * 100)
        old, launchpad.BIG_FILE = launchpad.BIG_FILE, 10
        try:
            a = self.inputs()["files"]["big.bin"]
            self.assertTrue(a.startswith("stat:100:"), a)
            time.sleep(0.02)
            self.edit("big.bin", "y" * 101)
            self.assertNotEqual(self.inputs()["files"]["big.bin"], a)
        finally:
            launchpad.BIG_FILE = old
        self.edit("big.bin", "x" * 100)
        self.assertEqual(self.inputs()["files"]["big.bin"], launchpad.file_sha(self.path("big.bin")), "a 4 MB file used to be skipped for good")


class UnusualNames(GitCase):
    NAMES = ["with space.rs", "tab\there.rs" if not WINDOWS else "tab_here.rs", "quote'single.rs", "unicode-ünï-文件.rs", "-leading-dash.rs", "a -> b.rs", "semi;colon&amp.rs", "dir with space/inner file.rs", "[brackets] {braces}.rs", "100%.rs"]

    def test_names_come_back_exactly_and_their_content_is_found(self):
        for n in self.NAMES:
            self.edit(n, f"// {n}\n")
        got = self.inputs()["files"]
        self.assertEqual(sorted(got), sorted(self.NAMES))
        for n in self.NAMES:
            self.assertEqual(got[n], launchpad.file_sha(self.path(n)), n)

    @POSIX_ONLY
    def test_quotes_and_newlines_in_names(self):
        names = ['double"quote.rs', "new\nline.rs", "back\\slash.rs"]
        for n in names:
            self.edit(n, "x\n")
        self.assertEqual(sorted(self.inputs()["files"]), sorted(names))

    @POSIX_ONLY
    def test_a_name_that_is_not_valid_utf8_survives(self):
        raw = os.fsencode(self.root) + b"/bad-\xff-name.rs"
        with open(raw, "wb") as f:
            f.write(b"x")
        files = self.inputs()["files"]
        self.assertEqual(len(files), 1)
        (name,) = files
        self.assertEqual(os.fsencode(name), b"bad-\xff-name.rs")
        self.assertEqual(json.loads(json.dumps(files)), files, "and it survives being written to the task file and read back")

    def test_a_renamed_file_with_odd_names_on_both_sides(self):
        self.edit("old name -> x.rs", "// a\n")
        commit_all(self.root)
        git(self.root, "mv", "old name -> x.rs", "new name -> y.rs")
        self.assertEqual(self.changed(), ["new name -> y.rs", "old name -> x.rs"])

    def test_the_pure_parser_reads_every_shape_of_porcelain_z(self):
        raw = b" M src/lib.rs\0M  staged.rs\0MM both.rs\0?? new file.rs\0 D gone.rs\0R  new -> name.rs\0old name.rs\0C  copy.rs\0orig.rs\0UU conflict.rs\0A  added.rs\0"
        got = [(e["index"] + e["worktree"], e["path"], e["orig"]) for e in red_resolve.parse_status_z(raw)]
        self.assertEqual(got, [(" M", "src/lib.rs", None), ("M ", "staged.rs", None), ("MM", "both.rs", None), ("??", "new file.rs", None), (" D", "gone.rs", None),
                               ("R ", "new -> name.rs", "old name.rs"), ("C ", "copy.rs", "orig.rs"), ("UU", "conflict.rs", None), ("A ", "added.rs", None)])
        self.assertEqual(red_resolve.parse_status_z(b""), [])
        self.assertEqual(red_resolve.parse_status_z(b"\0\0"), [], "empty fields are ignored, never an empty path")

    def test_paths_use_forward_slashes_whatever_the_platform(self):
        self.edit("a/b c/d.rs", "x\n")
        (name,) = self.inputs()["files"]
        self.assertEqual(name, "a/b c/d.rs")
        self.assertEqual(launchpad.path_fingerprint(self.root, name), launchpad.file_sha(self.path(name)), "and a '/'-separated name opens the file on every platform")


class ConcurrentEdits(GitCase):
    def test_a_file_rewritten_while_it_is_read_is_read_again(self):
        self.edit("src/lib.rs", "pub fn first() {}\n")
        real, calls = launchpad.file_sha, []

        def racing(path):
            calls.append(path)
            digest = real(path)
            if len(calls) == 1:
                time.sleep(0.02)
                self.edit("src/lib.rs", "pub fn second_edit_during_the_read() {}\n")
            return digest

        launchpad.file_sha = racing
        try:
            fp = launchpad.path_fingerprint(self.root, "src/lib.rs")
        finally:
            launchpad.file_sha = real
        self.assertEqual(fp, real(self.path("src/lib.rs")), "the second, settled read is what is recorded, not the torn first one")
        self.assertGreaterEqual(len(calls), 2)

    def test_a_file_that_never_stops_changing_is_unstable_and_never_equal(self):
        self.edit("src/lib.rs", "pub fn first() {}\n")
        real, n = launchpad.file_sha, [0]

        def always(path):
            digest = real(path)
            n[0] += 1
            self.edit("src/lib.rs", f"pub fn edit_{n[0]}() {{}}\n")
            return digest

        launchpad.file_sha = always
        try:
            inputs = self.inputs()
        finally:
            launchpad.file_sha = real
        self.assertEqual(inputs["files"]["src/lib.rs"], launchpad.UNSTABLE)
        self.assertFalse(inputs["settled"])
        self.assertFalse(launchpad.same_content(launchpad.UNSTABLE, launchpad.UNSTABLE))

    def test_head_moving_during_a_scan_is_rescanned(self):
        self.edit("src/lib.rs", "pub fn one_more() {}\n")
        real, moved = launchpad.path_fingerprint, []

        def commits_once(root, rel):
            if not moved:
                moved.append(commit_all(self.root, "someone else commits in the middle of the scan"))
            return real(root, rel)

        launchpad.path_fingerprint = commits_once
        try:
            inputs = self.inputs()
        finally:
            launchpad.path_fingerprint = real
        self.assertEqual(inputs["head"], moved[0], "the scan ran again against the new HEAD")
        self.assertEqual(inputs["files"], {}, "and the committed edit is no longer a change")
        self.assertTrue(inputs["settled"])


class Recovery(base.Sandbox):
    """Resuming a task in a real repository: what moved, what is still valid, what to do first."""

    def setUp(self):
        if not HAVE_GIT:
            self.skipTest("git is not installed")
        super().setUp()
        os.makedirs(os.path.join(self.engine, "scripts"), exist_ok=True)
        for f in ("launchpad.py", "red_resolve.py"):
            shutil.copy(os.path.join(HERE, f), os.path.join(self.engine, "scripts", f))
        base.write(os.path.join(self.engine, "src", "net", "server.rs"), "fn serve() {}\n")
        self.head = init_repo(self.engine)
        out, _ = self.lp("start", "fix a bug in src/net/server.rs", "--workflow", "engine-change")
        self.task = out["task"]["id"]
        self.start = out

    def resume(self, *extra):
        return self.lp("resume", *extra)[0]

    def edit(self, rel, text):
        base.write(os.path.join(self.engine, *rel.split("/")), text)

    def test_resume_with_nothing_changed_reuses_everything_and_keeps_the_plan(self):
        out = self.resume()
        s = out["since_start"]
        self.assertEqual(s["changes"], [])
        self.assertEqual((s["start_revision"], s["current_revision"], s["revision"]["relation"]), (self.head[:10], self.head[:10], "same"))
        self.assertEqual(out["next_action"], self.start["next_action"], "nothing to correct: the planned first step stands")
        self.assertNotIn("planned_next_action", out)
        self.assertGreaterEqual({r["what"] for r in s["reuse"]}, {"workflow and objective", "feature owners and context pointers", "engine CLI"})
        self.assertEqual(s["invalidated"], [])

    def test_an_edit_after_the_interruption_is_named_by_its_real_path_and_the_next_action_is_the_cheap_check(self):
        self.edit("src/net/server.rs", "fn serve() { /* edited after the interruption */ }\n")
        out = self.resume()
        ch = {c["what"]: c for c in out["since_start"]["changes"]}
        self.assertEqual(ch["source or game inputs"]["files"], ["src/net/server.rs"], "was rc/net/server.rs")
        self.assertFalse(out["since_start"]["recorded_results_trusted"])
        self.assertEqual(out["next_action"]["argv"][-1], "iterate")
        self.assertEqual(out["planned_next_action"]["summary"], self.start["next_action"]["summary"], "the original plan is kept alongside the correction")
        self.assertTrue(all(o["state"] == "stale" for o in out["evidence"]["observed"]))

    def test_two_different_edits_since_the_start_are_two_different_recoveries(self):
        self.edit("src/net/server.rs", "fn serve() { 1 }\n")
        a = self.resume()
        self.edit("src/net/server.rs", "fn serve() { 2 }\n")
        b = self.resume()
        for out in (a, b):
            self.assertEqual(out["since_start"]["changes"][0]["files"], ["src/net/server.rs"])
        self.assertNotEqual(self.start["identity"]["inputs_hash"], a["identity"]["inputs_hash"], "an edited tree is not the clean one the task started from")
        self.assertNotEqual(a["identity"]["inputs_hash"], b["identity"]["inputs_hash"], "different content, different identity (it used to be the same)")

    def test_an_edit_that_was_made_and_then_undone_is_not_a_change(self):
        original = base_read(os.path.join(self.engine, "src", "net", "server.rs"))
        self.edit("src/net/server.rs", "fn serve() { /* temp */ }\n")
        self.assertEqual(self.resume()["since_start"]["changes"][0]["files"], ["src/net/server.rs"])
        self.edit("src/net/server.rs", original)
        self.assertEqual(self.resume()["since_start"]["changes"], [])

    def test_an_edit_present_at_the_start_and_committed_unchanged_is_not_an_edit_but_the_revision_moved(self):
        self.edit("src/net/server.rs", "fn serve() { /* work in progress */ }\n")
        out, _ = self.lp("start", "keep going on the server", "--workflow", "engine-change")
        wip = commit_all(self.engine, "commit my own work, unchanged")
        out = self.resume("--task-id", out["task"]["id"])
        s = out["since_start"]
        self.assertEqual([c["what"] for c in s["changes"]], ["engine revision"], "same content as when the task started: only the revision moved")
        self.assertEqual((s["revision"]["relation"], s["revision"]["commits"], s["current_revision"]), ("advanced", 1, wip[:10]))
        self.assertEqual(s["revision"]["files"], ["src/net/server.rs"])

    def test_the_same_file_committed_with_other_content_is_an_edit(self):
        self.edit("src/net/server.rs", "fn serve() { /* work in progress */ }\n")
        out, _ = self.lp("start", "keep going on the server", "--workflow", "engine-change")
        self.edit("src/net/server.rs", "fn serve() { /* finished differently */ }\n")
        commit_all(self.engine)
        names = {c["what"]: c for c in self.resume("--task-id", out["task"]["id"])["since_start"]["changes"]}
        self.assertEqual(names["source or game inputs"]["files"], ["src/net/server.rs"])

    def test_another_agent_advancing_the_branch_is_read_before_anything_else(self):
        self.edit("src/net/other_agent.rs", "// somebody else's work\n")
        theirs = commit_all(self.engine, "another agent's commit")
        out = self.resume()
        s = out["since_start"]
        self.assertEqual((s["revision"]["relation"], s["revision"]["commits"], s["current_revision"]), ("advanced", 1, theirs[:10]))
        self.assertEqual(s["revision"]["files"], ["src/net/other_agent.rs"], "committed files are named even though the working tree is clean")
        na = out["next_action"]
        self.assertEqual(na["argv"][:4], ["git", "-C", self.engine, "log"])
        self.assertIn(f"{self.head}..{theirs}", na["argv"])
        self.assertTrue(all(o["state"] == "stale" for o in out["evidence"]["observed"]))

    def test_a_rewritten_history_is_diverged_and_a_pruned_one_is_unknown(self):
        self.edit("src/net/server.rs", "fn serve() { /* amended */ }\n")
        git(self.engine, "add", "-A")
        git(self.engine, "commit", "-q", "--amend", "-m", "baseline, amended")
        self.assertEqual(self.resume()["since_start"]["revision"]["relation"], "diverged")
        out, _ = self.lp("start", "again", "--workflow", "engine-change")
        t = os.path.join(self.engine, "out", "launchpad", out["task"]["id"] + ".json")
        rec = load_json(t)
        rec["identity"]["engine_head"] = "0" * 40
        save_json(t, rec)
        s = self.resume("--task-id", out["task"]["id"])["since_start"]
        self.assertEqual(s["revision"]["relation"], "unknown")
        self.assertIn("not in this repository", s["revision"]["why"])

    def test_a_file_deleted_or_renamed_since_the_start_is_a_change_under_both_names(self):
        git(self.engine, "mv", "src/net/server.rs", "src/net/renamed server.rs")
        files = self.resume()["since_start"]["changes"][0]["files"]
        self.assertEqual(files, ["src/net/renamed server.rs", "src/net/server.rs"])
        git(self.engine, "checkout", "-q", "HEAD", "--", ".")
        git(self.engine, "reset", "-q", "--hard")
        os.remove(os.path.join(self.engine, "src", "lib.rs"))
        self.assertEqual(self.resume()["since_start"]["changes"][0]["files"], ["src/lib.rs"])

    def test_an_untracked_new_file_after_the_start_is_a_change(self):
        self.edit("src/brand new/module.rs", "// new\n")
        self.assertEqual(self.resume()["since_start"]["changes"][0]["files"], ["src/brand new/module.rs"])

    def test_a_task_recorded_by_the_old_launchpad_is_not_trusted_and_says_why(self):
        t = os.path.join(self.engine, "out", "launchpad", self.task + ".json")
        rec = load_json(t)
        rec["identity"].pop("identity_version")
        rec["identity"]["inputs"]["files"] = {"rc/lib.rs": None}
        save_json(t, rec)
        out = self.resume()
        names = [c["what"] for c in out["since_start"]["changes"]]
        self.assertIn("recorded identity predates the git parsing fix", names)
        self.assertFalse(out["since_start"]["recorded_results_trusted"])

    def test_files_still_changing_say_so_and_the_next_action_is_to_wait(self):
        self.edit("src/lib.rs", "pub fn racing() {}\n")
        script = os.path.join(self.engine, "scripts", "launchpad.py")
        with open(script, encoding="utf-8") as f:
            text = f.read()
        race = ("import random\n\n\ndef _race(path):\n    if path.endswith('lib.rs'):\n        with open(path, 'a') as f:\n            f.write('// ' + str(random.random()) + '\\n')\n\n\n"
                "def file_sha(path):\n    _race(path)\n")
        patched = text.replace("def file_sha(path):\n", race, 1)
        with open(script, "w", encoding="utf-8") as f:
            f.write(patched)
        out = self.resume()
        self.assertIn("source still changing", [c["what"] for c in out["since_start"]["changes"]])
        self.assertEqual(out["next_action"]["kind"], "wait")


class ProjectTasks(base.Sandbox):
    """A game project pins an engine: edits to the engine checkout still count."""

    def setUp(self):
        if not HAVE_GIT:
            self.skipTest("git is not installed")
        super().setUp()
        os.makedirs(os.path.join(self.engine, "scripts"), exist_ok=True)
        for f in ("launchpad.py", "red_resolve.py"):
            shutil.copy(os.path.join(HERE, f), os.path.join(self.engine, "scripts", f))
        init_repo(self.engine)
        self.proj = self.project()
        out, _ = self.lp("start", "make the enemies faster", "--project", self.proj)
        self.task = out["task"]["id"]

    def test_an_edit_to_the_pinned_engine_is_reported_as_engine_source(self):
        base.write(os.path.join(self.engine, "src", "lib.rs"), "// edited engine\n")
        out = self.lp("resume", "--project", self.proj)[0]
        ch = {c["what"]: c for c in out["since_start"]["changes"]}
        self.assertEqual(ch["engine source"]["files"], ["src/lib.rs"])
        self.assertFalse(out["since_start"]["recorded_results_trusted"])
        self.assertNotIn("source or game inputs", ch, "the project's own files did not change")

    def test_a_project_edit_still_has_its_old_name_and_its_own_next_action(self):
        base.write(os.path.join(self.proj, "maps", "main.json"), '{"edited": true}', time.time() + 5)
        out = self.lp("resume", "--project", self.proj)[0]
        self.assertEqual(out["since_start"]["changes"][0]["files"], ["maps/main.json"])
        self.assertEqual(out["next_action"]["argv"][:2], ["scripts/red", "check"])


if __name__ == "__main__":
    unittest.main(verbosity=1)
