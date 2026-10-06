#!/usr/bin/env python3
"""A stdio MCP client that runs a scripted task against any MCP server and counts what an agent would pay for it.

    python3 scripts/mcp_bench.py --server "<command that starts the server>" --task old|new [--json]

Standard library only (newline-delimited JSON-RPC 2.0 over stdio, protocol 2024-11-05). It records, per call and in total:
  * bytes in `tools/list` (what an agent loads into its context at session start),
  * bytes the agent must SEND (the arguments of every tool call: a scene passed as text counts in full),
  * bytes the agent must READ as text, and the images it receives (counted apart: an image is priced by pixels, not by its base64 bytes),
  * wall time per call and in total, and the number of calls.
No model is involved: this prices the tool surface, not an agent's skill. The same abstract task is spelled for each server by `TASKS` below.
"""
import argparse
import json
import os
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class Client:
    def __init__(self, cmd, cwd=None, env=None):
        self.p = subprocess.Popen(cmd, shell=True, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, cwd=cwd, env=env, bufsize=1)
        self.n = 0

    def rpc(self, method, params=None, notify=False):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        if not notify:
            self.n += 1
            msg["id"] = self.n
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()
        if notify:
            return None
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("server closed: " + self.p.stderr.read()[-500:])
            try:
                r = json.loads(line)
            except ValueError:
                continue
            if r.get("id") == self.n:
                return r

    def start(self):
        r = self.rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "mcp_bench", "version": "1"}})
        self.rpc("notifications/initialized", notify=True)
        return r

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


def result_size(r):
    """What an agent reads from a tools/call result: (text bytes, image count, image base64 bytes, is_error). Images are priced by pixels, not bytes, so they
    are counted apart from text and never added into one 'bytes read' number."""
    res = r.get("result", {})
    text = images = image_bytes = 0
    for c in res.get("content", []):
        if c.get("type") == "text":
            text += len(c.get("text", "").encode())
        elif c.get("type") == "image":
            images += 1
            image_bytes += len(c.get("data", ""))
    if res.get("structuredContent"):
        text += len(json.dumps(res["structuredContent"], separators=(",", ":")))
    return text, images, image_bytes, bool(res.get("isError")) or "error" in r


def edited(scene_text):
    """The agent's one edit, done outside MCP with its own file tools: move the first lamp a little."""
    d = json.loads(scene_text)
    for o in d["objects"]:
        if str(o.get("id", "")).startswith("lamp"):
            o["position"] = [round(o["position"][0] + 0.2, 3), o["position"][1], o["position"][2]]
            break
    return json.dumps(d, indent=1)


def build_task(kind, scene_path, workdir):
    """The same agent-style task spelled for each server (nine tool calls; the edit between calls is a file write, not a call):
    orient, find a recipe, validate, lint, edit, validate, lint, verify, look at a plan, run the sim."""
    text = open(scene_path).read()
    text2 = edited(text)
    if kind == "old":  # the Python server: every call carries the whole scene as an argument
        return [("describe_brief", {}), ("search_engine", {"query": "two rooms joined by a door", "limit": 3}),
                ("validate_scene", {"scene_json": text}), ("lint_map", {"scene_json": text}),
                ("validate_scene", {"scene_json": text2}), ("lint_map", {"scene_json": text2}),
                ("verify_map", {"scene_json": text2}), ("plan_map", {"scene_json": text2}), ("sim_map", {"scene_json": text2})]
    os.makedirs(workdir, exist_ok=True)
    f = os.path.join(workdir, "map.json")
    return [("__write", f, text), ("describe", {"topic": "brief"}), ("search", {"query": "two rooms joined by a door", "limit": 3}),
            ("validate", {"file": "map.json"}), ("lint", {"file": "map.json"}),
            ("__write", f, text2), ("validate", {"file": "map.json"}), ("lint", {"file": "map.json"}),
            ("verify", {"file": "map.json"}), ("view", {"file": "map.json", "kind": "plan"}), ("sim", {"file": "map.json"})]


