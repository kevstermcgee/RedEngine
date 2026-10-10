#!/usr/bin/env python3
"""Tests for scripts/prebuilt.py and its three callers (run by tests/prebuilt.rs, or directly: python3 scripts/test_prebuilt.py).

A prebuilt `red_engine2` is used only when a release was built from exactly the checkout's sources. These tests build a throwaway git checkout, make a "release" in a directory
(real archives, a real manifest from `write_manifest`, real checksums, fake binaries that print a marker) and serve it through `RED_RELEASE_DIR`, so the selection, the checksum
refusal, the fallback and the "no compile" claim are observed: a fake `cargo` leaves a marker file if anything compiles. No network, no Rust, no GPU.
"""
import contextlib
import hashlib
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import prebuilt  # noqa: E402
import red_resolve  # noqa: E402

os.environ.setdefault("GIT_AUTHOR_NAME", "t")
os.environ.setdefault("GIT_AUTHOR_EMAIL", "t@t")
os.environ.setdefault("GIT_COMMITTER_NAME", "t")
os.environ.setdefault("GIT_COMMITTER_EMAIL", "t@t")
LINUX = sys.platform.startswith("linux") and prebuilt.platform_name() == "linux-x86_64"
NEEDS_LINUX = unittest.skipUnless(LINUX, "prebuilt binaries are Linux x86_64 only")
BINARIES = ["red_engine2", "re2", "red_server", "red_bot", "red_relay"]


def write(path, text, mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    if mode:
        os.chmod(path, mode)
    return path


def git(root, *args):
    subprocess.run(["git", "-C", root, *args], check=True, capture_output=True)


def make_checkout(tmp, spec="spec v1\n"):
    """A tiny engine checkout under git: build inputs (one embedded by `include_str!` from outside src/), a doc that is NOT an input, and the scripts this feature ships."""
    root = os.path.join(tmp, "engine")
    write(os.path.join(root, "Cargo.toml"), "[package]\nname='red_engine2'\n")
    write(os.path.join(root, "Cargo.lock"), "")
    write(os.path.join(root, "build.rs"), 'const FOLDERS: &[(&str, &str, &str)] = &[("docs/adr", "adr_files.rs", "ADR_FILES")];\nfn main() {}\n')
    write(os.path.join(root, "src", "lib.rs"), 'pub const SPEC: &str = include_str!("../SPEC.md");\n')
    write(os.path.join(root, "SPEC.md"), spec)
    write(os.path.join(root, "docs", "adr", "0001-x.md"), "# adr\n")
    write(os.path.join(root, "docs", "HOSTING.md"), "not an input\n")
    write(os.path.join(root, "README.md"), "not an input\n")
    write(os.path.join(root, "docs", "features.json"), json.dumps({"features": {}}))
    os.makedirs(os.path.join(root, "scripts"), exist_ok=True)
    for name in ("dev", "prebuilt.py", "red_resolve.py", "launchpad.py", "bootstrap.sh"):
        shutil.copy(os.path.join(HERE, name), os.path.join(root, "scripts", name))
    git(root, "init", "-q", "-b", "main")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "init")
    return root


def commit(root, message="change"):
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", message)


def make_release(root, releases, tag, kinds=("full", "headless")):
    """A release directory `releases/<tag>` of the checkout's CURRENT sources, packaged the way `package_release.sh` does and described by the real `write_manifest`."""
    dist = os.path.join(releases, tag)
    os.makedirs(dist)
    stage = tempfile.mkdtemp(prefix="stage_")
    try:
        for kind in kinds:
            name = f"red-engine-{tag}-linux-x86_64" + ("-headless" if kind == "headless" else "")
            d = os.path.join(stage, name)
            for b in (BINARIES if kind == "full" else ["red_engine2"]):
                write(os.path.join(d, b), f'#!/bin/sh\necho "Red Engine 2 prebuilt {tag} {kind} {b}"\n', 0o755)
            write(os.path.join(d, "README.txt"), "readme\n")
            with tarfile.open(os.path.join(dist, name + ".tar.gz"), "w:gz") as tf:
                tf.add(d, arcname=name)
    finally:
        shutil.rmtree(stage)
    with open(os.path.join(dist, "SHA256SUMS"), "w") as f:
        for n in sorted(n for n in os.listdir(dist) if n.endswith(".tar.gz")):
            f.write(f"{prebuilt.sha256_file(os.path.join(dist, n))}  {n}\n")
    return prebuilt.write_manifest(root, tag, dist)


