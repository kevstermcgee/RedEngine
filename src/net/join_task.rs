//! JOIN on a worker thread: the window keeps drawing while the code is looked up, the relay asked and the host connected to (ADR
//! 2026-10-20-join-by-short-code-survives-a-wrong-map-and-a-slow-relay).
//!
//! `JoinTarget::client_config` and `NetSession::wait_connected` block: a name lookup that never returns, a relay that does not answer (five
//! seconds), a host that does not answer (six seconds per map), three maps in a row. Run on the thread that draws the window they froze it, and the
//! "CONNECTING..." the screen was told to show was never painted. A [`JoinTask`] runs the same steps on a thread of its own; the screen asks it for a
//! line to show ([`JoinTask::status`]) and for the result ([`JoinTask::poll`]) once a frame, and can walk away ([`JoinTask::cancel`]).
//!
//! **Each map is a fresh attempt.** The host plays one map and a joiner must hold the same file, so the maps the player has are tried in turn, the host
//! refusing the wrong ones with `WrongMap`. A relay claim is single-use and bound to the socket that sent it, so for a short code *every* attempt asks
//! the relay again (a new claim) and connects from a new socket; reusing the first claim made every attempt after the first fail silently. A host that
//! stays silent is asked once more with a fresh claim before the task gives up, because a lost datagram is the most likely reason. The wire is exactly
//! what it was: nothing here needs a newer relay or server.

use super::bot::ClientWorld;
use super::client::ConnState;
use super::join::{refusal_text, JoinError, JoinTarget};
use super::protocol::{RejectReason, PROTOCOL_VERSION};
use super::relay_server::ResolveError;
use super::session::NetSession;
use crate::schema::Scene;
use crate::sim::player::PlayerState;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the relay gets to answer a short code (the question is asked several times inside it).
pub const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a host gets to welcome us once the connection to it is being made.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);

/// A map this copy of the game can play, to try against the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapChoice {
    /// A short name for the screen (`IRONWORKS`).
    pub label: String,
    /// The scene file.
    pub path: PathBuf,
}

/// What to join and how.
#[derive(Debug, Clone)]
pub struct JoinPlan {
    /// The code, understood.
    pub target: JoinTarget,
    /// The relay a short code is resolved against (`net::relay::relay_from_env`); `None` = relays are off on this PC.
    pub relay: Option<String>,
    /// The player's name.
    pub name: String,
    /// The maps to try, in order: the one the host most likely plays first.
    pub maps: Vec<MapChoice>,
    /// How long the relay gets to answer ([`RESOLVE_TIMEOUT`]).
    pub resolve_timeout: Duration,
    /// How long a host gets to welcome us ([`CONNECT_TIMEOUT`]).
    pub connect_timeout: Duration,
    /// What a game does to a new session before the first packet is read (Killchain sets how its own part of a snapshot is decoded): the session is
    /// polled on the worker thread, so this cannot wait until the window has it.
    pub prepare: Option<fn(&mut NetSession)>,
}

impl JoinPlan {
    /// A plan with the usual time limits.
    pub fn new(target: JoinTarget, relay: Option<String>, name: &str, maps: Vec<MapChoice>) -> JoinPlan {
        JoinPlan { target, relay, name: name.to_string(), maps, resolve_timeout: RESOLVE_TIMEOUT, connect_timeout: CONNECT_TIMEOUT, prepare: None }
    }
}

/// A connection the host welcomed: the session, ready for the renderer to be built around it.
pub struct Joined {
    /// The connected session (its avatar pool is not built yet: [`NetSession::add_avatar_pool`]).
    pub session: NetSession,
    /// The scene of the map the host plays.
    pub scene: Scene,
    /// Where the map was loaded from.
    pub map: PathBuf,
    /// Its place in [`JoinPlan::maps`].
    pub map_index: usize,
    /// Where the server put us (`None` when it put us in a lobby).
    pub spawn: Option<PlayerState>,
}

