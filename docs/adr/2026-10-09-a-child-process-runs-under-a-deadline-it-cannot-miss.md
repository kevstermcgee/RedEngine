# 2026-10-09. A child process runs under a deadline it cannot miss
Status: accepted
Summary: Agents and gates are supervised by a reader thread, a monotonic deadline and a process-group (Windows: taskkill /T) kill, so a silent, quiet or tree-spawning child stops on time and leaves nothing running

## Context
`idea_forge.stream_agent` ran the agent as `for line in p.stdout:` and looked at the clock after handling each line. Three consequences, all observed in the code rather than assumed:
a child that printed nothing blocked in the read for as long as it liked (`--timeout-min 150` meant "150 minutes after its last line"); `--quiet` did `continue` before the clock check, so a
quiet run had no deadline at all; and when the deadline did fire, `p.kill()` stopped only the direct child, so the cargo builds, test servers and shells the agent had started kept running in the
worktree, holding the output pipe too. `subprocess.run(..., timeout=)`, used for the engine commands and the improvement gates, has the same tail: after killing the child it waits for
the pipe to close, which a surviving grandchild prevents. The run also recorded only the exit code, so a timeout, a crash and a SIGKILL looked alike afterwards.

## Decision
`scripts/proc_supervisor.py` (standard library, no asyncio) is the one way this tool runs a child under a deadline:

- **A reader thread** takes the child's output and hands it over through a queue; the supervising loop wakes every `poll` (0.1 s) whether or not anything arrived. The deadline is `time.monotonic()`,
  checked every wake, so output behaviour (none, continuous, huge lines, binary) and quiet mode cannot matter. What arrives is written to the log file and flushed as it arrives.
- **The tree is the unit.** The child starts in its own process group (POSIX `start_new_session`) or process group (Windows). A stop is SIGTERM to the group, `term_grace` (5 s) for a child that
  cleans up, SIGKILL; on Windows `taskkill /PID n /T /F`. After a normal exit the group is swept as well, so a background process the agent started does not outlive the run (POSIX; a Windows
  descendant is reachable only while its parent is alive, which covers timeout and cancel but not a parent that exits first).
- **Five outcomes, one record**: `completed`, `failed` (non-zero exit), `timeout`, `cancelled`, `spawn_error`, with the return code, elapsed time, how the child was stopped, the seconds since it
  last printed and the tail of its output, returned to the caller and written next to the transcript as `agent_end.json`; the ledger row and `run.json` carry the status. Partial transcripts stay.
- **Races are decided in the child's favour**: if the child has already exited when the deadline is handled, that is a normal exit with its own status (`exited_at_deadline`), never a timeout.
- **Cancellation** is a `cancel` event or callable; Ctrl-C and SIGTERM (via `exit_on_sigterm`, so `systemctl stop`/`docker stop`/cron do not leave the agent running) stop the tree, report once and then
  re-raise, so the program still ends the way the signal meant.
- `run_capture` is `subprocess.run(capture_output=True, text=True, timeout=)` on the same footing (raises `TimeoutExpired`, keeps the last 4 MiB), used by `run_engine` and the improvement gates.
- **Documented tolerance**: a stopped child is back within `overrun_bound()` seconds of its deadline: `poll` + `term_grace` + `kill_wait` + `drain_grace` + 1, about 10 s with the defaults, and a child
  that cannot be killed is reported (`could not be stopped and was abandoned`) rather than waited on.

## Consequences
- Every agent execution respects its configured deadline within that bound, in quiet and loud mode, silent or chatty (`scripts/test_proc_supervisor.py`, run by `tests/idea_forge.rs`, on Linux and on
  the hosted Windows job). A run that timed out is still scored and its feedback still filed (ADR 2026-10-08), now labelled `timeout` in the ledger.
- The agent's stdin is the null device: an agent waiting for a terminal answer fails instead of hanging.
- `--timeout-min` takes fractions of a minute (tests use `0.03`).
- Not covered: a Windows descendant that outlives its parent (needs a job object; `taskkill` cannot find it), and a process that escapes its group on purpose (`setsid`).
