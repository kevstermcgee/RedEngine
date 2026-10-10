#!/usr/bin/env python3
"""Tests for scripts/red_resolve.py and scripts/launchpad.py (run by tests/launchpad.rs, or directly: python3 scripts/test_launchpad.py).

Every test builds a throwaway engine checkout and, where it matters, a game project. A fake `cargo` and a fake `red_engine2` leave marker files, so
"never compiled" and "never ran a stale binary" are checked, not assumed. Nothing here needs Rust, a GPU or the network.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile
import time
import unittest

# `start` looks for a prebuilt release when there is no executable (scripts/prebuilt.py); these tests must never reach the network, whatever git state a throwaway checkout is in.
os.environ["RED_NO_FETCH"] = "1"
HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
LAUNCHPAD = os.path.join(HERE, "launchpad.py")
RESOLVE = os.path.join(HERE, "red_resolve.py")
sys.path.insert(0, HERE)
import launchpad  # noqa: E402
import red_resolve  # noqa: E402

WINDOWS = os.name == "nt"
# Windows has no `#!/bin/sh`: a fake binary that must RUN (the read-only `propose`/`context` calls) cannot be written there. Resolution-only tests still run on Windows.
NEEDS_SH = unittest.skipIf(WINDOWS, "a fake executable here is a shell script")


def clean_path():
    """PATH without any directory that already holds a red_engine2: `cargo test` puts the repository's own target/debug on PATH (on Windows), and the resolver would
    find that real binary as an installed candidate."""
    exe = red_resolve.exe_name()
    return os.pathsep.join(p for p in os.environ.get("PATH", "").split(os.pathsep) if p and not os.path.isfile(os.path.join(p, exe)))

FEATURES = {"features": {
    "net_server": {"summary": "The authoritative UDP server: sessions, handshake, snapshots.", "files": ["src/net/server.rs", "src/net/session.rs"],
                   "tests": ["net_flow", "lib:net::server"], "commands": ["red_engine2 net-test examples/test_lab.json"], "docs": ["docs/adr/0016.md"], "depends_on": []},
    "ui_kit": {"summary": "The headless audited 2-D UI; draws the handshake state.", "files": ["src/ui/**"], "tests": ["lib:ui"], "commands": [], "docs": [], "depends_on": ["net_server"]}}}

PROPOSE_OK = {"schema": 1, "ok": True, "data": {"title": "Coin Dash", "genre": "arcade", "capabilities": {"presentation": "2d", "platforms": ["windows", "linux"], "networking": "offline"},
              "complexity": "small", "session_minutes": [2, 5], "reasons": ["2D is the simplest"], "problems": [], "warnings": [], "buildable": True}}
PROPOSE_NO = {"schema": 1, "ok": True, "data": {"title": "Big Online Thing", "capabilities": {"presentation": "3d", "platforms": ["windows", "linux"], "networking": "authoritative"},
              "problems": ["3D multiplayer on a 2D-only target is not supported"], "warnings": [], "buildable": False, "reasons": []}}


def write(path, text="x", mtime=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    if mtime is not None:
        os.utime(path, (mtime, mtime))
    return path


def make_engine(root, features=True):
    old = time.time() - 1000
    write(os.path.join(root, "Cargo.toml"), "[package]\nname='red_engine2'\n", old)
    write(os.path.join(root, "Cargo.lock"), "", old)
    write(os.path.join(root, "build.rs"), "fn main(){}", old)
    write(os.path.join(root, "src", "lib.rs"), "", old)
    write(os.path.join(root, "assets", "a.json"), "{}", old)
    write(os.path.join(root, "crates", "red2d", "src", "lib.rs"), "", old)
    if features:
        write(os.path.join(root, "docs", "features.json"), json.dumps(FEATURES), old)
    return root


def fake_exe(path, propose=None, marker=None, mtime=None):
    """An executable that answers the three read-only commands with canned JSON and records that it ran."""
    if WINDOWS and not path.endswith(".exe"):
        path += ".exe"
    body = "#!/bin/sh\n"
    if marker:
        body += f'echo "$@" >> "{marker}"\n'
    body += 'case "$1" in\n'
    body += f"  propose) cat <<'EOF'\n{json.dumps(propose or PROPOSE_OK)}\nEOF\n;;\n"
    body += '  capabilities|context) echo \'{"schema":1,"ok":true,"data":{"text":"ok"}}\';;\n'
    body += "esac\n"
    write(path, body)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    t = mtime if mtime is not None else time.time()
    os.utime(path, (t, t))
    return path


class Sandbox(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="re2_launchpad_")
        self.engine = make_engine(os.path.join(self.tmp, "engine"))
        self.cargo_marker = os.path.join(self.tmp, "cargo_ran")
        bindir = os.path.join(self.tmp, "bin")
        write(os.path.join(bindir, "cargo"), f'#!/bin/sh\necho "$@" >> "{self.cargo_marker}"\nexit 1\n')
        os.chmod(os.path.join(bindir, "cargo"), 0o755)
        self.env = {"PATH": bindir + os.pathsep + clean_path(), "HOME": self.tmp, "USERPROFILE": self.tmp, **{k: os.environ[k] for k in ("SYSTEMROOT", "TEMP", "TMP", "PATHEXT") if k in os.environ}, "RED_PREFIX": os.path.join(self.tmp, "prefix")}

    def lp(self, *args, cwd=None, env=None, engine=None):
        e = dict(self.env)
        e.update(env or {})
        script = os.path.join(engine or self.engine, "scripts", "launchpad.py")
        if not os.path.exists(script):
            os.makedirs(os.path.dirname(script), exist_ok=True)
            for f in ("launchpad.py", "red_resolve.py", "prebuilt.py"):
                with open(os.path.join(HERE, f), encoding="utf-8") as src, open(os.path.join(engine or self.engine, "scripts", f), "w", encoding="utf-8") as dst:
                    dst.write(src.read())
        p = subprocess.run([sys.executable, script, *args, "--json"] if "--json" not in args else [sys.executable, script, *args], capture_output=True, text=True, env=e, cwd=cwd or self.tmp)
        try:
            return json.loads(p.stdout), p
        except ValueError:
            self.fail(f"not JSON (exit {p.returncode}):\n{p.stdout}\n{p.stderr}")

    def resolve(self, project=None, env=None, engine=None):
        e = dict(self.env)
        e.update(env or {})
        return red_resolve.resolve(engine or self.engine, project, env=e)

    def project(self, name="proj", pin=None):
        d = os.path.join(self.tmp, name)
        write(os.path.join(d, "game.json"), json.dumps({"game": 1, "name": name, "engine": pin or {"path": "../engine"}}))
        write(os.path.join(d, "maps", "main.json"), "{}")
        return d

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)


class ExecutableSelection(Sandbox):
    def test_a_fresh_checkout_without_a_binary_is_diagnosed_without_compiling_or_running_anything(self):
        out, p = self.lp("start", "make a small 2d coin game for the browser", "--no-save")
        self.assertEqual((p.returncode, out["schema"], out["read_only"]), (0, "red-launchpad/1", True))
        self.assertEqual(out["identity"]["executable"]["status"], "missing")
        self.assertTrue(out["next_action"]["compiles"], "the build is the recommended next action, not something discovery did")
        self.assertFalse(os.path.exists(self.cargo_marker), "discovery must never invoke cargo")
        self.assertEqual(out["invoked"], [], "no engine command can run without a fresh executable")
        self.assertFalse(os.path.isdir(os.path.join(self.engine, "target")), "discovery creates no build directory")
        self.assertEqual(out["capabilities"]["checked"], False)
        self.assertTrue(any("unchecked" in u for u in out["uncertainty"]))
        detail = " ".join(p["detail"] for p in out["prerequisites"] if p["id"] == "engine-cli")
        self.assertIn("background", detail, "a missing binary says up front that the first build is slow")
        self.assertIn("scripts/dev seed", detail, "and names the warm-start route")

    def test_a_stale_binary_is_reported_and_never_selected_or_run(self):
        marker = os.path.join(self.tmp, "exe_ran")
        exe = fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"), marker=marker, mtime=time.time() - 500)
        write(os.path.join(self.engine, "src", "net", "server.rs"), "// edited after the build", time.time())
        r = self.resolve()
        self.assertEqual((r["selected"]["status"], r["selected"]["exe"], r["selected"]["stale_exe"]), ("stale", None, exe))
        self.assertIn("src/net/server.rs", r["candidates"][0]["stale_because"])
        out, _ = self.lp("start", "make a 2d game", "--no-save")
        self.assertEqual(out["identity"]["executable"]["status"], "stale")
        self.assertFalse(os.path.exists(marker), "a stale binary must not even be asked for its capabilities")
        self.assertEqual(out["invoked"], [])

    def test_an_edit_under_crates_makes_the_binary_stale_and_a_doc_edit_does_not(self):
        exe = fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"))
        self.assertEqual(self.resolve()["selected"]["status"], "ready")
        write(os.path.join(self.engine, "crates", "red2d", "README.md"), "docs are not build inputs", time.time() + 5)
        self.assertEqual(self.resolve()["selected"]["status"], "ready")
        write(os.path.join(self.engine, "crates", "red2d", "src", "lib.rs"), "// edited", time.time() + 10)
        r = self.resolve()
        self.assertEqual(r["selected"]["status"], "stale", r)
        self.assertIn("crates/red2d/src/lib.rs", r["candidates"][0]["stale_because"])
        self.assertTrue(os.path.isfile(exe))

    def test_the_configured_target_directory_and_profile_decide_and_other_candidates_are_listed_not_used(self):
        other = fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"))
        mine = fake_exe(os.path.join(self.tmp, "elsewhere", "debug", "red_engine2"))
        r = self.resolve(env={"CARGO_TARGET_DIR": os.path.join(self.tmp, "elsewhere")})
        self.assertEqual((r["selected"]["exe"], r["target_dir"]), (mine, os.path.join(self.tmp, "elsewhere")))
        self.assertNotIn(other, [c["path"] for c in r["candidates"]], "the checkout's own target/ is not consulted when CARGO_TARGET_DIR is set")
        # a release binary beside a missing debug one is listed, never silently preferred
        rel = fake_exe(os.path.join(self.tmp, "t2", "release", "red_engine2"))
        r = self.resolve(env={"CARGO_TARGET_DIR": os.path.join(self.tmp, "t2")})
        self.assertEqual(r["selected"]["status"], "missing")
        self.assertIn(rel, [c["path"] for c in r["candidates"] if "not selected" in c["source"]])
        r = self.resolve(env={"CARGO_TARGET_DIR": os.path.join(self.tmp, "t2"), "RED_PROFILE": "release"})
        self.assertEqual(r["selected"]["exe"], rel)

    def test_build_stamps_report_the_features_and_revision_a_binary_was_built_with(self):
        exe = fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"))
        d = os.path.dirname(exe)
        write(os.path.join(d, ".red-dev-red_engine2.mode"), "headless")
        write(os.path.join(d, ".red-build-red_engine2.json"), json.dumps({"rev": "abc123", "dirty_files": 2, "mode": "headless"}))
        c = self.resolve()["candidates"][0]
        self.assertEqual((c["features"], c["built_revision"], c["built_dirty_files"]), ("headless", "abc123", 2))
        os.remove(os.path.join(d, ".red-dev-red_engine2.mode"))
        os.remove(os.path.join(d, ".red-build-red_engine2.json"))
        self.assertEqual(self.resolve()["candidates"][0]["features"], "unknown", "a plain `cargo build` leaves no record: say unknown, not default")

    def test_an_explicit_override_is_used_as_given_but_reported_uncertain(self):
        exe = fake_exe(os.path.join(self.tmp, "mine", "red_engine2"))
        r = self.resolve(env={"RED_ENGINE_EXE": exe})
        self.assertEqual((r["selected"]["status"], r["selected"]["exe"]), ("uncertain", exe))
        self.assertEqual(self.resolve(env={"RED_ENGINE_EXE": os.path.join(self.tmp, "nope")})["selected"]["exe"], None)


class ProjectPins(Sandbox):
    def test_a_path_pin_selects_that_engines_binary(self):
        exe = fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"))
        r = self.resolve(project=self.project())
        self.assertEqual((r["engine"]["chosen_by"], r["selected"]["exe"]), ("project pin", exe))

    def test_red_engine_override_that_conflicts_with_the_pin_is_named(self):
        other = make_engine(os.path.join(self.tmp, "other_engine"))
        fake_exe(os.path.join(other, "target", "debug", "red_engine2"))
        r = self.resolve(project=self.project(), env={"RED_ENGINE": other})
        self.assertEqual(r["engine"]["root"], other)
        self.assertTrue(any("overrides the project's pinned engine" in n for n in r["notes"]), r["notes"])

    def test_a_git_pin_that_is_not_cloned_is_missing_and_discovery_does_not_clone_it(self):
        proj = self.project(pin={"git": "https://example.invalid/engine.git", "ref": "v1"})
        r = self.resolve(project=proj)
        self.assertEqual((r["selected"]["status"], r["engine"]["root"]), ("missing", None))
        self.assertTrue(any("red" in a for a in r["next_build"]["argv"]), r["next_build"])
        self.assertFalse(os.path.exists(os.path.join(proj, ".red")), "no .red/engine clone was made")
        out, p = self.lp("start", "make a game", "--project", proj, "--no-save", engine=self.engine)
        self.assertEqual(p.returncode, 0)
        self.assertFalse(os.path.exists(os.path.join(proj, ".red")))

    def test_a_checkout_that_is_not_at_the_pinned_ref_is_uncertain(self):
        subprocess.run(["git", "init", "-q", self.engine], check=True)
        env = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
        subprocess.run(["git", "-C", self.engine, "add", "-A"], check=True)
        subprocess.run(["git", "-C", self.engine, "commit", "-qm", "one"], check=True, env={**os.environ, **env})
        subprocess.run(["git", "-C", self.engine, "tag", "pinned"], check=True)
        subprocess.run(["git", "-C", self.engine, "commit", "-q", "--allow-empty", "-m", "two"], check=True, env={**os.environ, **env})
        fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"))
        proj = self.project(pin={"path": "../engine", "ref": "pinned"})
        r = self.resolve(project=proj)
        self.assertEqual(r["selected"]["status"], "uncertain")
        self.assertFalse(r["engine"]["matches_pin"])
        self.assertTrue(any("not the pinned revision" in n for n in r["notes"]))

    def test_a_prebuilt_install_serves_an_unpinned_checkout_only_as_uncertain_and_never_a_pinned_project(self):
        inst = fake_exe(os.path.join(self.tmp, "prefix", "bin", "red_engine2"))
        r = self.resolve()
        self.assertEqual((r["selected"]["status"], r["selected"]["exe"]), ("uncertain", inst))
        r = self.resolve(project=self.project())
        self.assertEqual((r["selected"]["status"], r["selected"]["exe"]), ("missing", None))
        self.assertTrue(any("pins an engine" in x for x in r["selected"]["reasons"]))

    def test_an_install_on_the_path_is_found_but_is_only_ever_uncertain(self):
        onpath = fake_exe(os.path.join(self.tmp, "pathbin", "red_engine2"))
        r = self.resolve(env={"PATH": os.path.dirname(onpath) + os.pathsep + self.env["PATH"]})
        self.assertEqual((r["selected"]["status"], r["selected"]["exe"]), ("uncertain", onpath))
        self.assertIn("installed (on PATH)", [c["source"] for c in r["candidates"]])
        r = self.resolve(project=self.project(), env={"PATH": os.path.dirname(onpath) + os.pathsep + self.env["PATH"]})
        self.assertEqual(r["selected"]["exe"], None, "a pinned project never silently uses whatever is on the PATH")

    def test_the_mcp_adapter_rule_raises_with_the_command_to_run(self):
        with self.assertRaises(RuntimeError) as cm:
            red_resolve.require(self.engine, env=self.env)
        self.assertIn("describe --brief", str(cm.exception))
        exe = fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"))
        self.assertEqual(red_resolve.require(self.engine, env=self.env), exe)


class Routing(Sandbox):
    def ready(self, propose=None):
        return fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"), propose=propose, marker=os.path.join(self.tmp, "exe_ran"))

    @NEEDS_SH
    def test_a_supported_small_game_gets_the_starter_command_and_checked_capabilities(self):
        exe = self.ready()
        out, _ = self.lp("start", "make a small 2d coin game for the browser", "--no-save")
        self.assertEqual((out["workflow"]["id"], out["capabilities"]["checked"], out["capabilities"]["supported"]), ("game-create", True, True))
        na = out["next_action"]
        self.assertEqual(na["argv"][:2], [exe, "new-game"])
        self.assertIn("--kind", na["argv"])
        self.assertEqual(na["argv"][na["argv"].index("--kind") + 1], "2d")
        self.assertEqual(na["cwd"], self.engine)
        self.assertEqual([i["argv"][1] for i in out["invoked"]], ["propose"], "only the read-only `propose` ran")
        self.assertEqual(out["next_action"]["then"]["argv"], ["scripts/red", "verify", "coin-browser.game2d.json"], "a 2D starter is verified with `verify`, never `check`")
        claims = {c["claim"] for c in out["final_requirements"]}
        self.assertTrue({"validation", "behavior", "visual/input inspection", "target execution (native window)", "networking"} <= claims, "separate claims, never merged")
        self.assertTrue(all(c["state"] in ("planned", "not_applicable") for c in out["evidence"]["claims"]), "nothing starts passed")

    @NEEDS_SH
    def test_an_unsupported_combination_is_a_blocker_with_an_extension_route_not_a_silently_smaller_game(self):
        self.ready(PROPOSE_NO)
        out, _ = self.lp("start", "make a 3d multiplayer shooter that runs in the browser", "--no-save")
        self.assertEqual((out["capabilities"]["supported"], out["workflow"]["id"]), (False, "game-create"))
        b = out["blockers"][0]
        self.assertEqual(b["id"], "unsupported-capability")
        self.assertIn("not supported", " ".join(b["detail"]))
        self.assertIn("engine-change", b["extension"]["argv"])
        self.assertEqual(out["next_action"]["kind"], "decide")
        self.assertNotIn("new-game", out["next_action"]["argv"], "no starter is offered for something the engine cannot do")
        self.assertIn("native", out["constraints"]["targets"])
        self.assertIn("multiplayer", out["constraints"]["targets"])

    def test_a_task_that_matches_nothing_is_unrouted_and_says_so(self):
        out, _ = self.lp("start", "hmm", "--no-save")
        self.assertEqual(out["workflow"]["id"], "unrouted")
        self.assertEqual(out["blockers"][0]["id"], "unrouted")
        self.assertIn("--workflow", out["next_action"]["argv"])
        out, _ = self.lp("start", "hmm", "--workflow", "diagnose", "--no-save")
        self.assertEqual(out["workflow"]["id"], "diagnose")

    @NEEDS_SH
    def test_a_focused_engine_change_names_the_owner_the_context_packet_and_the_ladder(self):
        exe = self.ready()
        out, _ = self.lp("start", "fix a bug in src/net/server.rs where the handshake drops a client", "--no-save")
        self.assertEqual(out["workflow"]["id"], "engine-change")
        owners = out["context"]["packet"]["owners"]
        self.assertEqual([o["feature"] for o in owners], ["net_server"], "a feature that merely mentions `handshake` is noise next to the owner of the named file")
        self.assertIn("net_flow", owners[0]["tests"])
        self.assertIn("reproduce_first", out["context"]["packet"], "a bug report asks for the failing test first")
        self.assertEqual(out["next_action"]["argv"], [exe, "context", "net_server"])
        ladder = [" ".join(c["argv"]) for c in out["iteration_checks"] + out["final_requirements"]]
        for args in (("iterate",), ("affected", "--quick"), ("affected",), ("affected", "--full"), ("preflight",)):
            self.assertIn(" ".join(launchpad.dev_argv(self.engine, *args)), ladder)
        self.assertEqual(out["invoked"], [], "engine-change asks the CLI nothing: the owners come from docs/features.json and the packet is the next action")

    def test_an_engine_change_without_a_binary_still_gets_owners_from_the_repository_and_says_what_the_cli_is_for(self):
        out, _ = self.lp("start", "refactor src/net/session.rs", "--no-save")
        self.assertEqual(out["context"]["packet"]["owners"][0]["feature"], "net_server")
        self.assertTrue(out["next_action"]["compiles"])
        self.assertTrue(any("engine CLI" in m for m in out["missing"]))
        self.assertIn("iterate", out["next_action"]["summary"])

    def test_an_existing_game_project_routes_to_game_change_and_its_own_wrapper(self):
        proj = self.project()
        out, _ = self.lp("start", "make the enemies faster", "--project", proj, "--no-save")
        self.assertEqual(out["workflow"]["id"], "game-change")
        self.assertEqual(out["next_action"]["cwd"], proj)
        self.assertEqual(out["next_action"]["argv"], ["scripts/red", "check"])
        d2 = self.project("proj2d")
        write(os.path.join(d2, "coins.game2d.json"), "{}")
        out, _ = self.lp("start", "make the coins worth more", "--project", d2, "--no-save")
        self.assertEqual(out["next_action"]["argv"], ["scripts/red", "verify", "coins.game2d.json"], "`check` is the walk-project check and fails on a 2D starter")
        out, _ = self.lp("start", "upgrade the engine pin", "--project", proj, "--no-save")
        self.assertEqual(out["workflow"]["id"], "upgrade")
        self.assertEqual(out["next_action"]["argv"], ["scripts/red", "game", "upgrade", "plan"])


class Resume(Sandbox):
    def setUp(self):
        super().setUp()
        self.proj = self.project()
        self.exe = fake_exe(os.path.join(self.engine, "target", "debug", "red_engine2"))
        write(os.path.join(self.proj, "out", "cache", "check-abc"), "ok", time.time())
        out, _ = self.lp("start", "make the enemies faster", "--project", self.proj)
        self.task = out["task"]["id"]

    def resume(self, *extra):
        return self.lp("resume", "--project", self.proj, *extra)[0]

    def test_resume_with_nothing_changed_trusts_nothing_it_cannot_check(self):
        out = self.resume()
        self.assertEqual(out["since_start"]["changes"], [])
        rec = out["evidence"]["observed"][0]
        self.assertEqual(rec["state"], "unverified", "a recorded cache entry is not a pass: only `scripts/red check` says whether it applies")
        self.assertEqual(rec["authority"], ["scripts/red", "check"])
        self.assertTrue(all(c["state"] != "passed" for c in out["evidence"]["claims"]))

    def test_changed_game_inputs_executable_and_verification_config_are_each_named_and_invalidate_records(self):
        write(os.path.join(self.proj, "maps", "main.json"), '{"edited": true}', time.time() + 5)
        out = self.resume()
        ch = {c["what"]: c for c in out["since_start"]["changes"]}
        self.assertIn("source or game inputs", ch)
        self.assertEqual(ch["source or game inputs"]["files"], ["maps/main.json"])
        self.assertTrue(all(o["state"] == "stale" for o in out["evidence"]["observed"]))
        self.assertFalse(out["since_start"]["recorded_results_trusted"])
        os.utime(self.exe, (time.time() + 20, time.time() + 20))
        write(os.path.join(self.engine, "docs", "features.json"), json.dumps({"features": {}}), time.time() + 30)
        out = self.resume()
        names = {c["what"] for c in out["since_start"]["changes"]}
        self.assertTrue({"executable identity", "verification configuration"} <= names, names)

    def test_a_rebuilt_binary_that_is_now_stale_after_an_edit_is_reported_as_an_executable_change(self):
        write(os.path.join(self.engine, "src", "lib.rs"), "// edit", time.time() + 50)
        out = self.resume()
        ex = [c for c in out["since_start"]["changes"] if c["what"] == "executable identity"][0]
        self.assertEqual((ex["then"]["status"], ex["now"]["status"]), ("ready", "stale"))

    def test_agent_notes_are_kept_apart_from_observed_results(self):
        out = self.resume("--note", "tried faster spawn; looks right")
        self.assertEqual(out["evidence"]["agent_notes"][0]["by"], "agent")
        self.assertTrue(all("tried faster" not in json.dumps(o) for o in out["evidence"]["observed"]))
        again = self.resume()
        self.assertEqual([n["text"] for n in again["evidence"]["agent_notes"]], ["tried faster spawn; looks right"], "notes persist across resumes")

    def test_a_recorded_green_for_the_engine_is_unverified_never_passed_and_an_edit_after_it_is_named(self):
        stamp = write(os.path.join(self.engine, "out", ".affected-green.json"), '{"green": ["abc"]}', time.time() - 100)
        out, _ = self.lp("start", "fix a bug in src/net/server.rs", "--workflow", "engine-change")
        rec = [o for o in out["evidence"]["observed"] if o["record"] == "out/.affected-green.json"]
        self.assertEqual(rec[0]["state"], "unverified")
        self.assertEqual(rec[0]["authority"], ["scripts/dev", "affected"])
        self.assertTrue(os.path.isfile(stamp))
        self.assertTrue(all(f["required"] for f in out["final_requirements"] if f["claim"].startswith("affected")))

    def test_resume_without_a_saved_task_says_how_to_start_one(self):
        d = os.path.join(self.tmp, "empty")
        os.makedirs(d)
        p = subprocess.run([sys.executable, LAUNCHPAD, "resume", "--project", d], capture_output=True, text=True, env=self.env)
        self.assertEqual(p.returncode, 2)
        self.assertIn("scripts/dev start", p.stderr)


class DevWrapper(unittest.TestCase):
    """`scripts/dev` must agree with the resolver: an edit under crates/ rebuilds, a doc edit does not, and the launchpad is reachable."""

    def setUp(self):
        if os.name == "nt" or not os.path.exists("/bin/bash"):
            self.skipTest("bash only")
        self.tmp = tempfile.mkdtemp(prefix="re2_devwrap_")
        self.root = make_engine(os.path.join(self.tmp, "engine"))
        for f in ("dev", "launchpad.py", "red_resolve.py", "prebuilt.py"):
            with open(os.path.join(HERE, f), encoding="utf-8") as s, open(os.path.join(self.root, "scripts", f) if os.path.isdir(os.path.join(self.root, "scripts")) else write(os.path.join(self.root, "scripts", f), ""), "w", encoding="utf-8") as d:
                d.write(s.read())
        os.chmod(os.path.join(self.root, "scripts", "dev"), 0o755)
        self.count = os.path.join(self.tmp, "cargo_count")
        bindir = os.path.join(self.tmp, "bin")
        # A fake cargo that "builds" red_engine2 into $CARGO_TARGET_DIR/debug and counts its runs.
        write(os.path.join(bindir, "cargo"), f'#!/bin/sh\necho x >> "{self.count}"\nmkdir -p "$CARGO_TARGET_DIR/debug"\nprintf \'#!/bin/sh\\necho built\\n\' > "$CARGO_TARGET_DIR/debug/red_engine2"\nchmod +x "$CARGO_TARGET_DIR/debug/red_engine2"\n')
        os.chmod(os.path.join(bindir, "cargo"), 0o755)
        self.env = {**os.environ, "PATH": bindir + os.pathsep + os.environ["PATH"], "CARGO_TARGET_DIR": os.path.join(self.tmp, "t"), "HOME": self.tmp}

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def builds(self):
        return len(open(self.count).read().split()) if os.path.exists(self.count) else 0

    def dev(self, *args):
        return subprocess.run(["bash", os.path.join(self.root, "scripts", "dev"), *args], capture_output=True, text=True, env=self.env, cwd=self.root)

    def test_edits_under_crates_rebuild_and_doc_edits_do_not(self):
        self.assertEqual(self.dev("red", "describe").returncode, 0)
        self.assertEqual(self.builds(), 1)
        self.dev("red", "describe")
        self.assertEqual(self.builds(), 1, "nothing changed: no second build")
        time.sleep(0.05)
        write(os.path.join(self.root, "crates", "red2d", "README.md"), "docs", time.time() + 2)
        self.dev("red", "describe")
        self.assertEqual(self.builds(), 1, "a doc under crates/ is not a build input")
        write(os.path.join(self.root, "crates", "red2d", "src", "lib.rs"), "// edit", time.time() + 4)
        self.dev("red", "describe")
        self.assertEqual(self.builds(), 2, "an edit under crates/ used to leave a stale binary in use")

    def test_start_through_the_wrapper_never_builds(self):
        p = self.dev("start", "make a small 2d game", "--no-save", "--json")
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(json.loads(p.stdout)["schema"], "red-launchpad/1")
        self.assertEqual(self.builds(), 0)

    def test_a_headless_build_is_recorded_so_the_cli_is_not_mistaken_for_the_full_one(self):
        self.dev("build", "--headless")
        d = os.path.join(self.tmp, "t", "debug")
        self.assertEqual(open(os.path.join(d, ".red-dev-red_engine2.mode")).read(), "headless")
        rec = json.load(open(os.path.join(d, ".red-build-red_engine2.json")))
        self.assertEqual((rec["mode"], rec["profile"]), ("headless", "debug"))


if __name__ == "__main__":
    unittest.main(verbosity=1)