class Sandbox(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="prebuilt_test_")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = make_checkout(self.tmp)
        self.releases = os.path.join(self.tmp, "releases")
        os.makedirs(self.releases)
        self.target = os.path.join(self.tmp, "target")
        self.env = {**os.environ, "RED_RELEASE_DIR": self.releases, "CARGO_TARGET_DIR": self.target}
        self.env.pop("RED_NO_FETCH", None)
        self.asound = mock.patch.object(prebuilt, "have_libasound", lambda: True)
        self.asound.start()
        self.addCleanup(self.asound.stop)

    def ensure(self, **kw):
        return prebuilt.ensure(self.root, env=self.env, **kw)


class Fingerprint(Sandbox):
    def test_it_covers_what_goes_into_a_binary_and_nothing_else(self):
        paths = prebuilt.input_paths(self.root)
        self.assertIn("SPEC.md", paths, "a file include_str! embeds from outside src/ is an input")
        self.assertIn("docs/adr", paths, "a folder build.rs embeds is an input")
        self.assertNotIn("docs/HOSTING.md", paths)
        a = prebuilt.fingerprint(self.root)
        self.assertTrue(a["ok"], a)
        write(os.path.join(self.root, "README.md"), "edited\n")
        write(os.path.join(self.root, "docs", "HOSTING.md"), "edited\n")
        commit(self.root, "docs only")
        self.assertEqual(prebuilt.fingerprint(self.root)["fingerprint"], a["fingerprint"], "a change to a file that is not an input does not change the fingerprint")
        write(os.path.join(self.root, "SPEC.md"), "spec v2\n")
        commit(self.root, "spec")
        self.assertNotEqual(prebuilt.fingerprint(self.root)["fingerprint"], a["fingerprint"], "an embedded file is part of the binary")

    def test_an_uncommitted_edit_to_an_input_or_a_missing_git_checkout_has_no_fingerprint(self):
        write(os.path.join(self.root, "src", "lib.rs"), "pub fn changed() {}\n")
        f = prebuilt.fingerprint(self.root)
        self.assertFalse(f["ok"])
        self.assertIn("uncommitted", f["reason"])
        self.assertIn("src/lib.rs", f["reason"])
        plain = os.path.join(self.tmp, "plain")
        write(os.path.join(plain, "Cargo.toml"), "x")
        self.assertFalse(prebuilt.fingerprint(plain)["ok"])


