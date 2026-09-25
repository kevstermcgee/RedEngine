//! `red_engine2 package`: a release you can prove (ADR 0032). One zip holding the tracked source, the built binaries and a
//! `PACKAGE-MANIFEST.json` with the SHA-256 of every file, the exact commit, whether the tree was dirty, the toolchain, and the hash of
//! `Cargo.lock`. `package --verify` re-checks a zip against its own manifest (and against a tamper-proof rule: the headless server and bot
//! must not contain the graphics stack), so "is this the build I think it is?" has a mechanical answer.
//!
//! Choices that make it better than "zip the working directory":
//! * **Reproducible.** Entries are sorted, timestamps are fixed, and no wall-clock time is written (the commit's own time is), so packaging the
//!   same commit with the same binaries twice gives the same bytes.
//! * **Honest about dirt.** A dirty tree is refused unless `--allow-dirty`, and then the dirty files are listed in the manifest.
//! * **Feature-isolated binaries.** The client and offline tools are built with the default features; the dedicated server and the bot are
//!   built with `--no-default-features` into their own target directory, so a graphics-enabled server cannot be packaged by accident. The
//!   manifest records the check and `--verify` repeats it on the files in the zip.
//! * **No new dependencies for the headless build.** Compression is `flate2` (already in the tree for PNG); hashing is `crypto`.

use crate::crypto::{hex, sha256};
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Strings the headless binaries must not contain: the graphics, window and audio stacks.
pub const FORBIDDEN_IN_HEADLESS: &[&str] = &["wgpu", "winit", "rodio", "cpal", "ffmpeg-sidecar"];
/// The names of the binaries that must stay headless.
pub const HEADLESS_BINARIES: &[&str] = &["red_server", "red_bot"];

/// One binary to include.
#[derive(Debug, Clone)]
pub struct Binary {
    /// Its file name in the package (`red_server.exe`).
    pub name: String,
    /// Where it is on disk.
    pub path: PathBuf,
}

/// What to package.
#[derive(Debug, Clone)]
pub struct Options {
    /// The repository root (paths in the package are relative to it).
    pub root: PathBuf,
    /// The files to include, relative to `root`; `None` = `git ls-files`.
    pub files: Option<Vec<PathBuf>>,
    /// The binaries to include under `bin/`.
    pub binaries: Vec<Binary>,
    /// Package a tree with uncommitted changes (they are listed in the manifest).
    pub allow_dirty: bool,
    /// Provenance, when not asked of git (tests).
    pub git: Option<GitInfo>,
    /// Extra facts for the manifest (toolchain, features).
    pub notes: BTreeMap<String, String>,
}

/// Where the sources came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitInfo {
    /// The commit hash.
    pub commit: String,
    /// The commit's time, unix seconds (the package's only notion of "when").
    pub commit_time: u64,
    /// Tracked files with uncommitted changes.
    pub dirty_files: Vec<String>,
    /// New files git does not track yet (and does not ignore): part of the tree, absent from a plain `git ls-files`.
    pub untracked_files: Vec<String>,
}

impl GitInfo {
    /// Whether the tree differs from the commit in any way.
    pub fn dirty(&self) -> bool {
        !self.dirty_files.is_empty() || !self.untracked_files.is_empty()
    }
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").args(args).current_dir(root).output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Reads the repository's commit, its time and its uncommitted files.
pub fn git_info(root: &Path) -> Result<GitInfo, String> {
    let commit = git(root, &["rev-parse", "HEAD"])?.trim().to_string();
    let commit_time = git(root, &["show", "-s", "--format=%ct", "HEAD"])?.trim().parse().unwrap_or(0);
    let (mut dirty_files, mut untracked_files) = (Vec::new(), Vec::new());
    for line in git(root, &["status", "--porcelain", "--untracked-files=all"])?.lines() {
        let path = line.get(3..).unwrap_or(line).rsplit(" -> ").next().unwrap_or("").trim().trim_matches('"').to_string();
        if line.starts_with("??") {
            untracked_files.push(path);
        } else {
            dirty_files.push(path);
        }
    }
    Ok(GitInfo { commit, commit_time, dirty_files, untracked_files })
}

/// The files of the repository, sorted, without the build outputs: the tracked ones, plus (with `include_untracked`) new files git does not
/// track yet but does not ignore.
pub fn tracked_files(root: &Path, include_untracked: bool) -> Result<Vec<PathBuf>, String> {
    let mut all = git(root, &["ls-files", "-z"])?;
    if include_untracked {
        all.push('\0');
        all.push_str(&git(root, &["ls-files", "-z", "--others", "--exclude-standard"])?);
    }
    let mut v: Vec<PathBuf> =
        all.split('\0').filter(|s| !s.is_empty()).map(PathBuf::from).filter(|p| !p.starts_with("out") && !p.starts_with("target")).collect();
    v.sort();
    v.dedup();
    Ok(v)
}

/// Whether `bytes` contains `needle`.
fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && bytes.windows(needle.len()).any(|w| w == needle)
}

