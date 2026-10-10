#!/usr/bin/env python3
"""Tests for scripts/proc_supervisor.py and the way scripts/idea_forge.py uses it (run by tests/idea_forge.rs, or directly: python3 scripts/test_proc_supervisor.py).

Every child is `sys.executable -c <code>`, so the same tests run on Linux and on Windows (the hosted CI job runs them on both). The failure being pinned: `stream_agent` only looked at the clock
when the child printed a line, never in quiet mode, and killed only the direct child, so a silent agent could run for hours past its timeout and leave its descendants behind.

Timeouts here are fractions of a second to a few seconds; the bound asserted on every stop is `proc_supervisor.overrun_bound(...)`, the documented tolerance, with the grace periods
shortened for the tests that need to wait one out.
"""
import io
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import idea_forge  # noqa: E402
import proc_supervisor as ps  # noqa: E402

WINDOWS = os.name == "nt"
PY = sys.executable
# Short graces so a test that has to wait one out does not take the production five seconds; the bound is computed from the same numbers.
FAST = dict(poll=0.05, term_grace=0.6, kill_wait=1.0, drain_grace=0.5)
BOUND = ps.overrun_bound(0.05, 0.6, 1.0, 0.5)


def child(code):
    return [PY, "-u", "-c", code]


def alive(pid):
    """Whether a process is running (a zombie, which a container without an init never reaps, is not running)."""
    if WINDOWS:
        out = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], capture_output=True, text=True).stdout
        return str(pid) in out
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().rsplit(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False
    except OSError:
        try:
            os.kill(pid, 0)
            return True
        except OSError:
            return False


def wait_dead(pid, seconds=5.0):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if not alive(pid):
            return True
        time.sleep(0.05)
    return not alive(pid)


def read_pid(path, seconds=5.0, context=""):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            with open(path) as f:
                text = f.read().strip()
            if text:
                return int(text)
        except (OSError, ValueError):
            pass
        time.sleep(0.02)
    raise AssertionError(f"no pid appeared in {path}; the tree printed: {context!r}")


# Time to give a tree of two fresh interpreters to start before a deadline or a cancel may fire: the first Windows run needed more than a second (the grandchild had not written its
# pid yet when it was killed), so the tests that need the grandchild alive wait for its pid file and start their clock from a longer allowance there.
START = 6.0 if WINDOWS else 1.0


# A parent that starts a grandchild (which writes its own pid to a file and sleeps), waits until that pid file exists, then does `then`.
def with_descendant(pidfile, then):
    return (
        "import os, subprocess, sys, time\n"
        f"g = subprocess.Popen([sys.executable, '-c', \"import os, time; open({pidfile!r}, 'w').write(str(os.getpid())); time.sleep(120)\"])\n"
        f"while not (os.path.exists({pidfile!r}) and os.path.getsize({pidfile!r})):\n    time.sleep(0.02)\n"
        "print('parent started', g.pid, flush=True)\n" + then
    )


class Sandbox(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="re2_supervise_")

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def path(self, name):
        return os.path.join(self.tmp, name)

    def run_it(self, code, timeout, **kw):
        t0 = time.monotonic()
        log = io.BytesIO()
        out = ps.supervise(child(code), timeout=timeout, log=log, **{**FAST, **kw})
        return out, log.getvalue().decode(), time.monotonic() - t0


class Deadline(Sandbox):
    def test_a_child_that_exits_before_the_deadline_is_completed_with_all_its_output(self):
        out, log, took = self.run_it("print('a'); print('b')", timeout=10)
        self.assertEqual((out["status"], out["returncode"], out["stopped_with"]), ("completed", 0, "none"))
        self.assertEqual(log.split(), ["a", "b"])
        self.assertLess(took, 5)
        self.assertEqual(out["lines"], 2)
        self.assertGreaterEqual(out["output_bytes"], 4)

    def test_a_nonzero_exit_is_failed_not_timeout_or_completed(self):
        out, _, _ = self.run_it("import sys; print('bad'); sys.exit(3)", timeout=10)
        self.assertEqual((out["status"], out["returncode"]), ("failed", 3))
        self.assertIn("status 3", out["reason"])

    def test_a_child_that_prints_continuously_still_stops_at_the_deadline(self):
        out, log, took = self.run_it("import time\nwhile True:\n    print('tick', flush=True)\n    time.sleep(0.02)", timeout=1.0)
        self.assertEqual(out["status"], "timeout")
        self.assertGreaterEqual(took, 1.0)
        self.assertLess(took, 1.0 + BOUND)
        self.assertGreater(log.count("tick"), 10, "what it printed until then is kept")

    def test_a_silent_child_stops_at_the_deadline_which_the_old_loop_never_noticed(self):
        out, log, took = self.run_it("import time; time.sleep(60)", timeout=1.0)
        self.assertEqual(out["status"], "timeout")
        self.assertIsNone(out["last_output_age_s"], "it never printed")
        self.assertLess(took, 1.0 + BOUND, f"stopped after {took:.1f}s")
        self.assertIn("still running after 1 s", out["reason"])

    def test_a_child_that_ignores_sigterm_is_killed_after_the_grace(self):
        code = "import signal, time\nsignal.signal(signal.SIGTERM, signal.SIG_IGN)\nprint('ready', flush=True)\nwhile True:\n    time.sleep(0.1)"
        out, log, took = self.run_it(code, timeout=0.8)
        self.assertEqual(out["status"], "timeout")
        self.assertLess(took, 0.8 + BOUND)
        if not WINDOWS:
            self.assertEqual(out["stopped_with"], "SIGKILL")
            self.assertEqual(out["returncode"], -9)
            self.assertGreaterEqual(took, 0.8 + FAST["term_grace"] - 0.1, "it was given the grace first")
        self.assertIn("ready", log)

    def test_a_deadline_longer_than_the_run_never_fires_and_none_means_no_limit(self):
        out, _, _ = self.run_it("import time; time.sleep(0.3)", timeout=60)
        self.assertEqual(out["status"], "completed")
        out, _, _ = self.run_it("import time; time.sleep(0.3)", timeout=None)
        self.assertEqual(out["status"], "completed")
        self.assertIsNone(out["timeout_s"])

    def test_the_deadline_is_monotonic_not_wall_clock(self):
        real = time.time
        time.time = lambda: real() - 86400  # a wall clock set back a day: nothing may depend on it
        try:
            out, _, took = self.run_it("import time; time.sleep(60)", timeout=0.6)
        finally:
            time.time = real
        self.assertEqual(out["status"], "timeout")
        self.assertLess(took, 0.6 + BOUND)


class ExitRaces(Sandbox):
    def test_a_child_that_exits_while_the_stop_is_being_decided_is_a_normal_exit(self):
        # The stop is decided (here: a cancel check that only answers once the child has finished and gone), and by the time the tree would be stopped the child has exited by itself.
        marker = self.path("finished")

        def cancel_once_it_is_gone():
            end = time.monotonic() + 20
            while not os.path.exists(marker) and time.monotonic() < end:
                time.sleep(0.01)
            time.sleep(0.4)  # the process is gone, not merely about to be
            return True

        out = ps.supervise(child(f"print('done', flush=True); open({marker!r}, 'w').close()"), timeout=30, cancel=cancel_once_it_is_gone, **FAST)
        self.assertEqual((out["status"], out["returncode"]), ("completed", 0), "an exit is not turned into a cancel or a timeout because the stop was being decided at the same moment")
        self.assertTrue(out["exited_at_deadline"])
        self.assertEqual(out["stopped_with"], "none", "nothing was signalled")

    def test_a_child_that_exits_during_a_slow_callback_at_the_deadline_is_completed(self):
        slept = []

        def slow_first_chunk(chunk):
            if not slept:
                slept.append(1)
                time.sleep(0.7)  # longer than the 0.3 s deadline

        out = ps.supervise(child("print('done', flush=True)"), timeout=0.3, on_chunk=slow_first_chunk, **FAST)
        self.assertEqual((out["status"], out["returncode"]), ("completed", 0))

    def test_a_child_that_dies_during_the_graceful_stop_is_still_a_timeout(self):
        code = "import signal, sys, time\nsignal.signal(signal.SIGTERM, lambda *a: sys.exit(7))\nprint('up', flush=True)\ntime.sleep(60)"
        out, log, took = self.run_it(code, timeout=0.6)
        self.assertEqual(out["status"], "timeout")
        if not WINDOWS:
            self.assertEqual((out["stopped_with"], out["returncode"]), ("SIGTERM", 7), "it handled SIGTERM itself: no SIGKILL was needed")
            self.assertLess(took, 0.6 + 1.0)

    def test_a_child_that_cannot_be_started_is_a_spawn_error_not_an_exception(self):
        out = ps.supervise([os.path.join(self.tmp, "no-such-program")], timeout=5, **FAST)
        self.assertEqual(out["status"], "spawn_error")
        self.assertIsNone(out["returncode"])
        self.assertIn("could not start", out["reason"])


class Descendants(Sandbox):
    def test_a_timeout_takes_the_whole_tree_not_just_the_child(self):
        pidfile = self.path("grandchild.pid")
        out, log, took = self.run_it(with_descendant(pidfile, "time.sleep(60)"), timeout=START)
        gpid = read_pid(pidfile, context=(out, log))
        self.assertEqual(out["status"], "timeout")
        self.assertTrue(wait_dead(gpid), f"the grandchild ({gpid}) was left running: the old kill() only reached the direct child")
        self.assertLess(took, START + BOUND)

    @unittest.skipIf(WINDOWS, "a Windows descendant is only reachable while its parent is alive (taskkill /T); the sweep after a normal exit is POSIX process-group behaviour")
    def test_a_child_that_exits_normally_but_leaves_a_background_process_does_not_leave_it_running(self):
        pidfile = self.path("background.pid")
        out, log, took = self.run_it(with_descendant(pidfile, "time.sleep(0.5)"), timeout=30)
        gpid = read_pid(pidfile)
        self.assertEqual(out["status"], "completed")
        self.assertTrue(out["swept"])
        self.assertTrue(wait_dead(gpid), "the background process outlived the run that started it")
        self.assertLess(took, 5, "a descendant holding the output pipe open must not make the supervisor wait for it")

    def test_cancelling_takes_the_whole_tree_too(self):
        pidfile = self.path("cancel.pid")
        cancel = threading.Event()
        threading.Timer(START, cancel.set).start()
        out, log, took = self.run_it(with_descendant(pidfile, "time.sleep(60)"), timeout=30, cancel=cancel)
        gpid = read_pid(pidfile, context=(out, log))
        self.assertEqual(out["status"], "cancelled")
        self.assertTrue(wait_dead(gpid))
        self.assertLess(took, START + BOUND)
        self.assertIn("parent started", log, "what it printed before the cancel is kept")


class Cancellation(Sandbox):
    def test_a_cancel_flag_stops_a_running_child_and_says_cancelled(self):
        cancel = threading.Event()
        threading.Timer(0.5, cancel.set).start()
        out, log, took = self.run_it("import time; print('working', flush=True); time.sleep(60)", timeout=30, cancel=cancel)
        self.assertEqual(out["status"], "cancelled")
        self.assertLess(took, 0.5 + BOUND)
        self.assertGreaterEqual(took, 0.4)
        self.assertIn("working", log)

    def test_a_cancel_callable_works_like_an_event(self):
        t0 = time.monotonic()
        out, _, took = self.run_it("import time; time.sleep(60)", timeout=30, cancel=lambda: time.monotonic() - t0 > 0.4)
        self.assertEqual(out["status"], "cancelled")

    def test_ctrl_c_stops_the_tree_reports_it_and_is_raised_again(self):
        import _thread
        pidfile = self.path("sigint.pid")
        reports = []
        threading.Timer(START, _thread.interrupt_main).start()
        with self.assertRaises(KeyboardInterrupt):
            ps.supervise(child(with_descendant(pidfile, "time.sleep(60)")), timeout=30, report=reports.append, **FAST)
        self.assertEqual([r["status"] for r in reports], ["cancelled"], "reported exactly once, on the way out")
        self.assertTrue(wait_dead(read_pid(pidfile)))
        self.assertTrue(wait_dead(reports[0]["pid"]))

    @unittest.skipIf(WINDOWS, "SIGTERM is not deliverable to a Windows process")
    def test_sigterm_during_a_run_stops_the_tree_then_exits_like_a_normal_termination(self):
        import signal
        pidfile = self.path("sigterm.pid")
        reports = []
        threading.Timer(1.0, lambda: os.kill(os.getpid(), signal.SIGTERM)).start()
        with self.assertRaises(SystemExit) as caught, ps.exit_on_sigterm():
            ps.supervise(child(with_descendant(pidfile, "time.sleep(60)")), timeout=30, report=reports.append, **FAST)
        self.assertEqual(caught.exception.code, 143)
        self.assertTrue(wait_dead(read_pid(pidfile)), "the agent's tree must not outlive a `systemctl stop`")
        self.assertEqual(reports[0]["status"], "cancelled")
        self.assertEqual(signal.getsignal(signal.SIGTERM), signal.SIG_DFL, "the previous handler is restored")


class Output(Sandbox):
    def test_partial_output_before_a_timeout_is_kept_in_the_log_and_the_diagnosis(self):
        out, log, _ = self.run_it("import time\nfor i in range(5):\n    print('partial-%d' % i, flush=True)\ntime.sleep(60)", timeout=1.0)
        self.assertEqual(out["status"], "timeout")
        self.assertEqual([l for l in log.split() if l.startswith("partial")], [f"partial-{i}" for i in range(5)])
        self.assertIn("partial-4", out["tail"])
        self.assertGreater(out["last_output_age_s"], 0.3, "the diagnosis says how long it had been silent")

    def test_a_very_long_line_and_binary_noise_do_not_break_the_supervisor(self):
        seen = []
        out = ps.supervise(child("import sys; sys.stdout.buffer.write(b'x' * 3000000 + b'\\xff\\xfe\\n' + b'ok\\n'); sys.stdout.buffer.flush()"), timeout=30, on_line=seen.append, **FAST)
        self.assertEqual(out["status"], "completed")
        self.assertEqual(out["output_bytes"], 3000000 + 2 + 1 + 3)
        self.assertTrue(seen[-1].startswith("ok"))

    def test_the_log_is_written_as_it_arrives_not_at_the_end(self):
        path = self.path("log.bin")
        seen = []

        def peek(chunk):
            seen.append(os.path.getsize(path))

        with open(path, "wb") as log:
            ps.supervise(child("import time\nprint('one', flush=True)\ntime.sleep(0.4)\nprint('two', flush=True)"), timeout=30, log=log, on_chunk=peek, **FAST)
        self.assertGreater(seen[0], 0, "the first line was already on disk when its callback ran")

    def test_a_watcher_that_raises_does_not_stop_the_run(self):
        def bad(line):
            raise RuntimeError("watcher bug")

        out, _, _ = self.run_it("print('a'); print('b')", timeout=10, on_line=bad)
        self.assertEqual(out["status"], "completed")

    def test_the_outcome_is_plain_json(self):
        out, _, _ = self.run_it("print('x')", timeout=10)
        self.assertEqual(json.loads(json.dumps(out))["status"], "completed")
        self.assertIn(out["status"], ps.STATUSES)


class NoLeaks(Sandbox):
    def test_repeated_failed_and_timed_out_runs_leave_no_threads_processes_or_descriptors(self):
        def fds():
            return len(os.listdir("/proc/self/fd")) if os.path.isdir("/proc/self/fd") else None

        before_threads, before_fds, pids = threading.active_count(), fds(), []
        for i in range(8):
            pidfile = self.path(f"leak{i}.pid")
            code = with_descendant(pidfile, "time.sleep(60)") if i % 2 == 0 else "import sys; print('x'); sys.exit(1)"
            out, _, _ = self.run_it(code, timeout=START if i % 2 == 0 else 0.6)
            self.assertIn(out["status"], ("timeout", "failed"))
            pids.append((out["pid"], pidfile if i % 2 == 0 else None))
        for pid, pidfile in pids:
            self.assertTrue(wait_dead(pid), f"child {pid} left behind")
            if pidfile:
                self.assertTrue(wait_dead(read_pid(pidfile)), "a grandchild left behind")
        deadline = time.monotonic() + 3
        while threading.active_count() > before_threads and time.monotonic() < deadline:
            time.sleep(0.05)
        self.assertLessEqual(threading.active_count(), before_threads, "a pump thread was left running")
        if before_fds is not None:
            self.assertLessEqual(fds(), before_fds + 1, "pipes were left open")


class RunCapture(Sandbox):
    def test_it_matches_subprocess_run_for_a_normal_command(self):
        p = ps.run_capture(child("import sys; print('out'); print('err', file=sys.stderr); sys.exit(2)"), timeout=10)
        self.assertEqual(p.returncode, 2)
        self.assertIn("out", p.stdout)
        self.assertIn("err", p.stdout, "stderr is part of the one stream")

    def test_a_timeout_raises_timeout_expired_with_what_was_printed_and_does_not_hang_on_a_grandchild(self):
        pidfile = self.path("capture.pid")
        t0 = time.monotonic()
        with self.assertRaises(subprocess.TimeoutExpired) as caught:
            ps.run_capture(child(with_descendant(pidfile, "time.sleep(60)")), timeout=START, **FAST)
        self.assertLess(time.monotonic() - t0, START + BOUND, "subprocess.run would have waited on the pipe the grandchild still holds")
        self.assertIn("parent started", caught.exception.output)
        self.assertTrue(wait_dead(read_pid(pidfile)))

    def test_a_missing_program_is_file_not_found_like_subprocess_run(self):
        with self.assertRaises(FileNotFoundError):
            ps.run_capture([os.path.join(self.tmp, "nope")], timeout=5)


class AgentRuns(Sandbox):
    """The same guarantees through `idea_forge.stream_agent`, the function the pipeline calls."""

    def agent(self, code, timeout, quiet, cancel=None):
        session = self.path("run/session.jsonl")
        os.makedirs(os.path.dirname(session), exist_ok=True)
        t0 = time.monotonic()
        # The module's own defaults for graces are production ones (5 s); the deadline is what is under test.
        outcome = idea_forge.stream_agent(child(code), self.tmp, dict(os.environ), session, timeout, quiet, cancel)
        return outcome, session, time.monotonic() - t0

    def test_a_silent_agent_is_stopped_at_its_timeout_in_quiet_mode_and_in_loud_mode(self):
        for quiet in (True, False):
            out, session, took = self.agent("import time; time.sleep(60)", timeout=1.0, quiet=quiet)
            self.assertEqual(out["status"], "timeout", f"quiet={quiet}")
            self.assertLess(took, 1.0 + ps.overrun_bound(), f"quiet={quiet}: {took:.1f}s")

    def test_the_end_record_and_the_partial_transcript_are_kept(self):
        code = "import json, time\nprint(json.dumps({'type': 'assistant', 'message': {'content': [{'type': 'tool_use', 'name': 'Bash', 'input': {'command': 'cargo test'}}]}}), flush=True)\ntime.sleep(60)"
        out, session, _ = self.agent(code, timeout=1.0, quiet=False)
        with open(session) as f:
            self.assertIn("tool_use", f.read(), "the transcript up to the stop is on disk")
        with open(os.path.join(os.path.dirname(session), "agent_end.json")) as f:
            record = json.load(f)
        self.assertEqual((record["status"], record["timeout_s"]), ("timeout", 1.0))
        self.assertIn("tool_use", record["tail"])
        self.assertEqual(record["pid"], out["pid"])

    def test_completion_failure_and_spawn_error_are_told_apart(self):
        ok, _, _ = self.agent("print('fine')", timeout=30, quiet=True)
        bad, _, _ = self.agent("import sys; sys.exit(4)", timeout=30, quiet=True)
        self.assertEqual((ok["status"], bad["status"], bad["returncode"]), ("completed", "failed", 4))
        session = self.path("run/session.jsonl")
        gone = idea_forge.stream_agent([os.path.join(self.tmp, "no-claude")], self.tmp, dict(os.environ), session, 30, True)
        self.assertEqual(gone["status"], "spawn_error")

    def test_the_cancel_flag_reaches_the_agent(self):
        cancel = threading.Event()
        threading.Timer(0.5, cancel.set).start()
        out, _, took = self.agent("import time; time.sleep(60)", timeout=60, quiet=True, cancel=cancel)
        self.assertEqual(out["status"], "cancelled")
        self.assertLess(took, 0.5 + ps.overrun_bound())


if __name__ == "__main__":
    unittest.main(verbosity=1)
