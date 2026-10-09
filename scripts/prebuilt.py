#!/usr/bin/env python3
"""A prebuilt `red_engine2` that was built from exactly this checkout's sources, instead of a cold compile.

    python3 scripts/prebuilt.py ensure [--root DIR] [--headless] [--no-fetch] [--refresh] [--quiet] [--json]
    python3 scripts/prebuilt.py fingerprint [--root DIR] [--json]
    python3 scripts/prebuilt.py manifest VERSION DIST_DIR          (release packaging: writes the manifest the fetch looks for)

A cold build of the CLI takes 7 minutes on four cores (every Idea Forge note lists it as a cost). A release of the Linux binaries (`release.yml`) is built from one commit; if its
sources are the checkout's sources, its binaries ARE the binaries a build here would produce, so they can be downloaded instead. "Its sources are the checkout's" is a fingerprint:
the git tree entries of everything that goes into a binary (Cargo.toml, Cargo.lock, build.rs, src/, crates/, assets/ and every file an `include_str!`/`include_bytes!` or build.rs
embeds, such as SPEC.md, recipes/, docs/adr/) hashed together. `release.yml` publishes `MANIFEST-<12 hex of the fingerprint>.json` next to the archives, so one call to the releases API
finds the match without downloading anything. No release matches, the tree has uncommitted edits to those inputs (a prebuilt would hide them), the platform is not Linux x86_64, the
network is down or a checksum differs: nothing is installed, the reason is printed, and the caller builds as before.

What is installed lands in `<target dir>/prebuilt/<fingerprint>/` (target dir = CARGO_TARGET_DIR, else <root>/target) with a record `prebuilt.json`; `scripts/red_resolve.py` reads the record
and selects the binary only while the checkout still has that fingerprint. Standard library only; it never builds and never writes outside the target directory.

Environment: RED_NO_FETCH=1 (never use the network), RED_REPO (owner/name, default kevstermcgee/RedEngine), RED_RELEASE_API, RED_RELEASE_BASE (BASE/<tag>/<file>, as `bootstrap.sh`),
RED_RELEASE_DIR (a directory of releases, DIR/<tag>/<file>: a mirror, or a test), RED_PREFIX is not used here.
"""
import argparse
import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request

SCHEMA_MANIFEST = "red-release/1"
SCHEMA_RECORD = "red-prebuilt/1"
INPUT_ROOTS = ("Cargo.toml", "Cargo.lock", "build.rs", "src", "crates", "assets")
NAME_OK = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,120}$")
NEGATIVE_TTL = 3600       # a release that does not match is not looked for again for an hour (the checkout may be rebuilt many times)
NETWORK_ERROR_TTL = 120   # a failed request is retried sooner
MAX_DOWNLOAD = 600 * 2**20
NO_LIBASOUND = "libasound.so.2 is not installed, so the full set (it plays sound) would not start: using the headless build (no rendering commands such as `frame`/`tour`)"


def platform_name():
    machine = os.uname().machine if hasattr(os, "uname") else ""
    return "linux-x86_64" if sys.platform.startswith("linux") and machine in ("x86_64", "amd64") else f"{sys.platform}-{machine or 'unknown'}"


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _git(root, *args):
    try:
        p = subprocess.run(["git", "-C", root, *args], capture_output=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        return None
    return p.stdout if p.returncode == 0 else None


# ------------------------------------------------------------------------------------------------------------------ the fingerprint
def _read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def input_paths(root):
    """Everything that goes into a binary, as repository-relative paths: the fixed roots, the folders build.rs embeds, and every file the Rust sources embed with
    `include_str!`/`include_bytes!` (found by reading them, so a new embedded file needs no list to edit)."""
    paths = set(INPUT_ROOTS)
    for folder in re.findall(r'\(\s*"(docs/[A-Za-z0-9_/-]+)"\s*,\s*"[a-z_]+\.rs"', _read(os.path.join(root, "build.rs"))):
        paths.add(folder)
    pat = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)')
    for top in ("src", "crates"):
        for dirpath, dirnames, filenames in os.walk(os.path.join(root, top)):
            dirnames[:] = [d for d in dirnames if d not in ("target", ".git", "__pycache__")]
            for fn in filenames:
                if not fn.endswith(".rs"):
                    continue
                for rel in pat.findall(_read(os.path.join(dirpath, fn))):
                    target = os.path.normpath(os.path.join(dirpath, rel))
                    inside = os.path.relpath(target, root).replace(os.sep, "/")
                    if not inside.startswith("..") and not any(inside == r or inside.startswith(r + "/") for r in INPUT_ROOTS):
                        paths.add(inside)
    return sorted(paths)


