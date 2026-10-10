#!/usr/bin/env python3
"""Run a child process under a deadline that does not depend on what the child does (ADR 2026-10-09-a-child-process-runs-under-a-deadline-it-cannot-miss).

    outcome = supervise(argv, cwd=..., env=..., timeout=seconds, log=open("session.jsonl", "wb"), on_line=print_a_tool_call, cancel=threading.Event())

Standard library only, Linux and Windows, no asyncio. The old way (`for line in p.stdout:` with a clock check inside the loop) never looked at the clock while the child was silent, and
skipped it entirely in quiet mode; a timeout then killed only the direct child, so its descendants ran on. Here:

* the child's output is read by a thread and handed over through a queue, so the supervising loop wakes every `poll` seconds whether or not anything was printed;
* the deadline is `time.monotonic()`, so a clock change cannot stretch or shrink it;
* the child gets its own process group (POSIX `start_new_session`) or process group + `taskkill /T` (Windows), and a stop takes the whole tree: SIGTERM, `term_grace` seconds, SIGKILL;
* after a normal exit the group is swept too (POSIX), so a build server the agent started in the background does not outlive the run;
* output is written to `log` as it arrives, so a run stopped at its deadline keeps everything it printed;
* the result says which of five things happened: `completed`, `failed` (non-zero exit), `timeout`, `cancelled`, `spawn_error`.

**Tolerance.** `supervise` returns at most `overrun_bound(...)` seconds after the deadline: one `poll` to notice, `term_grace` for a child that ignores SIGTERM, then a short
wait after SIGKILL (a child that cannot be killed, uninterruptible in the kernel, is reported and abandoned, never waited on forever). The defaults make that about 10 seconds.
"""
import contextlib
import json
import os
import queue
import select
import signal
import subprocess
import sys
import threading
import time

IS_WINDOWS = os.name == "nt"
POLL_S = 0.1
TERM_GRACE_S = 5.0
KILL_WAIT_S = 2.0
DRAIN_GRACE_S = 2.0
MAX_LINE_BYTES = 8 * 1024 * 1024
TAIL_BYTES = 4000
STATUSES = ("completed", "failed", "timeout", "cancelled", "spawn_error")


def overrun_bound(poll=POLL_S, term_grace=TERM_GRACE_S, kill_wait=KILL_WAIT_S, drain_grace=DRAIN_GRACE_S):
    """The most a stopped child can run past its deadline before `supervise` returns, in seconds (the documented tolerance): one `poll` to notice, `term_grace` and `kill_wait` to stop
    it, `drain_grace` to take its last output, plus a second for a loaded machine."""
    return poll + term_grace + kill_wait + drain_grace + 1.0


class Outcome(dict):
    """What happened to a supervised child, as a plain dict (it is written to JSON as it is).

    status          completed | failed | timeout | cancelled | spawn_error
    returncode      the exit code (negative: killed by that POSIX signal), or None when the child could not be started or could not be reaped
    reason          one sentence
    elapsed_s       seconds from start to return
    timeout_s       the deadline asked for (None: none)
    pid, output_bytes, lines, last_output_age_s (None: never printed), tail (the last output, text)
    stopped_with    how a still-running child was stopped: none | SIGTERM | SIGKILL | taskkill
    exited_at_deadline   the child had already exited when the deadline was handled (the race is a normal exit, not a timeout)
    swept           the process group was killed after the child exited so nothing it started is left running
    output_truncated     the pipe was still held open by something this could not reach (Windows descendants), so some output may be missing
    """