/// The forbidden strings found in a supposedly headless binary (empty = clean).
pub fn graphics_in(bytes: &[u8]) -> Vec<&'static str> {
    FORBIDDEN_IN_HEADLESS.iter().copied().filter(|s| contains(bytes, s.as_bytes())).collect()
}

fn is_headless(name: &str) -> bool {
    let stem = name.strip_suffix(".exe").unwrap_or(name);
    HEADLESS_BINARIES.contains(&stem)
}

// ---------------------------------------------------------------------------------------------------------------------------------
// a small, deterministic ZIP writer and reader (deflate; no zip64: a release is far below 4 GiB)
// ---------------------------------------------------------------------------------------------------------------------------------

/// 1980-01-01 00:00:00 in DOS date/time.
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x0021;

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn put32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// Writes `entries` (already in the order wanted) as a ZIP archive.
pub fn write_zip(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in entries {
        if name.len() > u16::MAX as usize || data.len() > u32::MAX as usize / 2 {
            return Err(format!("'{name}' is too large for a plain zip"));
        }
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
        enc.write_all(data).map_err(|e| e.to_string())?;
        let packed = enc.finish().map_err(|e| e.to_string())?;
        let (method, body): (u16, &[u8]) = if packed.len() < data.len() { (8, &packed) } else { (0, data) };
        let crc = crc32fast::hash(data);
        let offset = out.len() as u32;
        // local file header
        put32(&mut out, 0x0403_4b50);
        put16(&mut out, 20);
        put16(&mut out, 0x0800); // UTF-8 names
        put16(&mut out, method);
        put16(&mut out, DOS_TIME);
        put16(&mut out, DOS_DATE);
        put32(&mut out, crc);
        put32(&mut out, body.len() as u32);
        put32(&mut out, data.len() as u32);
        put16(&mut out, name.len() as u16);
        put16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        // central directory entry
        put32(&mut central, 0x0201_4b50);
        put16(&mut central, 20);
        put16(&mut central, 20);
        put16(&mut central, 0x0800);
        put16(&mut central, method);
        put16(&mut central, DOS_TIME);
        put16(&mut central, DOS_DATE);
        put32(&mut central, crc);
        put32(&mut central, body.len() as u32);
        put32(&mut central, data.len() as u32);
        put16(&mut central, name.len() as u16);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put16(&mut central, 0);
        put32(&mut central, 0);
        put32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    put32(&mut out, 0x0605_4b50);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, entries.len() as u16);
    put16(&mut out, entries.len() as u16);
    put32(&mut out, central.len() as u32);
    put32(&mut out, cd_offset);
    put16(&mut out, 0);
    Ok(out)
}

fn rd16(b: &[u8], i: usize) -> Result<u16, String> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "the zip is truncated".to_string())
}
fn rd32(b: &[u8], i: usize) -> Result<u32, String> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "the zip is truncated".to_string())
}