def fingerprint(root):
    """`{"ok": True, "fingerprint": <64 hex>, "inputs": [...]}`, or `{"ok": False, "reason": ...}` where no fingerprint can be trusted (not a git checkout, edited inputs)."""
    root = os.path.abspath(root)
    paths = input_paths(root)
    entries = _git(root, "ls-tree", "-r", "-z", "HEAD", "--", *paths)
    if entries is None or not entries:
        return {"ok": False, "reason": "this is not a git checkout with the engine's sources committed, so no prebuilt can be matched to it"}
    dirty = _git(root, "status", "--porcelain", "-z", "--untracked-files=all", "--", *paths)
    if dirty:
        files = [e[3:].decode("utf-8", "replace") for e in dirty.split(b"\0") if len(e) > 3]
        return {"ok": False, "reason": f"{len(files)} build-input file(s) have uncommitted changes ({', '.join(files[:3])}{', ...' if len(files) > 3 else ''}): a prebuilt would hide them, so this is built from source"}
    return {"ok": True, "fingerprint": hashlib.sha256(b"\0".join(sorted(entries.split(b"\0")))).hexdigest(), "inputs": paths}


# ------------------------------------------------------------------------------------------------------------------ release sources
class DirSource:
    """A directory of releases, DIR/<tag>/<asset>: a mirror, an offline copy, or a test. Newest first by modification time of the tag directory."""

    def __init__(self, directory):
        self.dir = directory

    def releases(self):
        tags = [t for t in os.listdir(self.dir) if os.path.isdir(os.path.join(self.dir, t))]
        tags.sort(key=lambda t: os.stat(os.path.join(self.dir, t)).st_mtime, reverse=True)
        return [{"tag": t, "assets": sorted(os.listdir(os.path.join(self.dir, t)))} for t in tags]

    def get(self, tag, name, dest):
        shutil.copyfile(os.path.join(self.dir, tag, name), dest)


class GitHubSource:
    """The releases of a GitHub repository: the API lists them (one request), the download URLs are BASE/<tag>/<asset> (the same layout `bootstrap.sh` uses)."""

    def __init__(self, repo, api, base, timeout=15):
        self.repo, self.api, self.base, self.timeout = repo, api.rstrip("/"), base.rstrip("/"), timeout

    def _open(self, url):
        return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "red-engine-prebuilt/1", "Accept": "application/vnd.github+json"}), timeout=self.timeout)

    def releases(self):
        with self._open(f"{self.api}/repos/{self.repo}/releases?per_page=30") as r:
            data = json.load(r)
        return [{"tag": x["tag_name"], "assets": [a["name"] for a in x.get("assets", [])]} for x in data if not x.get("draft")]

    def get(self, tag, name, dest):
        with self._open(f"{self.base}/{tag}/{name}") as r, open(dest, "wb") as f:
            if int(r.headers.get("Content-Length") or 0) > MAX_DOWNLOAD:
                raise OSError(f"{name} is larger than {MAX_DOWNLOAD >> 20} MB: refusing")
            done = 0
            for block in iter(lambda: r.read(1 << 20), b""):
                done += len(block)
                if done > MAX_DOWNLOAD:
                    raise OSError(f"{name} is larger than {MAX_DOWNLOAD >> 20} MB: refusing")
                f.write(block)