class Selection(unittest.TestCase):
    FP = "a" * 64

    def test_the_release_built_from_these_sources_is_chosen(self):
        rels = [{"tag": "v2", "assets": ["MANIFEST-bbbbbbbbbbbb.json"]}, {"tag": "v1", "assets": ["x.tar.gz", prebuilt.manifest_name(self.FP)]}]
        self.assertEqual(prebuilt.select_release(self.FP, rels), {"tag": "v1", "manifest": "MANIFEST-aaaaaaaaaaaa.json"})

    def test_every_way_of_not_matching_says_which(self):
        self.assertIn("no release has been published", prebuilt.select_release(self.FP, [])["reason"])
        old = prebuilt.select_release(self.FP, [{"tag": "v0.1", "assets": ["a.tar.gz"]}])["reason"]
        self.assertIn("none carries a source manifest", old)
        other = prebuilt.select_release(self.FP, [{"tag": "v2", "assets": ["MANIFEST-bbbbbbbbbbbb.json"]}])["reason"]
        self.assertIn("built from different sources", other)
        self.assertIn("v2", other)

    def test_the_full_set_needs_sound_libraries_and_headless_can_be_asked_for(self):
        m = {"assets": [{"name": "f", "kind": "full"}, {"name": "h", "kind": "headless"}]}
        self.assertEqual(prebuilt.choose_asset(m, False, True), ({"name": "f", "kind": "full"}, None))
        asset, note = prebuilt.choose_asset(m, False, False)
        self.assertEqual(asset["kind"], "headless")
        self.assertIn("libasound", note)
        self.assertEqual(prebuilt.choose_asset(m, True, True)[0]["kind"], "headless")
        self.assertEqual(prebuilt.choose_asset({"assets": [{"name": "f", "kind": "full"}]}, True, True)[0], None)

    def test_checksum_lines_are_read_whatever_the_mode_marker(self):
        h = "0" * 64
        self.assertEqual(prebuilt.parse_sums(f"{h}  a.tar.gz\n{h} *b.tar.gz\nnot a line\n"), {"a.tar.gz": h, "b.tar.gz": h})


