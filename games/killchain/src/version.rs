//! Which release of Killchain this is, and which wire protocol it speaks, for the home screen, the lobby and a version refusal.
//!
//! Two players whose copies differ in protocol cannot join each other, and the host's refusal cannot say which side is behind (the wire carries only the
//! reason, ADR 2026-10-20-join-by-short-code-survives-a-wrong-map-and-a-slow-relay). So every screen a player looks at before a join says what *this*
//! copy is, and they compare.
//!
//! The release number (the 4 of `killchain-v4`) is given out when the release is built, so the binary does not know it at compile time. The download
//! carries it: the release tool writes `play.cfg` beside the executable with `version=4` (the same file the Play launcher reads to offer an update).
//! A build run from a checkout has no such file and says so.

use red_engine2::net::protocol::PROTOCOL_VERSION;
use std::sync::OnceLock;

/// The release number in the text of a `play.cfg` (`version=4`), if it has one.
pub fn parse_play_cfg(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "version").then(|| value.trim().parse::<u32>().ok()).flatten().filter(|v| *v > 0)
    })
}

/// The release this copy was built as, read once from `play.cfg` beside the executable (`None` for a build that was not installed from a release).
pub fn release() -> Option<u32> {
    static RELEASE: OnceLock<Option<u32>> = OnceLock::new();
    *RELEASE.get_or_init(|| {
        let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        parse_play_cfg(&std::fs::read_to_string(dir.join("play.cfg")).ok()?)
    })
}

/// The line the screens show: `RELEASE 4  PROTOCOL 17`, or `DEV BUILD  PROTOCOL 17`.
pub fn label_for(release: Option<u32>) -> String {
    match release {
        Some(r) => format!("RELEASE {r}  PROTOCOL {PROTOCOL_VERSION}"),
        None => format!("DEV BUILD  PROTOCOL {PROTOCOL_VERSION}"),
    }
}

/// [`label_for`] this copy.
pub fn label() -> String {
    label_for(release())
}

/// What this copy is, in the words of a version refusal (`release 4, protocol 17`).
pub fn describe() -> String {
    red_engine2::net::join_task::describe_client(release())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_is_the_version_line_of_the_play_config() {
        let cfg = "name=Killchain\nslug=killchain\nversion=4\nmode=portable\nupdate_url=https://example.invalid/latest.json\n";
        assert_eq!(parse_play_cfg(cfg), Some(4));
        assert_eq!(parse_play_cfg("version = 12 \r\n"), Some(12), "spaces and a Windows line end do not matter");
        assert_eq!(parse_play_cfg("name=Killchain\n"), None);
        assert_eq!(parse_play_cfg("version=abc\n"), None);
        assert_eq!(parse_play_cfg("version=0\n"), None, "release numbers start at 1");
    }

    #[test]
    fn the_label_says_the_release_and_always_the_protocol() {
        assert_eq!(label_for(Some(4)), format!("RELEASE 4  PROTOCOL {PROTOCOL_VERSION}"));
        assert_eq!(label_for(None), format!("DEV BUILD  PROTOCOL {PROTOCOL_VERSION}"));
        assert!(describe().contains(&format!("protocol {PROTOCOL_VERSION}")));
    }
}