def source_from_env(env=None):
    env = os.environ if env is None else env
    if env.get("RED_RELEASE_DIR"):
        return DirSource(env["RED_RELEASE_DIR"])
    repo = env.get("RED_REPO") or "kevstermcgee/RedEngine"
    return GitHubSource(repo, env.get("RED_RELEASE_API") or "https://api.github.com", env.get("RED_RELEASE_BASE") or f"https://github.com/{repo}/releases/download")


# ------------------------------------------------------------------------------------------------------------------ selection (pure)
def manifest_name(fp):
    return f"MANIFEST-{fp[:12]}.json"


def select_release(fp, releases):
    """The release built from these sources: `{"tag", "manifest"}`, or `{"reason"}` saying what exists instead. `releases` is newest first: `[{"tag", "assets": [names]}]`."""
    want = manifest_name(fp)
    for r in releases:
        if want in r["assets"]:
            return {"tag": r["tag"], "manifest": want}
    if not releases:
        return {"reason": "no release has been published yet"}
    built = [r["tag"] for r in releases if any(a.startswith("MANIFEST-") for a in r["assets"])]
    if not built:
        return {"reason": f"{len(releases)} release(s), newest {releases[0]['tag']}, but none carries a source manifest (published before prebuilt fetching existed)"}
    return {"reason": f"no release was built from this checkout's sources ({fp[:12]}); the newest with a manifest is {built[0]}, built from different sources"}


def choose_asset(manifest, headless, have_asound):
    """`(asset dict, note)` from a manifest: the full set (CLI with rendering, the client, server, bots) when sound libraries are present and headless was not asked for, else the headless CLI."""
    by_kind = {a.get("kind"): a for a in manifest.get("assets", [])}
    if headless:
        pick, note = by_kind.get("headless"), None
    elif have_asound and "full" in by_kind:
        pick, note = by_kind["full"], None
    else:
        pick, note = by_kind.get("headless"), None if have_asound else NO_LIBASOUND
    return pick, note


def have_libasound():
    try:
        out = subprocess.run(["ldconfig", "-p"], capture_output=True, text=True, timeout=10).stdout
        if "libasound.so.2" in out:
            return True
    except (OSError, subprocess.SubprocessError):
        pass
    return bool(glob.glob("/usr/lib*/libasound.so.2") or glob.glob("/usr/lib/*/libasound.so.2"))


def parse_sums(text):
    """`{file name: sha256}` from a SHA256SUMS file."""
    out = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 2 and re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            out[parts[-1].lstrip("*")] = parts[0]
    return out


# ------------------------------------------------------------------------------------------------------------------ install
def target_dir(root, env=None):
    env = os.environ if env is None else env
    return env.get("CARGO_TARGET_DIR") or os.path.join(os.path.abspath(root), "target")


def install_dir(root, fp, env=None):
    return os.path.join(target_dir(root, env), "prebuilt", fp[:12])


def exe_name(binary):
    return binary + (".exe" if os.name == "nt" else "")


def installed(root, fp, binary="red_engine2", env=None):
    """The installed prebuilt for fingerprint `fp`: `{"exe", "version", "kind", "fingerprint", "dir"}`, or None. Checks the record and that the binary still has the installed size."""
    d = install_dir(root, fp, env)
    try:
        with open(os.path.join(d, "prebuilt.json"), encoding="utf-8") as f:
            rec = json.load(f)
    except (OSError, ValueError):
        return None
    if rec.get("schema") != SCHEMA_RECORD or rec.get("fingerprint") != fp:
        return None
    exe = os.path.join(d, exe_name(binary))
    size = (rec.get("binaries") or {}).get(binary)
    try:
        ok = os.path.isfile(exe) and os.access(exe, os.X_OK) and os.stat(exe).st_size == size
    except OSError:
        ok = False
    return {"exe": exe, "version": rec.get("version"), "kind": rec.get("kind"), "fingerprint": fp, "dir": d} if ok else None