/// Why a join did not happen, in the terms a player can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinCause {
    /// The text is not a join code.
    BadCode,
    /// A short code was typed but relays are switched off on this PC.
    RelaysOff,
    /// The relay's (or the host's) name could not be looked up: DNS.
    NameLookup,
    /// The relay's machine answered that nothing listens there: the relay is switched off.
    RelayDown,
    /// The relay did not answer at all: down, or the network drops UDP.
    RelayNoAnswer,
    /// The relay answered that nobody holds this code.
    CodeNotLive,
    /// The relay gave no identity to check the host against.
    HostNotVerifiable,
    /// A long code with no identity in it.
    NoIdentity,
    /// This PC could not send, or the relay answered with something that is not an answer.
    Network,
    /// The host did not answer (twice).
    HostNoAnswer,
    /// The host answered and refused us.
    Refused(RejectReason),
    /// The player walked away.
    Cancelled,
    /// Something on this PC failed (a map would not load).
    Local,
}

/// A join that failed: the cause (for the code) and the words (for the player, with what to do next).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinFailure {
    /// What went wrong.
    pub cause: JoinCause,
    /// A sentence or two for the screen.
    pub text: String,
}

impl std::fmt::Display for JoinFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for JoinFailure {}

/// A [`JoinError`] (the code could not become a connection attempt) as a failure.
pub fn failure_of(e: &JoinError) -> JoinFailure {
    let cause = match e {
        JoinError::BadCode(_) => JoinCause::BadCode,
        JoinError::RelaysOff => JoinCause::RelaysOff,
        JoinError::HostNotVerifiable => JoinCause::HostNotVerifiable,
        JoinError::NoIdentity => JoinCause::NoIdentity,
        JoinError::AddressNotFound(_) => JoinCause::NameLookup,
        JoinError::Relay(ResolveError::RelayNotFound(_)) => JoinCause::NameLookup,
        JoinError::Relay(ResolveError::RelayDown(_)) => JoinCause::RelayDown,
        JoinError::Relay(ResolveError::NoAnswer(_)) => JoinCause::RelayNoAnswer,
        JoinError::Relay(ResolveError::CodeNotLive) => JoinCause::CodeNotLive,
        JoinError::Relay(ResolveError::Unreachable(_) | ResolveError::Unexpected(_)) => JoinCause::Network,
    };
    JoinFailure { cause, text: e.to_string() }
}

/// The failure of a join the player walked away from (it has no words: nobody is reading).
fn cancelled() -> JoinFailure {
    JoinFailure { cause: JoinCause::Cancelled, text: String::new() }
}

/// A host's refusal as a failure: `mine` says what this client is (see [`refusal_text`]).
pub fn refused(reason: RejectReason, mine: &str) -> JoinFailure {
    JoinFailure { cause: JoinCause::Refused(reason), text: refusal_text(reason, mine) }
}

/// The words for a host that never answered, for a code that goes through a relay or one that goes straight to it.
pub fn host_silent(target: &JoinTarget, secs: f64) -> JoinFailure {
    let text = match target {
        JoinTarget::Relay(_) => format!(
            "The relay knows that code, but the host did not answer in {secs} s (asked twice). The game may be closed or the host's connection lost: ask your friend to check their lobby, then try again."
        ),
        JoinTarget::Direct(code) => format!(
            "No answer from {} in {secs} s (asked twice). The server may be off, the code old, or a firewall may block UDP: check the code, try another network, or ask the host.",
            code.address
        ),
    };
    JoinFailure { cause: JoinCause::HostNoAnswer, text }
}

/// How waiting for a welcome ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waited {
    /// Welcomed (the spawn state is in the session).
    Joined,
    /// The host refused us.
    Refused(RejectReason),
    /// Nothing within the time.
    TimedOut,
    /// The connection closed before it opened.
    Closed,
    /// The player walked away.
    Cancelled,
}

/// Polls `session` until the host welcomes it, refuses it, `budget` runs out or `cancel` is set. Returns how it ended and, when welcomed, where the
/// server put us. A window-free stand-in for the part of [`NetSession::wait_connected`] that tells a refusal from silence.
pub fn wait_joined(session: &mut NetSession, budget: Duration, cancel: &AtomicBool) -> (Waited, Option<PlayerState>) {
    let end = Instant::now() + budget;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return (Waited::Cancelled, None);
        }
        match session.wait_connected(0.1) {
            Ok(spawn) => return (Waited::Joined, spawn),
            Err(_) => match session.client.state() {
                ConnState::Rejected(r) => return (Waited::Refused(r), None),
                ConnState::Closed => return (Waited::Closed, None),
                _ => {}
            },
        }
        if Instant::now() >= end {
            return (Waited::TimedOut, None);
        }
    }
}