class _Pump:
    """Reads a child's output on a thread so the supervisor never blocks on it: chunks arrive through a queue, `eof` is set when every writer has closed the pipe."""

    def __init__(self, stream):
        self.q = queue.Queue()
        self.eof = threading.Event()
        self._halt = threading.Event()
        self.thread = threading.Thread(target=self._run, args=(stream,), name="supervise-pump", daemon=True)
        self.thread.start()

    def _run(self, stream):
        try:
            fd = stream.fileno()
            while not self._halt.is_set():
                if not IS_WINDOWS and not select.select([fd], [], [], 0.2)[0]:
                    continue  # nothing yet: look at `halt` again (a Windows pipe cannot be select()ed; there the read blocks until a writer or the end)
                data = os.read(fd, 65536)
                if not data:
                    break
                self.q.put(data)
        except (OSError, ValueError):
            pass
        finally:
            self.eof.set()

    def get(self, wait):
        """The chunks that arrived, waiting up to `wait` seconds for the first (sleeping when the pipe is already closed, so the caller's loop never spins)."""
        chunks = []
        try:
            if self.eof.is_set() and self.q.empty():
                time.sleep(wait)
                return chunks
            chunks.append(self.q.get(timeout=wait))
            while True:
                chunks.append(self.q.get_nowait())
        except queue.Empty:
            pass
        return chunks

    def halt(self, stream):
        """Stop reading and release the pipe. A reader still blocked (Windows, a descendant holding the pipe) is abandoned: it is a daemon thread and costs nothing."""
        self._halt.set()
        self.thread.join(timeout=1.0)
        if not self.thread.is_alive():
            with contextlib.suppress(OSError, ValueError):
                stream.close()


class _Lines:
    """Cuts a byte stream into text lines for a callback; a line longer than MAX_LINE_BYTES is passed on in pieces rather than held in memory."""

    def __init__(self, callback):
        self.callback = callback
        self.buf = b""

    def feed(self, chunk):
        if not self.callback:
            return
        self.buf += chunk
        *whole, self.buf = self.buf.split(b"\n")
        if len(self.buf) > MAX_LINE_BYTES:
            whole.append(self.buf)
            self.buf = b""
        for raw in whole:
            self._emit(raw)

    def flush(self):
        if self.buf:
            self._emit(self.buf)
            self.buf = b""

    def _emit(self, raw):
        if self.callback is None:
            return
        try:
            self.callback(raw.decode("utf-8", errors="replace") + "\n")
        except Exception as e:  # a watcher that fails must never take the supervision down with it
            print(f"supervise: the line callback failed once and is ignored ({e})", file=sys.stderr)
            self.callback = None


def _popen_kwargs():
    if IS_WINDOWS:
        return {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}
    return {"start_new_session": True}


def _signal_group(pid, sig):
    """Sends `sig` to every process of the child's group (POSIX); True when the group still existed."""
    try:
        os.killpg(pid, sig)
        return True
    except (ProcessLookupError, PermissionError):
        return False


def _taskkill(pid):
    with contextlib.suppress(OSError, subprocess.SubprocessError):
        subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True, timeout=15)


def _stop_tree(p, term_grace, kill_wait, out):
    """Stops the child and everything it started; returns whether it is reaped. POSIX: SIGTERM to the group, SIGKILL after `term_grace`, and SIGKILL to the group again once the
    leader is gone (so a descendant that was started in the background cannot stay behind). Windows: `taskkill /T /F` while the leader is alive."""
    if IS_WINDOWS:
        out["stopped_with"] = "taskkill"
        _taskkill(p.pid)
        with contextlib.suppress(subprocess.TimeoutExpired):
            p.wait(timeout=kill_wait)
        return p.poll() is not None
    out["stopped_with"] = "SIGTERM"
    _signal_group(p.pid, signal.SIGTERM)
    try:
        p.wait(timeout=term_grace)
    except subprocess.TimeoutExpired:
        out["stopped_with"] = "SIGKILL"
        _signal_group(p.pid, signal.SIGKILL)
        with contextlib.suppress(subprocess.TimeoutExpired):
            p.wait(timeout=kill_wait)
    if _signal_group(p.pid, signal.SIGKILL):  # the leader is gone, anything of its group that is left is not wanted
        out["swept"] = True
    return p.poll() is not None