def _safe_extract(archive, dest):
    """Extract regular files only, flat into `dest` (a release archive is one folder of binaries); returns their names."""
    names = []
    with tarfile.open(archive, "r:gz") as tf:
        for m in tf.getmembers():
            base = os.path.basename(m.name)
            if not m.isfile() or not base or not NAME_OK.match(base):
                continue
            src = tf.extractfile(m)
            with open(os.path.join(dest, base), "wb") as out:
                shutil.copyfileobj(src, out)
            names.append(base)
    return names


def fetch_and_install(root, fp, tag, manifest_file, source, headless, env=None, log=None):
    """Download, verify and install the release's matching asset; raises RuntimeError with the reason on any failure (nothing is left installed)."""
    log = log or (lambda m: None)
    if not NAME_OK.match(tag) or not NAME_OK.match(manifest_file):
        raise RuntimeError(f"release or file name {tag!r}/{manifest_file!r} is not a plain name: refusing")
    final = install_dir(root, fp, env)
    os.makedirs(os.path.dirname(final), exist_ok=True)
    work = tempfile.mkdtemp(prefix=".fetch-", dir=os.path.dirname(final))
    try:
        for name in (manifest_file, "SHA256SUMS"):
            source.get(tag, name, os.path.join(work, name))
        sums = parse_sums(_read(os.path.join(work, "SHA256SUMS")))
        mpath = os.path.join(work, manifest_file)
        if sums.get(manifest_file) and sums[manifest_file] != sha256_file(mpath):
            raise RuntimeError(f"checksum mismatch for {manifest_file} in release {tag}: refusing to install")
        with open(mpath, encoding="utf-8") as f:
            manifest = json.load(f)
        if manifest.get("schema") != SCHEMA_MANIFEST or manifest.get("fingerprint") != fp:
            raise RuntimeError(f"release {tag}'s manifest does not describe this checkout's sources (its fingerprint is {str(manifest.get('fingerprint'))[:12]}): refusing")
        if manifest.get("platform") != platform_name():
            raise RuntimeError(f"release {tag} was built for {manifest.get('platform')}, this is {platform_name()}")
        asset, note = choose_asset(manifest, headless, have_libasound())
        if note:
            log(note)
        if not asset or not NAME_OK.match(asset.get("name", "")):
            raise RuntimeError(f"release {tag} has no {'headless' if headless else 'usable'} archive for this machine")
        arc = os.path.join(work, asset["name"])
        log(f"downloading {asset['name']} from release {tag}")
        source.get(tag, asset["name"], arc)
        have = sha256_file(arc)
        for label, want in (("SHA256SUMS", sums.get(asset["name"])), ("the manifest", asset.get("sha256"))):
            if not want:
                raise RuntimeError(f"{asset['name']} is not listed in {label}: refusing to install an unchecked archive")
            if want != have:
                raise RuntimeError(f"checksum mismatch for {asset['name']} ({label} says {want[:12]}, the download is {have[:12]}): refusing to install")
        stage = os.path.join(work, "stage")
        os.makedirs(stage)
        names = _safe_extract(arc, stage)
        if "red_engine2" not in names:
            raise RuntimeError(f"{asset['name']} contains no red_engine2")
        sizes = {}
        for n in names:
            p = os.path.join(stage, n)
            if n in (manifest.get("binaries") or {}) or n in asset.get("binaries", []):
                os.chmod(p, 0o755)
                sizes[n] = os.stat(p).st_size
        record = {"schema": SCHEMA_RECORD, "fingerprint": fp, "version": tag, "kind": asset.get("kind"), "asset": asset["name"], "sha256": have, "commit": manifest.get("commit"),
                  "installed_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "binaries": sizes}
        with open(os.path.join(stage, "prebuilt.json"), "w", encoding="utf-8") as f:
            json.dump(record, f, indent=1)
        if os.path.isdir(final):
            shutil.rmtree(final)
        os.replace(stage, final)
        return record
    finally:
        shutil.rmtree(work, ignore_errors=True)


# ------------------------------------------------------------------------------------------------------------------ ensure
def _cache_path(root, env=None):
    return os.path.join(target_dir(root, env), "prebuilt", ".checked.json")


def _cache_get(root, fp, env=None):
    try:
        with open(_cache_path(root, env), encoding="utf-8") as f:
            e = json.load(f).get(fp[:12])
    except (OSError, ValueError):
        return None
    if e and time.time() - e.get("at", 0) < e.get("ttl", NEGATIVE_TTL):
        return e
    return None


def _cache_put(root, fp, reason, ttl, env=None):
    path = _cache_path(root, env)
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        try:
            with open(path, encoding="utf-8") as f:
                data = json.load(f)
        except (OSError, ValueError):
            data = {}
        data[fp[:12]] = {"at": time.time(), "ttl": ttl, "reason": reason}
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f)
    except OSError:
        pass


