//! `scripts/bootstrap.sh` (ADR 2026-10-04 "Prebuilt Linux binaries"): installs from a release, refuses a bad checksum, falls back to the headless build.
//! It runs against a release made on the spot in a temp directory (`RED_RELEASE_BASE=file://...`), so no network or real release is involved.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/bootstrap.sh")
}

/// A fake release `tag` in `dir`: a tarball holding a `red_engine2` that prints its own name, a checksum file (corrupted when asked).
fn release(dir: &Path, tag: &str, flavour: &str, corrupt: bool) {
    let name = format!("red-engine-{tag}-linux-x86_64{flavour}");
    let stage = dir.join("stage").join(&name);
    std::fs::create_dir_all(&stage).unwrap();
    let bin = stage.join("red_engine2");
    std::fs::write(&bin, format!("#!/bin/sh\necho \"fake red_engine2 {name} $1\"\n")).unwrap();
    Command::new("chmod").arg("+x").arg(&bin).status().unwrap();
    let out = dir.join(tag);
    std::fs::create_dir_all(&out).unwrap();
    assert!(Command::new("tar").arg("-C").arg(dir.join("stage")).arg("-czf").arg(out.join(format!("{name}.tar.gz"))).arg(&name).status().unwrap().success());
    let sum = Command::new("sha256sum").arg(format!("{name}.tar.gz")).current_dir(&out).output().unwrap();
    let mut text = String::from_utf8(sum.stdout).unwrap();
    if corrupt {
        text = text.replacen(&text[..4], "dead", 1);
    }
    std::fs::write(out.join("SHA256SUMS"), text).unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("re2_bootstrap_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec![script().to_str().unwrap().to_string()];
    args.extend(extra.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut cmd = Command::new("sh");
    cmd.args(&refs).current_dir(dir).env("RED_RELEASE_BASE", format!("file://{}", dir.display())).env("HOME", dir);
    cmd.output().unwrap()
}

#[test]
fn it_installs_a_release_links_the_binaries_and_runs_the_doctor() {
    let dir = scratch("ok");
    release(&dir, "v9.9.9", "-headless", false);
    let out = run(&dir, &["--tag", "v9.9.9", "--headless", "--prefix", dir.join("prefix").to_str().unwrap()]);
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}");
    let linked = dir.join("prefix/bin/red_engine2");
    assert!(linked.exists(), "the binary is on the prefix's bin: {text}");
    let said = Command::new(&linked).arg("hello").output().unwrap();
    assert!(String::from_utf8_lossy(&said.stdout).contains("fake red_engine2 red-engine-v9.9.9-linux-x86_64-headless hello"));
    assert!(text.contains("what this machine can do"), "{text}");
    assert!(dir.join("prefix/share/red_engine2/v9.9.9/red_engine2").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn it_refuses_a_checksum_that_does_not_match_and_installs_nothing() {
    let dir = scratch("bad");
    release(&dir, "v9.9.9", "-headless", true);
    let out = run(&dir, &["--tag", "v9.9.9", "--headless", "--prefix", dir.join("prefix").to_str().unwrap(), "--no-doctor"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("checksum mismatch"), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!dir.join("prefix/bin/red_engine2").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_release_and_a_bad_flag_are_explained() {
    let dir = scratch("missing");
    let out = run(&dir, &["--tag", "v0.0.0", "--headless", "--prefix", dir.join("prefix").to_str().unwrap()]);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("could not download"));
    let out = run(&dir, &["--nonsense"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--help"));
    let _ = std::fs::remove_dir_all(&dir);
}