def run(server, steps, cwd=None, env=None):
    t0 = time.time()
    c = Client(server, cwd, env)
    c.start()
    lt = time.time()
    tools = c.rpc("tools/list", {})
    listed = tools.get("result", {}).get("tools", [])
    out = {"tools": len(listed), "tools_list_bytes": len(json.dumps(tools.get("result", {}), separators=(",", ":")).encode()), "tools_list_seconds": round(time.time() - lt, 3), "calls": []}
    for step in steps:
        if step[0] == "__write":  # the agent's own file edit: not an MCP call, not counted
            with open(step[1], "w", encoding="utf-8") as fh:
                fh.write(step[2])
            continue
        name, args = step
        a = json.dumps(args)
        s = time.time()
        r = c.rpc("tools/call", {"name": name, "arguments": args})
        text, images, image_bytes, err = result_size(r)
        out["calls"].append({"tool": name, "sent_bytes": len(a.encode()), "read_text_bytes": text, "images": images, "image_base64_bytes": image_bytes,
                             "seconds": round(time.time() - s, 3), "error": err})
    c.close()
    out["total"] = {"calls": len(out["calls"]), "sent_bytes": sum(x["sent_bytes"] for x in out["calls"]), "read_text_bytes": sum(x["read_text_bytes"] for x in out["calls"]), "images": sum(x["images"] for x in out["calls"]),
                    "image_base64_bytes": sum(x["image_base64_bytes"] for x in out["calls"]),
                    "call_seconds": round(sum(x["seconds"] for x in out["calls"]), 3), "wall_seconds_including_startup": round(time.time() - t0, 3),
                    "errors": sum(1 for x in out["calls"] if x["error"])}
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", required=True)
    ap.add_argument("--steps", help="a JSON file: [[tool, arguments], ...]")
    ap.add_argument("--task", choices=("old", "new"), help="the built-in nine-call task, spelled for the Python server (old) or the native one (new)")
    ap.add_argument("--scene", default=os.path.join(REPO, "recipes", "rooms_and_door.json"))
    ap.add_argument("--workdir", default=None, help="where the new-server task keeps its map (the server's cwd)")
    ap.add_argument("--cwd", default=REPO)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--repeat", type=int, default=1, help="run the task this many times and report the median time (sizes do not vary)")
    ap.add_argument("--record", metavar="LABEL", help="append the result to benches/history/mcp-server.json under this label")
    a = ap.parse_args()
    steps = json.load(open(a.steps)) if a.steps else []
    cwd = a.cwd
    runs = []
    for _ in range(max(1, a.repeat)):
        if a.task:
            import tempfile
            work = a.workdir or tempfile.mkdtemp(prefix="re2_mcpbench_")
            steps = build_task(a.task, a.scene, work)
            cwd = work if a.task == "new" else a.cwd
        runs.append(run(a.server, steps, cwd))
    r = runs[-1]
    med = lambda xs: sorted(xs)[len(xs) // 2]
    r["repeat"] = len(runs)
    r["total"]["median_call_seconds"] = med([x["total"]["call_seconds"] for x in runs])
    r["total"]["median_wall_seconds_including_startup"] = med([x["total"]["wall_seconds_including_startup"] for x in runs])
    if a.record:
        hist_path = os.path.join(REPO, "benches", "history", "mcp-server.json")
        try:
            hist = json.load(open(hist_path))
        except (OSError, ValueError):
            hist = {"schema": "red-mcp-bench/1", "purpose": "What a tool surface costs an agent on one fixed nine-call task (scripts/mcp_bench.py): the tools/list loaded at session start, the bytes sent, the text read, calls and wall time. Append a run; never edit an old one. No model is involved: this prices the tool surface, not an agent's skill.", "runs": []}
        hist["runs"].append({"label": a.record, "server": a.server, "task": a.task, "scene": os.path.relpath(a.scene, REPO), "tools": r["tools"], "tools_list_bytes": r["tools_list_bytes"], **r["total"], "repeat": len(runs)})
        os.makedirs(os.path.dirname(hist_path), exist_ok=True)
        json.dump(hist, open(hist_path, "w"), indent=1)
    print(json.dumps(r, indent=1) if a.json else json.dumps({k: v for k, v in r.items() if k != "calls"}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