def ensure(root, headless=False, fetch=True, refresh=False, source=None, env=None, log=None):
    """Make a prebuilt that matches the checkout available. Returns `{"status": "installed"|"fetched"|"none", "exe", "version", "kind", "reason", "fingerprint"}`; never raises, never builds."""
    env = os.environ if env is None else env
    log = log or (lambda m: None)
    none = lambda reason, fp=None: {"status": "none", "exe": None, "reason": reason, "fingerprint": fp[:12] if fp else None}  # noqa: E731
    if platform_name() != "linux-x86_64":
        return none(f"prebuilt binaries exist for Linux x86_64 only (this is {platform_name()})")
    f = fingerprint(root)
    if not f["ok"]:
        return none(f["reason"])
    fp = f["fingerprint"]
    have = installed(root, fp, env=env)
    if have and not (headless and have["kind"] != "headless"):
        return {"status": "installed", "exe": have["exe"], "version": have["version"], "kind": have["kind"], "reason": None, "fingerprint": fp[:12]}
    if not fetch or env.get("RED_NO_FETCH") == "1":
        return none("fetching is switched off (--no-fetch or RED_NO_FETCH=1)", fp)
    if not refresh:
        cached = _cache_get(root, fp, env)
        if cached:
            return none(f"{cached['reason']} (checked {int((time.time() - cached['at']) / 60)} min ago; `scripts/prebuilt.py ensure --refresh` looks again)", fp)
    src = source or source_from_env(env)
    try:
        releases = src.releases()
    except (OSError, ValueError, KeyError) as e:
        reason = f"could not list the releases ({e})"
        _cache_put(root, fp, reason, NETWORK_ERROR_TTL, env)
        return none(reason, fp)
    pick = select_release(fp, releases)
    if "reason" in pick:
        _cache_put(root, fp, pick["reason"], NEGATIVE_TTL, env)
        return none(pick["reason"], fp)
    try:
        rec = fetch_and_install(root, fp, pick["tag"], pick["manifest"], src, headless, env, log)
    except (OSError, ValueError, RuntimeError, tarfile.TarError) as e:
        reason = f"release {pick['tag']} matches but could not be installed ({e})"
        _cache_put(root, fp, reason, NETWORK_ERROR_TTL, env)
        return none(reason, fp)
    have = installed(root, fp, env=env)
    if not have:
        return none(f"release {pick['tag']} was unpacked but its red_engine2 is not usable", fp)
    return {"status": "fetched", "exe": have["exe"], "version": rec["version"], "kind": rec["kind"], "reason": None, "fingerprint": fp[:12]}


def describe_result(r):
    """One line for a human or an agent: what happened and what happens next."""
    if r["status"] == "installed":
        return f"prebuilt: using release {r['version']} ({r['kind']}), built from exactly this checkout's sources: no compile"
    if r["status"] == "fetched":
        return f"prebuilt: fetched release {r['version']} ({r['kind']}), checksum verified, built from exactly this checkout's sources: no compile"
    return f"prebuilt: none ({r['reason']}); building from source instead (about 7 minutes cold on four cores: run it in the background)"