@NEEDS_LINUX
class Ensure(Sandbox):
    def test_a_release_built_from_these_sources_is_fetched_verified_and_used(self):
        make_release(self.root, self.releases, "v1")
        r = self.ensure()
        self.assertEqual((r["status"], r["version"], r["kind"]), ("fetched", "v1", "full"), r)
        out = subprocess.run([r["exe"], "describe", "--brief"], capture_output=True, text=True).stdout
        self.assertIn("prebuilt v1 full red_engine2", out)
        self.assertTrue(os.path.isfile(os.path.join(os.path.dirname(r["exe"]), "re2")), "the full set brings the client")
        with open(os.path.join(os.path.dirname(r["exe"]), "prebuilt.json")) as f:
            rec = json.load(f)
        self.assertEqual(rec["fingerprint"], prebuilt.fingerprint(self.root)["fingerprint"])
        self.assertEqual(rec["sha256"], prebuilt.sha256_file(os.path.join(self.releases, "v1", rec["asset"])))
        again = self.ensure(fetch=False)
        self.assertEqual((again["status"], again["exe"]), ("installed", r["exe"]), "once installed it needs no network")

    def test_without_sound_libraries_the_headless_cli_is_installed_and_says_so(self):
        make_release(self.root, self.releases, "v1")
        said = []
        with mock.patch.object(prebuilt, "have_libasound", lambda: False):
            r = self.ensure(log=said.append)
        self.assertEqual(r["kind"], "headless")
        self.assertTrue(any("libasound" in m for m in said), said)
        self.assertFalse(os.path.exists(os.path.join(os.path.dirname(r["exe"]), "re2")))

    def test_a_release_of_different_sources_is_not_used(self):
        make_release(self.root, self.releases, "v1")
        write(os.path.join(self.root, "src", "lib.rs"), "pub fn newer() {}\n")
        commit(self.root, "newer sources")
        r = self.ensure()
        self.assertEqual(r["status"], "none")
        self.assertIn("built from different sources", r["reason"])
        self.assertFalse(os.path.exists(os.path.join(self.target, "prebuilt", r["fingerprint"])), "nothing is installed")

    def test_no_release_at_all_falls_back_with_a_reason(self):
        r = self.ensure()
        self.assertEqual((r["status"], r["exe"]), ("none", None))
        self.assertIn("no release has been published", r["reason"])
        self.assertIn("building from source", prebuilt.describe_result(r))

    def test_a_corrupted_download_is_refused_and_nothing_is_installed(self):
        make_release(self.root, self.releases, "v1")
        archive = os.path.join(self.releases, "v1", "red-engine-v1-linux-x86_64.tar.gz")
        with open(archive, "ab") as f:
            f.write(b"tampered")
        r = self.ensure()
        self.assertEqual(r["status"], "none")
        self.assertIn("checksum mismatch", r["reason"])
        self.assertFalse(os.path.isdir(os.path.join(self.target, "prebuilt", r["fingerprint"])))
        self.assertEqual([n for n in os.listdir(os.path.join(self.target, "prebuilt")) if n.startswith(".fetch-")], [], "the scratch download is removed")

    def test_an_archive_the_checksum_file_does_not_list_is_refused(self):
        make_release(self.root, self.releases, "v1")
        sums = os.path.join(self.releases, "v1", "SHA256SUMS")
        lines = [l for l in open(sums) if "headless" in l or "MANIFEST" in l]
        open(sums, "w").write("".join(lines))
        r = self.ensure()
        self.assertEqual(r["status"], "none")
        self.assertIn("not listed in SHA256SUMS", r["reason"])

    def test_a_manifest_that_describes_other_sources_is_refused(self):
        make_release(self.root, self.releases, "v1")
        fp = prebuilt.fingerprint(self.root)["fingerprint"]
        mpath = os.path.join(self.releases, "v1", prebuilt.manifest_name(fp))
        m = json.load(open(mpath))
        m["fingerprint"] = "c" * 64
        json.dump(m, open(mpath, "w"))
        sums = prebuilt.parse_sums(open(os.path.join(self.releases, "v1", "SHA256SUMS")).read())
        sums[prebuilt.manifest_name(fp)] = prebuilt.sha256_file(mpath)
        open(os.path.join(self.releases, "v1", "SHA256SUMS"), "w").write("".join(f"{h}  {n}\n" for n, h in sums.items()))
        r = self.ensure()
        self.assertEqual(r["status"], "none")
        self.assertIn("does not describe this checkout", r["reason"])

    def test_uncommitted_edits_to_the_sources_mean_a_build_not_a_prebuilt(self):
        make_release(self.root, self.releases, "v1")
        write(os.path.join(self.root, "src", "lib.rs"), "pub fn edited() {}\n")
        r = self.ensure()
        self.assertEqual(r["status"], "none")
        self.assertIn("uncommitted", r["reason"])

    def test_fetching_can_be_switched_off_and_a_miss_is_not_asked_again_for_an_hour(self):
        make_release(self.root, self.releases, "v1")
        self.assertEqual(self.ensure(fetch=False)["status"], "none")
        self.assertEqual(prebuilt.ensure(self.root, env={**self.env, "RED_NO_FETCH": "1"})["status"], "none")
        shutil.rmtree(os.path.join(self.releases, "v1"))
        first = self.ensure()
        self.assertEqual(first["status"], "none")

        class Fails:
            def releases(self):
                raise AssertionError("the releases were asked for again within the hour")

        second = prebuilt.ensure(self.root, env=self.env, source=Fails())
        self.assertEqual(second["status"], "none")
        self.assertIn("checked", second["reason"])
        make_release(self.root, self.releases, "v2")
        self.assertEqual(self.ensure(refresh=True)["status"], "fetched", "--refresh looks again")

    def test_a_network_failure_falls_back_cleanly(self):
        class Down:
            def releases(self):
                raise OSError("name resolution failed")

        r = prebuilt.ensure(self.root, env=self.env, source=Down())
        self.assertEqual(r["status"], "none")
        self.assertIn("could not list the releases", r["reason"])

    def test_the_resolver_selects_it_only_while_the_sources_are_unchanged(self):
        make_release(self.root, self.releases, "v1")
        before = red_resolve.resolve(self.root, env=self.env)
        self.assertNotEqual(before["selected"]["status"], "ready")
        r = self.ensure()
        after = red_resolve.resolve(self.root, env=self.env)
        self.assertEqual((after["selected"]["status"], after["selected"]["exe"]), ("ready", r["exe"]))
        self.assertTrue(any("prebuilt release v1" in x for x in after["selected"]["reasons"]), after["selected"]["reasons"])
        write(os.path.join(self.root, "src", "lib.rs"), "pub fn edited() {}\n")
        edited = red_resolve.resolve(self.root, env=self.env)
        self.assertNotEqual(edited["selected"]["status"], "ready", "an edit makes the prebuilt stale: it must be rebuilt, never run")
        self.assertIsNone(edited["selected"]["exe"])

    def test_a_binary_built_here_wins_over_the_prebuilt(self):
        make_release(self.root, self.releases, "v1")
        self.ensure()
        built = write(os.path.join(self.target, "debug", "red_engine2"), "#!/bin/sh\n", 0o755)
        os.utime(built, (time.time() + 5, time.time() + 5))
        sel = red_resolve.resolve(self.root, env=self.env)["selected"]
        self.assertEqual(sel["exe"], built)


