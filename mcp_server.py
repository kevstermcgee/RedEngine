#!/usr/bin/env python3
"""Launcher for the engine's MCP server, which is native: `red_engine2 mcp` (ADR 2026-10-06-a-native-mcp-server).

This file used to be a 500-line Python wrapper with 36 tools that shelled out to the CLI on every call. It now only finds the right executable and becomes it, so an
existing client configuration (`python mcp_server.py`) keeps working with no Python packages at all. Prefer pointing the client straight at `red_engine2 mcp`.

The executable is chosen by `scripts/red_resolve.py` when present (CARGO_TARGET_DIR, RED_PROFILE, a stale binary is never used), else the first of
<CARGO_TARGET_DIR or ./target>/{debug,release}/red_engine2 that exists. Build it with `scripts/dev red describe --brief`.
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))


def find_exe():
    sys.path.insert(0, os.path.join(ROOT, "scripts"))
    try:
        import red_resolve  # the shared rule, when this checkout has it
        return red_resolve.require(ROOT)
    except ImportError:
        pass
    exe = "red_engine2.exe" if os.name == "nt" else "red_engine2"
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
    for profile in ("debug", "release"):
        path = os.path.join(target, profile, exe)
        if os.path.isfile(path):
            return path
    raise RuntimeError(f"red_engine2 is not built under {target}. Build it: `scripts/dev red describe --brief` in {ROOT}")


def main():
    try:
        exe = find_exe()
    except RuntimeError as e:
        print(e, file=sys.stderr)
        return 1
    if os.name == "nt":  # no exec on Windows: run it as a child with the same stdio and exit code
        return subprocess.call([exe, "mcp", *sys.argv[1:]])
    os.execv(exe, [exe, "mcp", *sys.argv[1:]])


if __name__ == "__main__":
    sys.exit(main())