/// Reads every entry of a ZIP archive written by [`write_zip`] (stored or deflated), checking each CRC.
pub fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;
    let eocd =
        (0..bytes.len().saturating_sub(21)).rev().find(|&i| rd32(bytes, i) == Ok(0x0605_4b50)).ok_or("not a zip (no end-of-central-directory record)")?;
    let (count, cd_size, cd_off) = (rd16(bytes, eocd + 10)? as usize, rd32(bytes, eocd + 12)? as usize, rd32(bytes, eocd + 16)? as usize);
    if cd_off.checked_add(cd_size).is_none_or(|e| e > bytes.len()) {
        return Err("the zip's directory is out of range".to_string());
    }
    let mut out = Vec::new();
    let mut p = cd_off;
    for _ in 0..count {
        if rd32(bytes, p)? != 0x0201_4b50 {
            return Err("the zip's directory is damaged".to_string());
        }
        let (method, crc, csize, usize_, nlen, xlen, clen, lho) = (
            rd16(bytes, p + 10)?,
            rd32(bytes, p + 16)?,
            rd32(bytes, p + 20)? as usize,
            rd32(bytes, p + 24)? as usize,
            rd16(bytes, p + 28)? as usize,
            rd16(bytes, p + 30)? as usize,
            rd16(bytes, p + 32)? as usize,
            rd32(bytes, p + 42)? as usize,
        );
        let name = String::from_utf8_lossy(bytes.get(p + 46..p + 46 + nlen).ok_or("the zip is truncated")?).to_string();
        p += 46 + nlen + xlen + clen;
        if rd32(bytes, lho)? != 0x0403_4b50 {
            return Err(format!("the zip's entry '{name}' has no local header"));
        }
        let data_at = lho + 30 + rd16(bytes, lho + 26)? as usize + rd16(bytes, lho + 28)? as usize;
        let body = bytes.get(data_at..data_at + csize).ok_or("the zip is truncated")?;
        let data = match method {
            0 => body.to_vec(),
            8 => {
                let mut v = Vec::with_capacity(usize_);
                DeflateDecoder::new(body).read_to_end(&mut v).map_err(|e| format!("'{name}' does not inflate: {e}"))?;
                v
            }
            m => return Err(format!("'{name}' uses compression method {m}, which this reader does not support")),
        };
        if data.len() != usize_ || crc32fast::hash(&data) != crc {
            return Err(format!("'{name}' fails its zip checksum"));
        }
        out.push((name, data));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------------------------
// package and verify
// ---------------------------------------------------------------------------------------------------------------------------------

/// What was packaged.
#[derive(Debug, Clone)]
pub struct Packaged {
    /// The archive bytes.
    pub zip: Vec<u8>,
    /// The manifest that went inside.
    pub manifest: Value,
    /// Files in the archive (including the manifest).
    pub files: usize,
    /// SHA-256 of the archive: its identity.
    pub sha256: String,
}

/// The top-level folder every member lives in.
fn folder(version: &str, platform: &str) -> String {
    format!("red_engine2-{version}-{platform}")
}

/// The tree's git state, or the refusal a dirty tree earns without `--allow-dirty`. The CLI calls this *before* the (minutes-long) release
/// builds, so a refusal is instant.
pub fn preflight(root: &Path, allow_dirty: bool) -> Result<GitInfo, String> {
    refuse_dirty(git_info(root)?, allow_dirty)
}

fn refuse_dirty(info: GitInfo, allow_dirty: bool) -> Result<GitInfo, String> {
    if info.dirty() && !allow_dirty {
        let mut shown: Vec<String> = info.dirty_files.iter().take(4).cloned().collect();
        shown.extend(info.untracked_files.iter().take(4).map(|f| format!("{f} (untracked)")));
        return Err(format!(
            "the working tree has uncommitted changes ({}); commit them so the package names a real commit, or pass --allow-dirty to include them and record them in the manifest",
            shown.join(", ")
        ));
    }
    Ok(info)
}

/// Builds the package in memory.
pub fn build(opts: &Options) -> Result<Packaged, String> {
    let info = refuse_dirty(
        match &opts.git {
            Some(g) => g.clone(),
            None => git_info(&opts.root)?,
        },
        opts.allow_dirty,
    )?;
    let files = match &opts.files {
        Some(f) => f.clone(),
        None => tracked_files(&opts.root, opts.allow_dirty)?,
    };
    let version = env!("CARGO_PKG_VERSION");
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let top = folder(version, &platform);
    let mut members: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for rel in &files {
        let path = opts.root.join(rel);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("{} is a symlink; a package refuses symlinks (they can point outside the repository)", rel.display()));
        }
        if !meta.is_file() {
            continue; // a submodule or directory entry
        }
        members.insert(format!("{top}/{}", rel.to_string_lossy().replace('\\', "/")), std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    let mut headless_ok = true;
    let mut headless_problems = Vec::new();
    for b in &opts.binaries {
        let data = std::fs::read(&b.path).map_err(|e| format!("{}: {e}", b.path.display()))?;
        if is_headless(&b.name) {
            let found = graphics_in(&data);
            if !found.is_empty() {
                headless_ok = false;
                headless_problems.push(format!("{} contains {}", b.name, found.join(", ")));
            }
        }
        members.insert(format!("{top}/bin/{}", b.name), data);
    }
    if !headless_ok {
        return Err(format!(
            "a headless binary contains the graphics stack ({}): build it with --no-default-features into its own target directory",
            headless_problems.join("; ")
        ));
    }
    let lock_hash = members.get(&format!("{top}/Cargo.lock")).map(|d| hex(&sha256(d)));
    let listing: BTreeMap<&String, Value> = members.iter().map(|(k, v)| (k, json!({"sha256": hex(&sha256(v)), "size": v.len()}))).collect();
    let mut notes = serde_json::Map::new();
    for (k, v) in &opts.notes {
        notes.insert(k.clone(), json!(v));
    }
    let manifest = json!({
        "format": 1,
        "engine": "red_engine2",
        "version": version,
        "platform": platform,
        "commit": info.commit,
        "commit_time": info.commit_time,
        "dirty": info.dirty(),
        "untracked_files_included": info.untracked_files,
        "dirty_files": info.dirty_files,
        "cargo_lock_sha256": lock_hash,
        "headless_binaries_checked": HEADLESS_BINARIES,
        "headless_forbidden": FORBIDDEN_IN_HEADLESS,
        "notes": notes,
        "note": "Not a statement that tests passed: run scripts/ci.sh (or `scripts/dev test`) for that. `red_engine2 package --verify` re-checks every hash here.",
        "files": listing,
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    members.insert(format!("{top}/PACKAGE-MANIFEST.json"), manifest_bytes);
    let entries: Vec<(String, Vec<u8>)> = members.into_iter().collect();
    let zip = write_zip(&entries)?;
    let sha = hex(&sha256(&zip));
    Ok(Packaged { files: entries.len(), zip, manifest, sha256: sha })
}

/// One thing `verify` found wrong.
pub type Problem = String;

/// Re-checks a package against its own manifest. Returns the manifest and the list of problems (empty = the package is what it says).
pub fn verify(zip: &[u8]) -> Result<(Value, Vec<Problem>), String> {
    let entries = read_zip(zip)?;
    let manifest_entry = entries.iter().find(|(n, _)| n.ends_with("/PACKAGE-MANIFEST.json")).ok_or("the zip has no PACKAGE-MANIFEST.json")?;
    let manifest: Value = serde_json::from_slice(&manifest_entry.1).map_err(|e| format!("the manifest is not JSON: {e}"))?;
    let top = manifest_entry.0.trim_end_matches("/PACKAGE-MANIFEST.json").to_string();
    let listed = manifest["files"].as_object().ok_or("the manifest lists no files")?;
    let mut problems = Vec::new();
    let mut seen = BTreeSet::new();
    for (name, data) in &entries {
        if name == &manifest_entry.0 {
            continue;
        }
        seen.insert(name.clone());
        match listed.get(name) {
            None => problems.push(format!("{name}: in the zip but not in the manifest")),
            Some(want) => {
                let got = hex(&sha256(data));
                if want["sha256"].as_str() != Some(got.as_str()) {
                    problems.push(format!("{name}: sha256 is {got}, the manifest says {}", want["sha256"].as_str().unwrap_or("?")));
                }
                if want["size"].as_u64() != Some(data.len() as u64) {
                    problems.push(format!("{name}: size {} differs from the manifest", data.len()));
                }
            }
        }
        // The headless rule, re-checked on what is actually in the zip.
        if name.starts_with(&format!("{top}/bin/")) && is_headless(name.rsplit('/').next().unwrap_or("")) {
            let found = graphics_in(data);
            if !found.is_empty() {
                problems.push(format!("{name}: a headless binary contains the graphics stack ({})", found.join(", ")));
            }
        }
    }
    for name in listed.keys() {
        if !seen.contains(name) {
            problems.push(format!("{name}: in the manifest but missing from the zip"));
        }
    }
    if !top.starts_with("red_engine2-") {
        problems.push(format!("unexpected top-level folder '{top}'"));
    }
    if manifest["dirty"].as_bool() == Some(true) {
        problems.push("note: built from a dirty tree (the manifest lists the uncommitted files)".to_string());
    }
    Ok((manifest, problems))
}

/// The problems that make a package untrustworthy (everything except the informational `note:` lines).
pub fn failures(problems: &[Problem]) -> Vec<&Problem> {
    problems.iter().filter(|p| !p.starts_with("note:")).collect()
}

/// Builds the release binaries with cargo into isolated target directories and returns them. GUI and offline tools: default features;
/// the server and the bot: `--no-default-features` in a different target directory, so nothing graphical can leak into them.
pub fn build_binaries(root: &Path) -> Result<(Vec<Binary>, BTreeMap<String, String>), String> {
    let exe = if cfg!(windows) { ".exe" } else { "" };
    let target = std::env::var("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|_| root.join("target"));
    let run = |dir: &Path, extra: &[&str], bins: &[&str]| -> Result<(), String> {
        let mut c = Command::new("cargo");
        c.current_dir(root).args(["build", "--release", "--locked"]).arg("--target-dir").arg(dir).args(extra);
        for b in bins {
            c.args(["--bin", b]);
        }
        eprintln!("+ {c:?}");
        let status = c.status().map_err(|e| format!("cannot run cargo: {e}"))?;
        status.success().then_some(()).ok_or_else(|| format!("cargo build failed ({status})"))
    };
    let (gui, headless) = (target.join("package-gui"), target.join("package-headless"));
    run(&gui, &[], &["re2", "red_engine2"])?;
    run(&headless, &["--no-default-features"], &["red_server", "red_bot"])?;
    let mut bins = Vec::new();
    for (dir, names) in [(&gui, ["re2", "red_engine2"]), (&headless, ["red_server", "red_bot"])] {
        for n in names {
            bins.push(Binary { name: format!("{n}{exe}"), path: dir.join("release").join(format!("{n}{exe}")) });
        }
    }
    let mut notes = BTreeMap::new();
    if let Ok(v) = Command::new("rustc").arg("--version").output() {
        notes.insert("rustc".to_string(), String::from_utf8_lossy(&v.stdout).trim().to_string());
    }
    notes.insert("gui_build".to_string(), "cargo build --release --locked (default features)".to_string());
    notes.insert("headless_build".to_string(), "cargo build --release --locked --no-default-features (separate target directory)".to_string());
    Ok((bins, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("re2_pkg_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fixture(name: &str, server_bytes: &[u8]) -> Options {
        let d = tmp(name);
        std::fs::write(d.join("README.md"), "hello").unwrap();
        std::fs::write(d.join("Cargo.lock"), "# lock").unwrap();
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/lib.rs"), "fn main() {}\n".repeat(50)).unwrap();
        std::fs::write(d.join("re2.exe"), b"client with wgpu and winit").unwrap();
        std::fs::write(d.join("red_server.exe"), server_bytes).unwrap();
        Options {
            root: d.clone(),
            files: Some(vec!["Cargo.lock".into(), "README.md".into(), "src/lib.rs".into()]),
            binaries: vec![
                Binary { name: "re2.exe".into(), path: d.join("re2.exe") },
                Binary { name: "red_server.exe".into(), path: d.join("red_server.exe") },
            ],
            allow_dirty: false,
            git: Some(GitInfo { commit: "abc123".into(), commit_time: 1_700_000_000, ..Default::default() }),
            notes: BTreeMap::new(),
        }
    }

    #[test]
    fn a_package_verifies_and_packaging_twice_gives_the_same_bytes() {
        let opts = fixture("ok", b"a clean headless server");
        let a = build(&opts).unwrap();
        let b = build(&opts).unwrap();
        assert_eq!(a.zip, b.zip, "reproducible: same inputs, same bytes");
        assert_eq!(a.sha256, b.sha256);
        let (manifest, problems) = verify(&a.zip).unwrap();
        assert!(failures(&problems).is_empty(), "{problems:?}");
        assert_eq!(manifest["commit"], "abc123");
        assert_eq!(manifest["dirty"], false);
        assert_eq!(manifest["cargo_lock_sha256"], hex(&sha256(b"# lock")));
        let names: Vec<String> = read_zip(&a.zip).unwrap().into_iter().map(|e| e.0).collect();
        assert!(names.iter().any(|n| n.ends_with("/bin/red_server.exe")) && names.iter().any(|n| n.ends_with("/PACKAGE-MANIFEST.json")), "{names:?}");
        assert!(names.windows(2).all(|w| w[0] <= w[1]), "entries are sorted");
    }

    #[test]
    fn tampering_is_found_whatever_was_touched() {
        let a = build(&fixture("tamper", b"server")).unwrap();
        let entries = read_zip(&a.zip).unwrap();
        // Change a source file: its hash no longer matches.
        let mut e = entries.clone();
        let i = e.iter().position(|(n, _)| n.ends_with("README.md")).unwrap();
        e[i].1 = b"HELLO".to_vec();
        let (_, p) = verify(&write_zip(&e).unwrap()).unwrap();
        assert!(failures(&p).iter().any(|s| s.contains("README.md") && s.contains("sha256")), "{p:?}");
        // Add a file the manifest does not know.
        let mut e = entries.clone();
        e.push(("red_engine2-x/extra.txt".to_string(), b"sneaky".to_vec()));
        let (_, p) = verify(&write_zip(&e).unwrap()).unwrap();
        assert!(failures(&p).iter().any(|s| s.contains("not in the manifest")), "{p:?}");
        // Remove one.
        let mut e = entries.clone();
        e.remove(i);
        let (_, p) = verify(&write_zip(&e).unwrap()).unwrap();
        assert!(failures(&p).iter().any(|s| s.contains("missing from the zip")), "{p:?}");
        // Damage the archive itself.
        let mut z = a.zip.clone();
        let mid = z.len() / 3;
        z[mid] ^= 0xff;
        assert!(verify(&z).is_err() || !failures(&verify(&z).unwrap().1).is_empty(), "a flipped byte must be caught");
        assert!(verify(b"not a zip at all, nothing to see here").is_err());
    }

    #[test]
    fn a_headless_binary_with_the_graphics_stack_is_refused_at_build_and_at_verify() {
        let dirty = fixture("gfx", b"server built with the default features: wgpu-core, winit");
        let e = build(&dirty).unwrap_err();
        assert!(e.contains("red_server.exe") && e.contains("wgpu") && e.contains("--no-default-features"), "{e}");
        // Verify repeats the check on the archive even if the manifest was forged to omit it.
        let good = build(&fixture("gfx2", b"clean")).unwrap();
        let mut entries = read_zip(&good.zip).unwrap();
        let i = entries.iter().position(|(n, _)| n.ends_with("/bin/red_server.exe")).unwrap();
        entries[i].1 = b"smuggled winit".to_vec();
        let (_, p) = verify(&write_zip(&entries).unwrap()).unwrap();
        assert!(failures(&p).iter().any(|s| s.contains("graphics stack")), "{p:?}");
    }

    #[test]
    fn a_dirty_tree_is_refused_unless_allowed_and_then_recorded() {
        let mut o = fixture("dirty", b"clean");
        o.git = Some(GitInfo { commit: "abc".into(), commit_time: 1, dirty_files: vec!["src/lib.rs".into()], untracked_files: vec![] });
        let e = build(&o).unwrap_err();
        assert!(e.contains("uncommitted") && e.contains("src/lib.rs") && e.contains("--allow-dirty"), "{e}");
        o.allow_dirty = true;
        let p = build(&o).unwrap();
        assert_eq!(p.manifest["dirty"], true);
        assert_eq!(p.manifest["dirty_files"][0], "src/lib.rs");
        let (_, problems) = verify(&p.zip).unwrap();
        assert!(failures(&problems).is_empty() && problems.iter().any(|s| s.starts_with("note:")), "a dirty build is flagged but not a failure: {problems:?}");
    }

    #[test]
    fn untracked_new_files_make_the_tree_dirty_and_are_packaged_only_when_allowed() {
        let mut o = fixture("untracked", b"clean");
        o.git = Some(GitInfo { commit: "abc".into(), commit_time: 1, dirty_files: vec![], untracked_files: vec!["src/new_module.rs".into()] });
        let e = build(&o).unwrap_err();
        assert!(e.contains("src/new_module.rs (untracked)") && e.contains("--allow-dirty"), "{e}");
        o.allow_dirty = true;
        let p = build(&o).unwrap();
        assert_eq!(p.manifest["dirty"], true);
        assert_eq!(p.manifest["untracked_files_included"][0], "src/new_module.rs");
    }

    #[test]
    fn a_real_git_repository_lists_untracked_files_and_ls_files_alone_would_miss_them() {
        let git_ok = |d: &Path, args: &[&str]| {
            Command::new("git")
                .args(["-c", "user.email=t@t", "-c", "user.name=t", "-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(d)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        let d = tmp("realgit");
        if !git_ok(&d, &["init", "-q"]) {
            return; // no git on this machine: nothing to test
        }
        std::fs::write(d.join("tracked.txt"), "a").unwrap();
        assert!(git_ok(&d, &["add", "tracked.txt"]) && git_ok(&d, &["commit", "-q", "-m", "one"]));
        std::fs::write(d.join("brand_new.rs"), "b").unwrap();
        let info = git_info(&d).unwrap();
        assert!(info.dirty_files.is_empty() && info.untracked_files == vec!["brand_new.rs".to_string()] && info.dirty(), "{info:?}");
        assert_eq!(tracked_files(&d, false).unwrap(), vec![PathBuf::from("tracked.txt")], "a plain ls-files leaves the new file out");
        assert_eq!(tracked_files(&d, true).unwrap(), vec![PathBuf::from("brand_new.rs"), PathBuf::from("tracked.txt")]);
        std::fs::write(d.join("tracked.txt"), "changed").unwrap();
        assert_eq!(git_info(&d).unwrap().dirty_files, vec!["tracked.txt".to_string()]);
    }

    #[test]
    fn the_zip_writer_round_trips_compressible_and_incompressible_data() {
        let noisy: Vec<u8> = (0..5000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        let entries = vec![
            ("a/zeros.bin".to_string(), vec![0u8; 100_000]),
            ("a/noise.bin".to_string(), noisy),
            ("a/empty".to_string(), vec![]),
            ("b/ünï.txt".to_string(), b"unicode name".to_vec()),
        ];
        let z = write_zip(&entries).unwrap();
        assert!(z.len() < 30_000, "zeros compress: {} bytes", z.len());
        assert_eq!(read_zip(&z).unwrap(), entries);
    }

    #[test]
    fn a_symlink_is_never_packaged() {
        // Only where the platform lets an unprivileged process make one.
        let o = fixture("sym", b"clean");
        let link = o.root.join("link.md");
        #[cfg(unix)]
        std::os::unix::fs::symlink(o.root.join("README.md"), &link).unwrap();
        #[cfg(windows)]
        if std::os::windows::fs::symlink_file(o.root.join("README.md"), &link).is_err() {
            return;
        }
        let mut o = o;
        o.files.as_mut().unwrap().push("link.md".into());
        assert!(build(&o).unwrap_err().contains("symlink"));
    }
}
