#!/usr/bin/env python3
"""Compare the latest `cargo bench --bench sim` results against the stored baseline: PASS/FAIL, not a guess.

    cargo bench --bench sim
    python benches/check.py                 # exit 1 if any benchmark regressed beyond the tolerance
    python benches/check.py --bless         # accept the current numbers as the new baseline
    python benches/check.py --tolerance 0.5 --only tick/   # looser, filtered

Reads criterion's `target/criterion/**/new/{benchmark,estimates}.json` (median, ns per iteration) and
`benches/baseline.json`. Wall-clock micro-benchmarks are noisy (and machine-specific): the default
tolerance is 35% and differences under 40 ns are ignored. The noise-free, deterministic signals live
in tests instead (`tests/alloc_budget.rs`, the body/entity counts in `physics::tests`).
"""
import argparse, json, pathlib, platform, sys, datetime

ROOT = pathlib.Path(__file__).resolve().parent.parent
CRITERION = ROOT / "target" / "criterion"
BASELINE = ROOT / "benches" / "baseline.json"
NOISE_FLOOR_NS = 40.0


def current():
    out = {}
    for bj in CRITERION.glob("**/new/benchmark.json"):
        est = bj.parent / "estimates.json"
        if not est.exists():
            continue
        full_id = json.loads(bj.read_text())["full_id"]
        out[full_id] = json.loads(est.read_text())["median"]["point_estimate"]
    return out


def fmt(ns):
    return f"{ns:9.1f} ns" if ns < 1e3 else f"{ns/1e3:9.2f} us" if ns < 1e6 else f"{ns/1e6:9.2f} ms"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bless", action="store_true", help="write the current numbers as the baseline")
    ap.add_argument("--tolerance", type=float, default=0.35, help="allowed slowdown, fraction (default 0.35)")
    ap.add_argument("--only", default="", help="only ids containing this text")
    ap.add_argument("--baseline", default=str(BASELINE), help="baseline file (default benches/baseline.json)")
    a = ap.parse_args()

    cur = current()
    if not cur:
        sys.exit("no criterion results under target/criterion — run `cargo bench --bench sim` first")
    path = pathlib.Path(a.baseline)

    if a.bless:
        doc = {
            "about": "median ns/iter from `cargo bench --bench sim` (bench profile). Machine-specific: re-bless on a new machine.",
            "blessed": datetime.date.today().isoformat(),
            "machine": platform.platform() + " / " + platform.processor(),
            "results": {k: round(v, 2) for k, v in sorted(cur.items())},
        }
        path.write_text(json.dumps(doc, indent=2) + "\n")
        print(f"blessed {len(cur)} benchmarks -> {path}")
        return

    base = json.loads(path.read_text())["results"]
    fails, rows = 0, []
    for k in sorted(set(base) | set(cur)):
        if a.only not in k:
            continue
        if k not in cur:
            rows.append((k, base[k], None, "MISSING (bench removed/renamed? re-bless)"))
            fails += 1
        elif k not in base:
            rows.append((k, None, cur[k], "new (not in baseline; --bless to record)"))
        else:
            b, c = base[k], cur[k]
            ratio = c / b if b else float("inf")
            if c > b * (1 + a.tolerance) and (c - b) > NOISE_FLOOR_NS:
                rows.append((k, b, c, f"REGRESSED x{ratio:.2f}"))
                fails += 1
            elif c < b * (1 - a.tolerance) and (b - c) > NOISE_FLOOR_NS:
                rows.append((k, b, c, f"faster x{ratio:.2f} (consider --bless)"))
            else:
                rows.append((k, b, c, "ok"))
    w = max(len(r[0]) for r in rows)
    for k, b, c, note in rows:
        print(f"{k:<{w}}  base {fmt(b) if b else '        -   '}  now {fmt(c) if c else '        -   '}  {note}")
    print(f"\n{'FAIL' if fails else 'PASS'}: {fails} regression(s) beyond {a.tolerance:.0%}")
    sys.exit(1 if fails else 0)


if __name__ == "__main__":
    main()