def _cancelled(cancel):
    if cancel is None:
        return False
    return bool(cancel()) if callable(cancel) else cancel.is_set()


def supervise(argv, cwd=None, env=None, *, timeout=None, log=None, on_chunk=None, on_line=None, cancel=None, report=None,
              poll=POLL_S, term_grace=TERM_GRACE_S, kill_wait=KILL_WAIT_S, drain_grace=DRAIN_GRACE_S):
    """Runs `argv` and returns an `Outcome`. See the module docs for the guarantees.

    timeout  seconds (monotonic) the child may run, or None for no limit
    log      a binary file object every chunk of output is written to and flushed as it arrives
    on_chunk callable(bytes) for each chunk; on_line callable(str) for each line (exceptions in either are contained)
    cancel   a threading.Event or a callable returning true: when set the child's tree is stopped and the status is `cancelled`
    report   callable(Outcome) called exactly once on every path, including when a KeyboardInterrupt or SystemExit (a SIGTERM handler) ends the wait: the exception is re-raised after the
             child's tree has been stopped and the report made
    A child that cannot be started is a `spawn_error` outcome, not an exception."""
    started = time.monotonic()
    out = Outcome(status=None, returncode=None, reason="", elapsed_s=0.0, timeout_s=timeout, pid=None, output_bytes=0, lines=0, last_output_age_s=None, tail="",
                  stopped_with="none", exited_at_deadline=False, swept=False, output_truncated=False)

    reported = []

    def finish(status, reason):
        if reported:
            return out
        reported.append(True)
        out["status"], out["reason"] = status, reason
        out["elapsed_s"] = round(time.monotonic() - started, 3)
        if report:
            with contextlib.suppress(Exception):
                report(out)
        return out

    try:
        p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **_popen_kwargs())
    except OSError as e:
        return finish("spawn_error", f"could not start {argv[0] if argv else '(nothing)'}: {e}")
    out["pid"] = p.pid
    pump, lines, tail, last_output = _Pump(p.stdout), _Lines(on_line), bytearray(), [None]

    def consume(chunks):
        for chunk in chunks:
            out["output_bytes"] += len(chunk)
            out["lines"] += chunk.count(b"\n")
            last_output[0] = time.monotonic()
            tail.extend(chunk)
            del tail[:-TAIL_BYTES]
            if log is not None:
                log.write(chunk)
                log.flush()
            if on_chunk:
                try:
                    on_chunk(chunk)
                except Exception as e:
                    print(f"supervise: the chunk callback failed once and is ignored ({e})", file=sys.stderr)
            lines.feed(chunk)

    def settle(stop_reason=None):
        """After the child is gone or stopped: take what is still in the pipe, sweep the group, release the pipe, fill in the diagnostics."""
        end = time.monotonic() + drain_grace
        while not (pump.eof.is_set() and pump.q.empty()) and time.monotonic() < end:
            consume(pump.get(min(poll, max(0.0, end - time.monotonic()))))
        if not IS_WINDOWS and p.poll() is not None and _signal_group(p.pid, signal.SIGKILL):
            out["swept"] = True  # something of the child's group was still running (it holds the pipe open too): gone now
            for _ in range(20):
                if pump.eof.is_set():
                    break
                time.sleep(0.05)
        consume(pump.get(0.0))
        if not pump.eof.is_set():
            out["output_truncated"] = True
        pump.halt(p.stdout)
        consume(pump.get(0.0))
        lines.flush()
        out["returncode"] = p.poll()
        out["tail"] = tail.decode("utf-8", errors="replace")
        out["last_output_age_s"] = None if last_output[0] is None else round(time.monotonic() - last_output[0], 3)

    deadline = None if timeout is None else started + timeout
    try:
        stopping = None
        while True:
            now = time.monotonic()
            if _cancelled(cancel):
                stopping = "cancelled"
                break
            if deadline is not None and now >= deadline:
                stopping = "timeout"
                break
            wait = poll if deadline is None else max(0.0, min(poll, deadline - now))
            consume(pump.get(wait))
            if p.poll() is not None:
                break
        if stopping and p.poll() is not None:
            # It exited by itself in the very moment the deadline (or a cancel) was being handled: that is a normal exit, whatever its status was.
            out["exited_at_deadline"] = True
            stopping = None
        if stopping:
            if not _stop_tree(p, term_grace, kill_wait, out):
                settle()
                return finish(stopping, f"the process could not be stopped and was abandoned (pid {p.pid})")
            settle()
            if stopping == "timeout":
                return finish("timeout", f"still running after {timeout:g} s; stopped with {out['stopped_with']} (the whole process tree)")
            return finish("cancelled", f"cancelled; stopped with {out['stopped_with']} (the whole process tree)")
        settle()
        code = out["returncode"]
        if code == 0:
            return finish("completed", "exited with status 0")
        return finish("failed", f"exited with status {code}")
    except BaseException as e:  # KeyboardInterrupt, a SIGTERM handler's SystemExit, anything: never leave the tree running
        _stop_tree(p, term_grace, kill_wait, out)
        with contextlib.suppress(BaseException):
            settle()
        finish("cancelled", f"interrupted ({type(e).__name__}); stopped with {out['stopped_with']} (the whole process tree)")
        raise


