//! `red_engine2 doctor`: what can this machine do? One call tells an AI (or a person) on an unknown box which parts of
//! Red will work here, and what to install or avoid when one will not.
//!
//! Red is meant to run in very different places: a dev laptop with a GPU, a CI runner with none, a container, a small
//! VPS. Most of it needs nothing (map tools, the dedicated server, auto-walks, `verify` without views); only rendered
//! frames need a GPU adapter (a software one such as Mesa lavapipe or Windows WARP is enough), and only the game client needs a
//! window. `doctor` probes each of those for real (it opens sockets, files, a GPU adapter) instead of guessing from the OS.

use std::net::UdpSocket;
use std::path::Path;
use std::process::{Command, Stdio};

/// How a probe came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Works.
    Ok,
    /// Missing or degraded, but only some commands are affected.
    Warn,
    /// Broken in a way that stops core functionality.
    Fail,
}

/// One probe's result.
#[derive(Debug, Clone)]
pub struct Check {
    /// Short name (`gpu`, `udp`, ...).
    pub name: &'static str,
    /// Outcome.
    pub status: Status,
    /// What was found, with a fix when it is not `Ok`.
    pub detail: String,
}

pub(crate) fn check(name: &'static str, status: Status, detail: impl Into<String>) -> Check {
    Check { name, status, detail: detail.into() }
}

#[cfg(feature = "gfx")]
fn probe_gpu() -> Check {
    crate::probe::gpu()
}

#[cfg(not(feature = "gfx"))]
fn probe_gpu() -> Check {
    check(
        "gpu",
        Status::Warn,
        "this is the headless build (no `gfx` feature): frame/tour/render/golden views are not compiled in. Rebuild without --no-default-features for them",
    )
}

#[cfg(feature = "gfx")]
fn probe_audio() -> Check {
    crate::probe::audio()
}

#[cfg(not(feature = "gfx"))]
fn probe_audio() -> Check {
    check("audio", Status::Ok, "not needed (headless build has no audio code)")
}

fn probe_ffmpeg() -> Check {
    match Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(s) if s.success() => check("ffmpeg", Status::Ok, "found (only `render`/MP4 output needs it)"),
        _ => check("ffmpeg", Status::Warn, "not on PATH: only `render` (MP4 video) needs it; PNG frames, tours and everything else do not"),
    }
}

fn probe_udp() -> Check {
    let loop_ok = (|| -> std::io::Result<bool> {
        let a = UdpSocket::bind("127.0.0.1:0")?;
        let b = UdpSocket::bind("127.0.0.1:0")?;
        b.set_read_timeout(Some(std::time::Duration::from_millis(500)))?;
        a.send_to(b"red", b.local_addr()?)?;
        let mut buf = [0u8; 8];
        Ok(b.recv_from(&mut buf).is_ok())
    })();
    match loop_ok {
        Ok(true) => {
            let port = crate::net::DEFAULT_PORT;
            match UdpSocket::bind(("0.0.0.0", port)) {
                Ok(_) => check(
                    "udp",
                    Status::Ok,
                    format!("loopback works and the default server port {port} is free (players outside your network also need it forwarded/allowed)"),
                ),
                Err(e) => check(
                    "udp",
                    Status::Warn,
                    format!("loopback works but UDP {port} is unavailable ({e}): another server is running, or start with `--port N`"),
                ),
            }
        }
        Ok(false) => check(
            "udp",
            Status::Fail,
            "a loopback UDP packet was not delivered: a firewall or sandbox is blocking UDP, so the multiplayer server and clients cannot talk",
        ),
        Err(e) => check("udp", Status::Fail, format!("cannot create UDP sockets ({e}): multiplayer needs them")),
    }
}

fn probe_out(dir: &Path) -> Check {
    let p = dir.join(".doctor_probe");
    match std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&p, b"ok")) {
        Ok(()) => {
            let _ = std::fs::remove_file(&p);
            check("out", Status::Ok, format!("{} is writable (plans, tours and verify diffs go here)", dir.display()))
        }
        Err(e) => check(
            "out",
            Status::Fail,
            format!("cannot write to {} ({e}): images and reports cannot be saved; pass explicit output paths elsewhere", dir.display()),
        ),
    }
}

fn probe_git() -> Check {
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => check("git", Status::Ok, String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => check("git", Status::Warn, "git not found: `status` shows no git position, `diff --git` and engine cloning by `scripts/red` need it"),
    }
}

/// Runs every probe. `out_dir` is where tools write images (`out/` by default).
pub fn run(out_dir: &Path) -> Vec<Check> {
    vec![
        check(
            "build",
            Status::Ok,
            format!(
                "red_engine2 {} for {}-{}, {} build",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                if cfg!(feature = "gfx") { "full (gfx)" } else { "headless" }
            ),
        ),
        probe_gpu(),
        probe_audio(),
        probe_ffmpeg(),
        probe_udp(),
        probe_out(out_dir),
        probe_git(),
    ]
}

/// The probes as text plus a plain-language "what works here" summary.
pub fn render(checks: &[Check]) -> String {
    let mut s = String::new();
    for c in checks {
        let tag = match c.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        };
        s.push_str(&format!("{tag} {:<7} {}\n", c.name, c.detail));
    }
    let ok = |n: &str| checks.iter().find(|c| c.name == n).is_some_and(|c| c.status != Status::Fail);
    let gpu = checks
        .iter()
        .find(|c| c.name == "gpu")
        .is_some_and(|c| c.status == Status::Ok || (c.status == Status::Warn && c.detail.contains("software rendering")));
    let yn = |b: bool| if b { "yes" } else { "NO " };
    s.push_str("\nwhat works here:\n");
    s.push_str("  yes  map tools: lint, reach, walk (incl. --auto), plan, build, verify --no-views, sim, replay, status, ui-check\n");
    s.push_str(&format!("  {}  dedicated server + bots (UDP multiplayer)\n", yn(ok("udp"))));
    s.push_str(&format!("  {}  rendered images: frame, tour, verify golden views (ui-shot needs none)\n", yn(gpu)));
    s.push_str(&format!("  {}  saving outputs\n", yn(ok("out"))));
    s
}

/// Whether any probe is a hard failure.
pub fn has_failure(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.status == Status::Fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_runs_everywhere_and_summarises() {
        let dir = std::env::temp_dir().join(format!("re2_doctor_{}", std::process::id()));
        let checks = run(&dir);
        assert!(["build", "gpu", "audio", "ffmpeg", "udp", "out", "git"].iter().all(|n| checks.iter().any(|c| c.name == *n)));
        let text = render(&checks);
        assert!(text.contains("what works here") && text.contains("map tools"), "{text}");
        assert_eq!(checks.iter().find(|c| c.name == "out").unwrap().status, Status::Ok, "the temp dir is writable");
        assert!(checks.iter().find(|c| c.name == "udp").unwrap().status != Status::Fail, "loopback UDP works on any dev/CI box");
    }
}