/// A join running on a thread of its own.
pub struct JoinTask {
    rx: mpsc::Receiver<Result<Joined, JoinFailure>>,
    status: Arc<Mutex<String>>,
    cancel: Arc<AtomicBool>,
    finished: bool,
}

impl JoinTask {
    /// Starts the join. `mine` says what this client is, for a version refusal (`"release 4, protocol 17"`).
    pub fn start(plan: JoinPlan, mine: &str) -> JoinTask {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new("Starting...".to_string()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (say_to, cancelled, mine) = (status.clone(), cancel.clone(), mine.to_string());
        // If the thread cannot be made, the sender is dropped with the closure and the first `poll` says the attempt stopped.
        let _ = std::thread::Builder::new().name("red-join".into()).spawn(move || {
            let say = |line: String| *say_to.lock().unwrap_or_else(|p| p.into_inner()) = line;
            let result = run(&plan, &mine, &say, &cancelled);
            let _ = tx.send(result);
        });
        JoinTask { rx, status, cancel, finished: false }
    }

    /// What the join is doing right now, for the screen (`Asking the relay...`).
    pub fn status(&self) -> String {
        self.status.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The result, once there is one (`None` until then, and after it was handed out). Call it once a frame.
    pub fn poll(&mut self) -> Option<Result<Joined, JoinFailure>> {
        if self.finished {
            return None;
        }
        match self.rx.try_recv() {
            Ok(r) => {
                self.finished = true;
                Some(r)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.finished = true;
                Some(Err(JoinFailure { cause: JoinCause::Local, text: "The connection attempt stopped unexpectedly. Try again.".to_string() }))
            }
        }
    }

    /// Walks away: the worker sees it at its next look (within a tenth of a second while it waits for the host; a name lookup in flight finishes by
    /// itself), then drops whatever it had, which closes a half-made connection: it ends well under a second after this (0.37 s measured against a
    /// host that never answered). Nothing waits for it: dropping the task is enough.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for JoinTask {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The join itself, on the worker thread.
fn run(plan: &JoinPlan, mine: &str, say: &dyn Fn(String), cancel: &AtomicBool) -> Result<Joined, JoinFailure> {
    let total = plan.maps.len();
    if total == 0 {
        return Err(JoinFailure { cause: JoinCause::Local, text: "This copy of the game has no map to join with. Reinstall it.".to_string() });
    }
    let relayed = matches!(plan.target, JoinTarget::Relay(_));
    let (mut i, mut asked_again) = (0usize, false);
    while i < total {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let map = &plan.maps[i];
        say(if relayed { "Asking the relay for the host...".to_string() } else { "Looking up the host...".to_string() });
        // A fresh claim for every attempt (a short code is resolved again here; a long code only looks its name up).
        let resolved = plan.target.client_config(plan.relay.as_deref(), plan.resolve_timeout);
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let mut cfg = resolved.map_err(|e| failure_of(&e))?;
        let (scene, world) =
            ClientWorld::load(&map.path).map_err(|e| JoinFailure { cause: JoinCause::Local, text: format!("Cannot load the map {}: {e}", map.label) })?;
        cfg.map_hash = world.map_hash;
        cfg.name = plan.name.clone();
        say(match (i, asked_again) {
            (_, true) => format!("The host has not answered: asking again ({})...", map.label),
            (0, false) => format!("Connecting to the host ({})...", map.label),
            _ => format!("Not that map: trying {} ({} of {total})...", map.label, i + 1),
        });
        let mut session = NetSession::connect_with(cfg, world).map_err(|e| JoinFailure {
            cause: JoinCause::Network,
            text: format!("This PC could not open a network connection ({e}). Check that a firewall is not blocking the game."),
        })?;
        if let Some(prepare) = plan.prepare {
            prepare(&mut session);
        }
        match wait_joined(&mut session, plan.connect_timeout, cancel) {
            (Waited::Joined, spawn) => return Ok(Joined { session, scene, map: map.path.clone(), map_index: i, spawn }),
            // The host plays another map: the next one is a new attempt.
            (Waited::Refused(RejectReason::WrongMap), _) => {
                i += 1;
                asked_again = false;
            }
            (Waited::Refused(r), _) => return Err(refused(r, mine)),
            (Waited::TimedOut | Waited::Closed, _) if !asked_again => asked_again = true,
            (Waited::TimedOut | Waited::Closed, _) => return Err(host_silent(&plan.target, plan.connect_timeout.as_secs_f64())),
            (Waited::Cancelled, _) => return Err(cancelled()),
        }
    }
    Err(refused(RejectReason::WrongMap, mine))
}

/// What a client is, in the words of a version refusal: `release 4, protocol 17`, or just `protocol 17` when the release is not known.
pub fn describe_client(release: Option<u32>) -> String {
    match release {
        Some(r) => format!("release {r}, protocol {PROTOCOL_VERSION}"),
        None => format!("protocol {PROTOCOL_VERSION}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_way_the_code_cannot_become_a_connection_has_its_own_cause_and_words() {
        let cases = [
            (JoinError::BadCode("x".into()), JoinCause::BadCode),
            (JoinError::RelaysOff, JoinCause::RelaysOff),
            (JoinError::HostNotVerifiable, JoinCause::HostNotVerifiable),
            (JoinError::NoIdentity, JoinCause::NoIdentity),
            (JoinError::AddressNotFound("h:1".into()), JoinCause::NameLookup),
            (JoinError::Relay(ResolveError::RelayNotFound("dns".into())), JoinCause::NameLookup),
            (JoinError::Relay(ResolveError::RelayDown("r:1".into())), JoinCause::RelayDown),
            (JoinError::Relay(ResolveError::NoAnswer("r:1".into())), JoinCause::RelayNoAnswer),
            (JoinError::Relay(ResolveError::CodeNotLive), JoinCause::CodeNotLive),
            (JoinError::Relay(ResolveError::Unreachable("u".into())), JoinCause::Network),
        ];
        for (e, cause) in cases {
            let f = failure_of(&e);
            assert_eq!(f.cause, cause, "{e:?}");
            assert_eq!(f.text, e.to_string());
        }
    }

    #[test]
    fn a_silent_host_is_worded_for_the_route_the_code_took() {
        let relayed = JoinTarget::parse("H3PQXR-K7Q2-MZ4P-WTXA").unwrap();
        let direct = JoinTarget::parse("203.0.113.9:27015").unwrap();
        let (a, b) = (host_silent(&relayed, 6.0), host_silent(&direct, 6.0));
        assert!(a.text.contains("relay knows that code") && a.text.contains("lobby"), "{}", a.text);
        assert!(b.text.contains("203.0.113.9:27015") && b.text.contains("firewall"), "{}", b.text);
        assert_eq!((a.cause, b.cause), (JoinCause::HostNoAnswer, JoinCause::HostNoAnswer));
    }

    #[test]
    fn the_client_is_described_with_its_release_when_it_has_one() {
        assert_eq!(describe_client(Some(4)), format!("release 4, protocol {PROTOCOL_VERSION}"));
        assert_eq!(describe_client(None), format!("protocol {PROTOCOL_VERSION}"));
    }

    #[test]
    fn a_task_with_no_map_fails_at_once_and_says_so_once() {
        let mut task = JoinTask::start(JoinPlan::new(JoinTarget::parse("127.0.0.1:9").unwrap(), None, "Me", Vec::new()), "protocol 17");
        let end = Instant::now() + Duration::from_secs(5);
        let failed = loop {
            if let Some(r) = task.poll() {
                break r;
            }
            assert!(Instant::now() < end, "the task never reported");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(failed.err().map(|f| f.cause), Some(JoinCause::Local));
        assert!(task.poll().is_none(), "the result is handed out once");
    }
}