# ------------------------------------------------------------------------------------------------------------------ release packaging
def write_manifest(root, version, dist):
    """Describe the archives in `dist` (as `package_release.sh` made them) for the fetch: MANIFEST.json and MANIFEST-<fingerprint>.json, and add both to SHA256SUMS."""
    f = fingerprint(root)
    if not f["ok"]:
        raise SystemExit(f"manifest: {f['reason']}")
    commit = (_git(root, "rev-parse", "HEAD") or b"").decode().strip()
    assets = []
    for path in sorted(glob.glob(os.path.join(dist, "*.tar.gz"))):
        with tarfile.open(path, "r:gz") as tf:
            bins = sorted(os.path.basename(m.name) for m in tf.getmembers() if m.isfile() and os.path.basename(m.name) not in ("README.txt",))
        assets.append({"name": os.path.basename(path), "kind": "headless" if path.endswith("-headless.tar.gz") else "full", "binaries": bins, "sha256": sha256_file(path)})
    manifest = {"schema": SCHEMA_MANIFEST, "version": version, "commit": commit, "fingerprint": f["fingerprint"], "inputs": f["inputs"], "platform": "linux-x86_64", "assets": assets}
    names = ["MANIFEST.json", manifest_name(f["fingerprint"])]
    for n in names:
        with open(os.path.join(dist, n), "w", encoding="utf-8") as out:
            json.dump(manifest, out, indent=1, sort_keys=True)
            out.write("\n")
    sums = parse_sums(_read(os.path.join(dist, "SHA256SUMS")))
    for n in names:
        sums[n] = sha256_file(os.path.join(dist, n))
    with open(os.path.join(dist, "SHA256SUMS"), "w", encoding="utf-8") as out:
        for n in sorted(sums):
            out.write(f"{sums[n]}  {n}\n")
    return manifest


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    root_default = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    e = sub.add_parser("ensure", help="make a matching prebuilt available; prints the red_engine2 path (exit 1 and the reason on stderr when there is none)")
    e.add_argument("--root", default=root_default)
    e.add_argument("--headless", action="store_true", help="the headless CLI even where the full set could run")
    e.add_argument("--no-fetch", action="store_true", help="only use what is already installed")
    e.add_argument("--refresh", action="store_true", help="look for a release again even if one was looked for in the last hour")
    e.add_argument("--quiet", action="store_true", help="no progress lines (the path, or the reason, is still printed)")
    e.add_argument("--json", action="store_true")
    fp = sub.add_parser("fingerprint")
    fp.add_argument("--root", default=root_default)
    fp.add_argument("--json", action="store_true")
    m = sub.add_parser("manifest")
    m.add_argument("version")
    m.add_argument("dist")
    m.add_argument("--root", default=root_default)
    a = ap.parse_args(argv)
    if a.cmd == "fingerprint":
        r = fingerprint(a.root)
        if a.json:
            print(json.dumps(r, indent=1))
        else:
            print(r["fingerprint"][:12] if r["ok"] else r["reason"])
        return 0 if r["ok"] else 1
    if a.cmd == "manifest":
        m_ = write_manifest(a.root, a.version, a.dist)
        print(f"manifest: {manifest_name(m_['fingerprint'])} ({len(m_['assets'])} archive(s))")
        return 0
    r = ensure(a.root, headless=a.headless, fetch=not a.no_fetch, refresh=a.refresh, log=None if a.quiet else (lambda msg: print("prebuilt: " + msg, file=sys.stderr)))
    if a.json:
        print(json.dumps(r, indent=1))
    else:
        # --quiet drops the progress lines and the "already installed" line (an agent pays for every byte of stderr); a fetch and a miss are always said
        if r["status"] != "installed" or not a.quiet:
            print(describe_result(r), file=sys.stderr)
        if r["exe"]:
            print(r["exe"])
    return 0 if r["exe"] else 1


if __name__ == "__main__":
    sys.exit(main())