@contextlib.contextmanager
def exit_on_sigterm():
    """While active (main thread only), SIGTERM (`systemctl stop`, `docker stop`, a cron kill) becomes SystemExit(143) so a supervised child's tree is stopped before the program ends,
    instead of being left running when the default handler terminates the program on the spot."""
    if threading.current_thread() is not threading.main_thread() or not hasattr(signal, "SIGTERM"):
        yield
        return

    def handler(signum, frame):
        raise SystemExit(128 + signum)

    previous = signal.signal(signal.SIGTERM, handler)
    try:
        yield
    finally:
        signal.signal(signal.SIGTERM, previous)


def run_capture(argv, cwd=None, env=None, timeout=None, **options):
    """`subprocess.run(argv, capture_output=True, text=True, timeout=timeout)` that cannot outlive its deadline: stdout and stderr are one stream (`.stdout`, `.stderr` is empty), the
    last 4 MiB are kept, and a timeout stops the whole tree and raises `subprocess.TimeoutExpired` as `subprocess.run` does (which would instead wait on pipes a grandchild holds)."""
    keep = bytearray()

    def take(chunk):
        keep.extend(chunk)
        del keep[:-4 * 1024 * 1024]

    outcome = supervise(argv, cwd=cwd, env=env, timeout=timeout, on_chunk=take, **options)
    text = keep.decode("utf-8", errors="replace")
    if outcome["status"] == "spawn_error":
        raise FileNotFoundError(outcome["reason"])
    if outcome["status"] == "timeout":
        raise subprocess.TimeoutExpired(argv, timeout, output=text)
    return subprocess.CompletedProcess(argv, outcome["returncode"], stdout=text, stderr="")


def main(argv=None):
    """`python3 scripts/proc_supervisor.py --timeout SECONDS -- command ...`: runs a command under a deadline and prints the outcome as JSON on stderr (exit 124 on timeout, the child's code otherwise)."""
    args = list(sys.argv[1:] if argv is None else argv)
    if "--timeout" not in args or "--" not in args:
        print(main.__doc__, file=sys.stderr)
        return 2
    timeout = float(args[args.index("--timeout") + 1])
    command = args[args.index("--") + 1:]
    def echo(chunk):
        sys.stdout.buffer.write(chunk)
        sys.stdout.buffer.flush()

    outcome = supervise(command, timeout=timeout, on_chunk=echo)
    print(json.dumps(outcome, indent=1), file=sys.stderr)
    return 124 if outcome["status"] == "timeout" else (outcome["returncode"] or 0) if outcome["status"] in ("completed", "failed") else 1


if __name__ == "__main__":
    sys.exit(main())
