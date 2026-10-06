#!/usr/bin/env python3
"""Discovery benchmark: from a plain-language request, does `search` put the right place in the top results, and how much does reading the answer cost?

    python3 benches/discovery_bench.py run [--top 3] [--label TEXT] [--no-record] [--engine PATH]
    python3 benches/discovery_bench.py list

Each query of `benches/discovery_queries.json` is run through the built CLI's `search`; a query is a HIT when any of its `accept` substrings appears in the title or the
where-to-read-more line of one of the first `--top` results. The run records recall@top, the mean bytes of an answer (what an agent reads), and the misses, in
`benches/history/discovery.json`. Deterministic (search is, and calls no model), so a change in the number is a change in the engine's docs or index, never noise.
"""
import argparse, datetime, json, os, pathlib, re, subprocess, sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
HISTORY = ROOT / "benches" / "history" / "discovery.json"
SCHEMA = "red-discovery/1"


def cli(engine):
    target = os.environ.get("CARGO_TARGET_DIR")
    return (pathlib.Path(target) if target else pathlib.Path(engine) / "target") / "debug" / "red_engine2"


def results(out):
    """The (title, where) of each result in `search`'s text output: a `[kind] title` line, then body lines, then `-> where`."""
    res, cur = [], None
    for line in out.splitlines():
        m = re.match(r"^\[(\w+)\] (.*)$", line)
        if m:
            cur = {"kind": m.group(1), "title": m.group(2), "where": ""}
            res.append(cur)
        elif cur is not None and "->" in line:
            cur["where"] = line.split("->", 1)[1].strip()
    return res


def run(a):
    engine = pathlib.Path(a.engine or ROOT).resolve()
    red = cli(engine)
    if not red.exists():
        sys.exit(f"{red} is not built")
    queries = json.loads((ROOT / "benches" / "discovery_queries.json").read_text())["queries"]
    hits, sizes, misses, rows = 0, [], [], []
    for q in queries:
        p = subprocess.run([str(red), "search", q["q"]], capture_output=True, text=True, env=dict(os.environ, NO_COLOR="1", TERM="dumb"))
        out = p.stdout + p.stderr
        found = results(out)[: a.top]
        text = " | ".join(f"{r['title']} {r['where']}" for r in found).lower()
        hit = any(acc.lower() in text for acc in q["accept"])
        hits += hit
        sizes.append(len(out.encode()))
        if not hit:
            misses.append({"q": q["q"], "top": [f"[{r['kind']}] {r['title'][:60]}" for r in found]})
        rows.append((hit, q["q"], len(out.encode()), found[0]["title"][:58] if found else "(nothing)"))
    for hit, q, n, top in rows:
        print(f"  {'HIT ' if hit else 'MISS'} {q[:62]:<62} {n:6d} B  first: {top}")
    n = len(queries)
    rec = {"schema": SCHEMA, "date": datetime.datetime.now().astimezone().isoformat(timespec="seconds"), "label": a.label,
           "engine_commit": subprocess.run(["git", "-C", str(engine), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip(),
           "top": a.top, "queries": n, "hits": hits, "recall": round(hits / n, 3), "mean_answer_bytes": sum(sizes) // n, "misses": misses}
    print(f"  recall@{a.top}: {hits}/{n} = {hits / n:.0%}; an answer is {rec['mean_answer_bytes']} B on average (~{rec['mean_answer_bytes'] // 4} tokens)")
    if not a.no_record:
        data = json.loads(HISTORY.read_text()) if HISTORY.exists() else {"schema": SCHEMA, "runs": []}
        data["runs"].append(rec)
        HISTORY.write_text(json.dumps(data, indent=2) + "\n")
        print(f"  recorded in {HISTORY.relative_to(ROOT)} (run {len(data['runs'])})")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--top", type=int, default=3)
    r.add_argument("--label", default="")
    r.add_argument("--no-record", action="store_true")
    r.add_argument("--engine", default=None)
    sub.add_parser("list")
    a = ap.parse_args()
    if a.cmd == "run":
        run(a)
    else:
        for r in json.loads(HISTORY.read_text())["runs"] if HISTORY.exists() else []:
            print(f"{r['date'][:16]} {r['engine_commit']:<9} recall@{r['top']} {r['hits']}/{r['queries']}  {r['mean_answer_bytes']:6d} B/answer  {r['label']}")


if __name__ == "__main__":
    main()
