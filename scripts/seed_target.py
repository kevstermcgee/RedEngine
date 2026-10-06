#!/usr/bin/env python3
"""Warm-start a checkout's build directory: copy the *third-party* dependency artifacts a donor checkout has already compiled.

A new git worktree (or a fresh clone) starts with an empty `target/`, and the first build compiles ~420 crates (wgpu, rapier, quinn, ...) from source: minutes of CPU that an
agent spends before it can check anything. Those artifacts are identical in every checkout of the same lock file, so this copies them.

Only third-party crates are copied, never this workspace's own crates: cargo keys a workspace crate by name and version, not by path, so a build directory shared between two
checkouts can answer "up to date" for source it has never seen (measured: two worktrees of one crate printed `Finished in 0.18s` and ran the other's binary). Copying only the
dependencies and letting cargo rebuild the workspace crates itself is safe: a registry crate's fingerprint is its version, features and flags, not where the checkout lives.
A copy that does not fit (another toolchain, other features, other versions) is simply not used by cargo, and costs only disk.

    python3 scripts/seed_target.py                       # donor = the first worktree's target/ (the main checkout), profiles debug
    python3 scripts/seed_target.py --from ../Other/target --profiles debug,fast
    python3 scripts/seed_target.py --dry-run             # say what would be copied
"""
import argparse, json, os, pathlib, re, shutil, subprocess, sys, time

ROOT = pathlib.Path(__file__).resolve().parent.parent


def cargo():
    """cargo, found the way scripts/dev finds it (it is often not on a non-login shell's PATH)."""
    for c in (shutil.which("cargo"), os.path.expanduser("~/.cargo/bin/cargo"), os.path.expanduser("~/.local/toolchain/bin/cargo")):
        if c and os.path.exists(c):
            return c
    sys.exit("cargo not found (see scripts/dev doctor)")


def third_party_crates():
    out = subprocess.run([cargo(), "metadata", "--format-version", "1", "--manifest-path", str(ROOT / "Cargo.toml")], capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit("cargo metadata failed: " + out.stderr.strip()[-300:])
    meta = json.loads(out.stdout)
    return {p["name"].replace("-", "_") for p in meta["packages"] if p["source"] is not None}


def donor_target():
    out = subprocess.run(["git", "-C", str(ROOT), "worktree", "list", "--porcelain"], capture_output=True, text=True).stdout
    first = next((ln.split(" ", 1)[1] for ln in out.splitlines() if ln.startswith("worktree ")), None)
    return pathlib.Path(first) / "target" if first else None


def size_of(path):
    if path.is_file():
        return path.stat().st_size
    total = 0
    for r, _, files in os.walk(path):
        for f in files:
            try:
                total += os.path.getsize(os.path.join(r, f))
            except OSError:
                pass
    return total


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--from", dest="src", default=None, help="the donor target directory (default: the main checkout's)")
    ap.add_argument("--to", dest="dst", default=None, help="the target directory to fill (default: this checkout's, or $CARGO_TARGET_DIR)")
    ap.add_argument("--profiles", default="debug", help="comma list of profile directories to seed (debug, fast, release, ...)")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    src = pathlib.Path(a.src) if a.src else donor_target()
    dst = pathlib.Path(a.dst or os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    if not src or not src.is_dir():
        sys.exit(f"no donor target directory ({src}): build something in another checkout first, or pass --from")
    if src.resolve() == dst.resolve():
        sys.exit("the donor and the destination are the same directory")
    crates = third_party_crates()
    pat = re.compile(r"^(.+?)-[0-9a-f]{16}")

    def wanted(name):
        """Whether an artifact name (`libfoo-<hash>.rlib`, `.fingerprint/foo-<hash>`, `build/foo-<hash>`) belongs to a third-party crate. A `lib` prefix is a file-name prefix in
        `deps/` but part of the name in the others (`libc`, `libm`): try both readings, because copying one entry too many costs disk and nothing else."""
        m = pat.match(name)
        if not m:
            return False
        stem = m.group(1).replace("-", "_")
        return stem in crates or (stem.startswith("lib") and stem[3:] in crates)

    t0 = time.time()
    copied = skipped = 0
    nbytes = 0
    for prof in a.profiles.split(","):
        for sub in ("deps", ".fingerprint", "build"):
            d = src / prof / sub
            if not d.is_dir():
                continue
            (dst / prof / sub).mkdir(parents=True, exist_ok=True)
            for e in os.scandir(d):
                if not wanted(e.name):
                    continue
                to = dst / prof / sub / e.name
                if to.exists():
                    skipped += 1
                    continue
                nbytes += size_of(pathlib.Path(e.path))
                copied += 1
                if not a.dry_run:
                    if e.is_dir():
                        shutil.copytree(e.path, to, symlinks=True, copy_function=shutil.copy2)
                    else:
                        shutil.copy2(e.path, to, follow_symlinks=False)
    verb = "would copy" if a.dry_run else "copied"
    print(f"seed_target: {verb} {copied} third-party artifact entries ({nbytes / 1e9:.2f} GB) from {src} to {dst} in {time.time() - t0:.1f} s; {skipped} already there; workspace crates are left for cargo to build")


if __name__ == "__main__":
    main()