@NEEDS_LINUX
class Callers(Sandbox):
    """`scripts/dev red`, `scripts/dev start` and `scripts/bootstrap.sh`, run from the throwaway checkout with a fake `cargo` that records a compile."""

    def setUp(self):
        super().setUp()
        self.bin = os.path.join(self.tmp, "bin")
        self.compiled = os.path.join(self.tmp, "compiled.marker")
        write(os.path.join(self.bin, "cargo"), f'#!/bin/sh\necho "$@" >> "{self.compiled}"\nexit 1\n', 0o755)
        self.env["PATH"] = self.bin + os.pathsep + os.environ["PATH"]
        self.env["HOME"] = os.path.join(self.tmp, "home")   # scripts/dev sources ~/.cargo/env; the real one would put the real cargo first
        os.makedirs(self.env["HOME"])

    def run_dev(self, *args, **kw):
        return subprocess.run(["bash", os.path.join(self.root, "scripts", "dev"), *args], capture_output=True, text=True, cwd=self.root, env={**self.env, **kw.get("env", {})}, timeout=120)

    def test_describe_brief_on_a_fresh_checkout_runs_the_prebuilt_with_no_compile(self):
        make_release(self.root, self.releases, "v1")
        p = self.run_dev("red", "describe", "--brief")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("prebuilt v1 full red_engine2", p.stdout)
        self.assertIn("no compile", p.stderr)
        self.assertFalse(os.path.exists(self.compiled), "cargo was run")
        q = self.run_dev("red", "describe", "--brief")
        self.assertIn("prebuilt v1", q.stdout)
        self.assertFalse(os.path.exists(self.compiled))

    def test_without_a_matching_release_it_says_why_and_builds(self):
        p = self.run_dev("red", "describe", "--brief")
        self.assertNotEqual(p.returncode, 0, "the fake cargo fails: that is the build being attempted")
        self.assertIn("prebuilt: none (no release has been published yet)", p.stderr)
        self.assertIn("building from source instead", p.stderr)
        self.assertTrue(os.path.exists(self.compiled), "it fell back to cargo")

    def test_an_edited_checkout_is_built_not_served_a_stale_binary(self):
        make_release(self.root, self.releases, "v1")
        self.run_dev("red", "describe", "--brief")
        write(os.path.join(self.root, "src", "lib.rs"), "pub fn edited() {}\n")
        p = self.run_dev("red", "describe", "--brief")
        self.assertIn("uncommitted", p.stderr)
        self.assertTrue(os.path.exists(self.compiled))

    def test_start_fetches_names_the_executable_and_never_builds(self):
        make_release(self.root, self.releases, "v1")
        p = subprocess.run([sys.executable, os.path.join(self.root, "scripts", "launchpad.py"), "start", "fix a bug in src/lib.rs", "--no-save", "--json"],
                           capture_output=True, text=True, cwd=self.root, env=self.env, timeout=120)
        self.assertEqual(p.returncode, 0, p.stderr)
        out = json.loads(p.stdout)
        self.assertEqual(out["prebuilt"]["status"], "fetched")
        self.assertEqual(out["identity"]["executable"]["status"], "ready")
        self.assertFalse(os.path.exists(self.compiled))
        text = subprocess.run([sys.executable, os.path.join(self.root, "scripts", "launchpad.py"), "start", "fix a bug in src/lib.rs", "--no-save"], capture_output=True, text=True,
                              cwd=self.root, env=self.env, timeout=120).stdout
        self.assertRegex(text, r"executable READY: \S+/prebuilt/[0-9a-f]{12}/red_engine2")
        self.assertIn("builds triggered: 0", text)

    def test_an_older_scripts_directory_without_prebuilt_py_still_works(self):
        old = os.path.join(self.tmp, "old")
        for f in ("launchpad.py", "red_resolve.py"):
            os.makedirs(os.path.join(old, "scripts"), exist_ok=True)
            shutil.copy(os.path.join(HERE, f), os.path.join(old, "scripts", f))
        write(os.path.join(old, "Cargo.toml"), "[package]\nname='red_engine2'\n")
        write(os.path.join(old, "docs", "features.json"), json.dumps({"features": {}}))
        p = subprocess.run([sys.executable, os.path.join(old, "scripts", "launchpad.py"), "start", "fix a bug in src/lib.rs", "--no-save", "--json"], capture_output=True, text=True,
                           cwd=old, env=self.env, timeout=120)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertNotIn("prebuilt", json.loads(p.stdout), "nothing to look for, nothing said")

    def test_start_with_fetching_off_touches_no_network(self):
        make_release(self.root, self.releases, "v1")
        p = subprocess.run([sys.executable, os.path.join(self.root, "scripts", "launchpad.py"), "start", "fix a bug in src/lib.rs", "--no-save", "--no-fetch", "--json"],
                           capture_output=True, text=True, cwd=self.root, env=self.env, timeout=120)
        out = json.loads(p.stdout)
        self.assertEqual(out["prebuilt"]["status"], "none")
        self.assertIn("switched off", out["prebuilt"]["reason"])
        self.assertNotEqual(out["identity"]["executable"]["status"], "ready")
        self.assertFalse(os.path.isdir(os.path.join(self.target, "prebuilt")) and os.listdir(os.path.join(self.target, "prebuilt")))

    def test_bootstrap_in_a_checkout_installs_the_match_or_exits_3_with_the_way_out(self):
        script = os.path.join(self.root, "scripts", "bootstrap.sh")
        none = subprocess.run(["sh", script], capture_output=True, text=True, cwd=self.root, env=self.env, timeout=120)
        self.assertEqual(none.returncode, 3, none.stdout + none.stderr)
        self.assertIn("nothing was installed", none.stderr)
        self.assertIn("scripts/dev red describe --brief", none.stderr)
        make_release(self.root, self.releases, "v1")
        ok = subprocess.run(["sh", script], capture_output=True, text=True, cwd=self.root, env=self.env, timeout=120)
        self.assertEqual(ok.returncode, 0, ok.stdout + ok.stderr)
        self.assertIn("installed for this checkout", ok.stdout)
        self.assertTrue(os.path.isfile(os.path.join(self.target, "prebuilt", prebuilt.fingerprint(self.root)["fingerprint"][:12], "red_engine2")))

    def test_the_release_manifest_is_listed_in_the_checksum_file(self):
        m = make_release(self.root, self.releases, "v1")
        sums = prebuilt.parse_sums(open(os.path.join(self.releases, "v1", "SHA256SUMS")).read())
        self.assertIn(prebuilt.manifest_name(m["fingerprint"]), sums)
        self.assertIn("MANIFEST.json", sums)
        for n, h in sums.items():
            self.assertEqual(h, hashlib.sha256(open(os.path.join(self.releases, "v1", n), "rb").read()).hexdigest(), n)
        self.assertEqual({a["kind"] for a in m["assets"]}, {"full", "headless"})
        self.assertEqual(sorted(next(a for a in m["assets"] if a["kind"] == "headless")["binaries"]), ["red_engine2"])


if __name__ == "__main__":
    unittest.main()
