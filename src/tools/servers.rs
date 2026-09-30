//! `red_engine2 servers`: see, start, stop and restart the Red game servers on this machine (Linux, systemd user units).
//!
//! It is deliberately a thin layer over what already supervises a server. `deploy/game-host.sh` installs every hosted game as a per-user systemd unit whose `ExecStart` is
//! `red_server`; systemd already restarts it on failure, caps its memory and starts it at boot. So there is no registry and no process supervisor here: a **server** is a user
//! unit whose `ExecStart` is `red_server`, and the operations are `systemctl --user`. Provisioning (building the binary, the QUIC identity, the private join key) stays in
//! `game-host.sh`: this tool operates servers, it does not create them and it never reads or prints a secret.
//!
//! The logic is a pure function of a [`Backend`] (systemd, the process table, the journal), so the tests use a fake one and cannot touch a real server; the CLI in
//! `cli/servers.rs` only renders these structs (or their JSON), and a future GUI can do the same.
//!
//! * **Player counts** come from the server's own periodic `stats:` log line (`RED_STATS_SECS`), read from the journal, and carry their age; a count older than three
//!   intervals, or no line at all, is reported as *unknown*, never as zero.
//! * **Safety**: stopping or restarting a server that (freshly) reports players needs `--yes` ([`gate`]); `red_server` says goodbye to its clients on SIGTERM.
//! * **Unmanaged** `red_server` processes (no unit) are listed and can be stopped by pid, but not restarted: nothing knows how they were started.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::process::Command;
use std::time::Duration;

/// A server's default port when its unit does not set `RED_PORT`.
pub const DEFAULT_PORT: u16 = 27015;
/// How many stats intervals old a player count may be before it is called unknown.
const STALE_AFTER_INTERVALS: u64 = 3;
/// Used when the unit does not say how often the server prints stats.
const DEFAULT_STATS_SECS: u64 = 60;

/// What the tool asks of the machine. [`SystemdUser`] is the real one; tests use a fake.
pub trait Backend {
    /// The installed user service unit names (`great-outdoors.service`), templates excluded.
    fn unit_files(&self) -> Result<Vec<String>, String>;
    /// `systemctl --user show` output for `units`: `Key=Value` lines, one blank line between units.
    fn show(&self, units: &[String]) -> Result<String, String>;
    /// Runs `start`, `stop` or `restart` on a unit.
    fn control(&self, unit: &str, verb: &str) -> Result<(), String>;
    /// The last `lines` journal lines of a unit, with unix timestamps (`-o short-unix`).
    fn journal(&self, unit: &str, lines: usize) -> Result<String, String>;
    /// How many seconds process `pid` has been running.
    fn uptime_of(&self, pid: u32) -> Option<u64>;
    /// Every running process called `red_server`: `(pid, argv)`.
    fn red_server_processes(&self) -> Vec<(u32, Vec<String>)>;
    /// Sends SIGTERM to `pid`.
    fn terminate(&self, pid: u32) -> Result<(), String>;
    /// The current unix time, seconds.
    fn now_unix(&self) -> u64;
    /// Waits a little (the real one sleeps; a fake just counts).
    fn pause(&self, d: Duration);
}

/// The real backend: `systemctl --user`, `journalctl --user`, `/proc`, `ps` and `kill`.
pub struct SystemdUser;

fn run(cmd: &mut Command) -> Result<String, String> {
    let program = format!("{:?}", cmd.get_program());
    let out = cmd.output().map_err(|e| format!("cannot run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{program} failed: {}", err.trim().lines().next().unwrap_or("no message")))
    }
}

impl Backend for SystemdUser {
    fn unit_files(&self) -> Result<Vec<String>, String> {
        let text = run(Command::new("systemctl").args(["--user", "list-unit-files", "--type=service", "--no-legend", "--no-pager", "--plain"]))
            .map_err(|e| format!("{e} (this needs a systemd user session: Linux with `systemctl --user` working)"))?;
        Ok(text.lines().filter_map(|l| l.split_whitespace().next()).filter(|u| u.ends_with(".service") && !u.contains('@')).map(str::to_string).collect())
    }

    fn show(&self, units: &[String]) -> Result<String, String> {
        if units.is_empty() {
            return Ok(String::new());
        }
        let props = "Id,Description,ActiveState,SubState,MainPID,ExecStart,Environment,MemoryCurrent,NRestarts,UnitFileState,Result";
        run(Command::new("systemctl").arg("--user").arg("show").arg(format!("--property={props}")).args(units).arg("--no-pager"))
    }

    fn control(&self, unit: &str, verb: &str) -> Result<(), String> {
        run(Command::new("systemctl").args(["--user", verb, unit, "--no-pager"])).map(|_| ())
    }

    fn journal(&self, unit: &str, lines: usize) -> Result<String, String> {
        run(Command::new("journalctl").args(["--user", "-u", unit, "-n", &lines.to_string(), "--no-pager", "-o", "short-unix"]))
    }

    fn uptime_of(&self, pid: u32) -> Option<u64> {
        run(Command::new("ps").args(["-o", "etimes=", "-p", &pid.to_string()])).ok()?.trim().parse().ok()
    }

    fn red_server_processes(&self) -> Vec<(u32, Vec<String>)> {
        let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
        let mut out = Vec::new();
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if std::fs::read_to_string(e.path().join("comm")).map(|c| c.trim() == "red_server").unwrap_or(false) {
                let argv = std::fs::read(e.path().join("cmdline"))
                    .map(|b| b.split(|c| *c == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).to_string()).collect())
                    .unwrap_or_default();
                out.push((pid, argv));
            }
        }
        out.sort();
        out
    }

    fn terminate(&self, pid: u32) -> Result<(), String> {
        run(Command::new("kill").args(["-TERM", &pid.to_string()])).map(|_| ())
    }

    fn now_unix(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }

    fn pause(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// What the server's last `stats:` line said.
#[derive(Debug, Clone, PartialEq)]
pub struct Players {
    /// Connected players.
    pub count: u32,
    /// The match phase (`waiting`, `playing`, ...).
    pub phase: String,
    /// The worst tick in the last stats interval, microseconds.
    pub tick_worst_us: u64,
    /// How old the line is, seconds.
    pub age_secs: u64,
}

/// One game server: a systemd user unit that runs `red_server`.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerInfo {
    /// The unit name without `.service`: what you type.
    pub name: String,
    /// The unit, `name.service`.
    pub unit: String,
    /// What the unit is for.
    pub description: String,
    /// systemd's `ActiveState` (`active`, `inactive`, `failed`, ...).
    pub state: String,
    /// systemd's `SubState` (`running`, `dead`, ...).
    pub substate: String,
    /// Whether it starts at login/boot.
    pub enabled: bool,
    /// The main process, if running.
    pub pid: Option<u32>,
    /// How long that process has run, seconds.
    pub uptime_secs: Option<u64>,
    /// The UDP port it serves (`RED_PORT` or `--port`, else the default).
    pub port: u16,
    /// The map file it loads.
    pub map: Option<String>,
    /// Resident memory of the unit, bytes.
    pub memory_bytes: Option<u64>,
    /// Automatic restarts so far.
    pub restarts: Option<u32>,
    /// The binary.
    pub binary: String,
    /// Seconds between the server's stats lines (`RED_STATS_SECS`).
    pub stats_secs: Option<u64>,
    /// Players, from the journal, or `None` when unknown or stale.
    pub players: Option<Players>,
}

impl ServerInfo {
    /// Whether the unit is running.
    pub fn running(&self) -> bool {
        self.state == "active"
    }
}

/// A `red_server` process that no unit owns.
#[derive(Debug, Clone, PartialEq)]
pub struct Unmanaged {
    /// Its process id.
    pub pid: u32,
    /// How long it has run, seconds.
    pub uptime_secs: Option<u64>,
    /// Its command line, with secret-looking values masked.
    pub cmdline: String,
}

/// Everything found on this machine.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fleet {
    /// Servers run by systemd user units.
    pub servers: Vec<ServerInfo>,
    /// Stray `red_server` processes.
    pub unmanaged: Vec<Unmanaged>,
}

/// A variable or option name whose value must never be shown.
fn secret_name(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    ["KEY", "SECRET", "TOKEN", "PASSWORD", "PASSWD", "CREDENTIAL", "COOKIE"].iter().any(|s| n.contains(s))
}

/// Masks the value of every `NAME=value` word whose name looks secret (`RED_JOIN_KEY=abc` -> `RED_JOIN_KEY=***`), in any line of text.
pub fn redact(line: &str) -> String {
    line.split(' ')
        .map(|w| match w.split_once('=') {
            Some((name, _)) if secret_name(name.trim_start_matches('-')) => format!("{name}=***"),
            _ => w.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Splits systemd's `Environment=A=1 B="x y"` value into `(name, value)` pairs.
fn parse_environment(value: &str) -> Vec<(String, String)> {
    let (mut words, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in value.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.into_iter().filter_map(|w| w.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect()
}

/// `(binary path, argv)` from `ExecStart={ path=/x/red_server ; argv[]=/x/red_server --map m ; ignore_errors=no ; ... }`.
fn parse_exec_start(value: &str) -> Option<(String, Vec<String>)> {
    let path = value.split("path=").nth(1)?.split(" ;").next()?.trim().to_string();
    let argv = value.split("argv[]=").nth(1).map(|a| a.split(" ;").next().unwrap_or("").split_whitespace().map(str::to_string).collect()).unwrap_or_default();
    Some((path, argv))
}

fn is_red_server(path: &str) -> bool {
    path.rsplit('/').next() == Some("red_server")
}

/// Key/value blocks from `systemctl show` of several units.
fn parse_show(text: &str) -> Vec<BTreeMap<String, String>> {
    text.split("\n\n")
        .map(|block| block.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>())
        .filter(|m| !m.is_empty())
        .collect()
}

/// The newest `stats:` line in `journal` (`-o short-unix`: `1790798221.848 host red_server[1]: [52500.12s] stats: players 0 | phase waiting | tick avg 0 us worst 181 us | ...`).
pub fn parse_players(journal: &str, now_unix: u64, stats_secs: Option<u64>) -> Option<Players> {
    let line = journal.lines().rev().find(|l| l.contains("stats: players "))?;
    let ts: f64 = line.split_whitespace().next()?.parse().ok()?;
    let age_secs = now_unix.saturating_sub(ts as u64);
    if age_secs > stats_secs.unwrap_or(DEFAULT_STATS_SECS) * STALE_AFTER_INTERVALS {
        return None;
    }
    let stats = line.split("stats: ").nth(1)?;
    let field = |name: &str| stats.split('|').map(str::trim).find_map(|p| p.strip_prefix(name)).map(str::trim);
    let count = field("players")?.split_whitespace().next()?.parse().ok()?;
    let phase = field("phase").unwrap_or("?").to_string();
    let tick_worst_us =
        field("tick avg").and_then(|t| t.split("worst ").nth(1)).and_then(|w| w.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap_or(0);
    Some(Players { count, phase, tick_worst_us, age_secs })
}

/// Finds every game server and stray process. A unit is a server if its `ExecStart` is `red_server`.
pub fn discover(b: &dyn Backend) -> Result<Fleet, String> {
    let units = b.unit_files()?;
    let shown = parse_show(&b.show(&units)?);
    let mut servers = Vec::new();
    for m in shown {
        let Some((binary, argv)) = m.get("ExecStart").and_then(|e| parse_exec_start(e)) else { continue };
        if !is_red_server(&binary) {
            continue;
        }
        let unit = m.get("Id").cloned().unwrap_or_default();
        let env: BTreeMap<String, String> = parse_environment(m.get("Environment").map(String::as_str).unwrap_or("")).into_iter().collect();
        let arg_after = |flag: &str| argv.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone());
        let port = env.get("RED_PORT").and_then(|p| p.parse().ok()).or_else(|| arg_after("--port").and_then(|p| p.parse().ok())).unwrap_or(DEFAULT_PORT);
        let map = env.get("RED_MAP").cloned().or_else(|| arg_after("--map"));
        let stats_secs = env.get("RED_STATS_SECS").and_then(|s| s.parse().ok());
        let pid = m.get("MainPID").and_then(|p| p.parse::<u32>().ok()).filter(|p| *p != 0);
        let state = m.get("ActiveState").cloned().unwrap_or_default();
        let players = if state == "active" { b.journal(&unit, 400).ok().and_then(|j| parse_players(&j, b.now_unix(), stats_secs)) } else { None };
        servers.push(ServerInfo {
            name: unit.trim_end_matches(".service").to_string(),
            unit,
            description: m.get("Description").cloned().unwrap_or_default(),
            state,
            substate: m.get("SubState").cloned().unwrap_or_default(),
            enabled: m.get("UnitFileState").is_some_and(|s| s.starts_with("enabled")),
            pid,
            uptime_secs: pid.and_then(|p| b.uptime_of(p)),
            port,
            map,
            memory_bytes: m.get("MemoryCurrent").and_then(|v| v.parse().ok()),
            restarts: m.get("NRestarts").and_then(|v| v.parse().ok()),
            binary,
            stats_secs,
            players,
        });
    }
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    let owned: Vec<u32> = servers.iter().filter_map(|s| s.pid).collect();
    let unmanaged = b
        .red_server_processes()
        .into_iter()
        .filter(|(pid, _)| !owned.contains(pid))
        .map(|(pid, argv)| Unmanaged { pid, uptime_secs: b.uptime_of(pid), cmdline: redact(&argv.join(" ")) })
        .collect();
    Ok(Fleet { servers, unmanaged })
}

/// The server a user's word means: an exact name or unit, else a unique prefix.
pub fn resolve<'a>(servers: &'a [ServerInfo], word: &str) -> Result<&'a ServerInfo, String> {
    let w = word.trim_end_matches(".service");
    if let Some(s) = servers.iter().find(|s| s.name == w) {
        return Ok(s);
    }
    let hits: Vec<&ServerInfo> = servers.iter().filter(|s| s.name.starts_with(w)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no game server called `{word}` (servers: {})", names(servers))),
        many => Err(format!("`{word}` is ambiguous: {}", many.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

fn names(servers: &[ServerInfo]) -> String {
    if servers.is_empty() {
        "none found: a server is a systemd user unit whose ExecStart is red_server".to_string()
    } else {
        servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
    }
}

/// An operation on a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Turn it on.
    Start,
    /// Shut it down.
    Stop,
    /// Stop and start again.
    Restart,
}

impl Verb {
    /// The systemctl verb.
    pub fn word(self) -> &'static str {
        match self {
            Verb::Start => "start",
            Verb::Stop => "stop",
            Verb::Restart => "restart",
        }
    }
}

/// What the safety policy says about doing `verb` to `info`.
#[derive(Debug, Clone, PartialEq)]
pub enum Gate {
    /// Go ahead.
    Go,
    /// Go ahead, but say this first.
    Warn(String),
    /// Do not, unless told with `--yes`.
    Refuse(String),
    /// Nothing to do (already in the wanted state).
    Noop(String),
}

/// Stopping or restarting a server that freshly reports players needs `--yes`; an unknown player count is a warning, not a refusal (a server without `RED_STATS_SECS`
/// could otherwise never be stopped).
pub fn gate(info: &ServerInfo, verb: Verb, yes: bool) -> Gate {
    match (verb, info.running()) {
        (Verb::Start, true) => Gate::Noop(format!("{} is already running", info.name)),
        (Verb::Stop, false) => Gate::Noop(format!("{} is not running ({})", info.name, info.state)),
        (Verb::Start, false) | (Verb::Restart, false) => Gate::Go,
        (Verb::Stop | Verb::Restart, true) => match &info.players {
            Some(p) if p.count > 0 && !yes => Gate::Refuse(format!(
                "{} has {} player(s) connected (phase {}, as of {} s ago): pass --yes to {} it anyway",
                info.name,
                p.count,
                p.phase,
                p.age_secs,
                verb.word()
            )),
            Some(_) => Gate::Go,
            None => Gate::Warn(format!("{}: the player count is unknown (no recent stats line; set RED_STATS_SECS): continuing", info.name)),
        },
    }
}

/// The running server that already serves `info`'s UDP port, if any (a second `red_server` on one port fails to bind: better to say so than to start it and read the journal).
pub fn port_conflict<'a>(servers: &'a [ServerInfo], info: &ServerInfo) -> Option<&'a ServerInfo> {
    servers.iter().find(|s| s.unit != info.unit && s.running() && s.port == info.port)
}

/// Applies `verb` to `info` if the policy allows it (see [`gate`]); `servers` is everything on the machine, to catch two servers wanting one port. With `wait_secs`, `start`/`restart` return only once the unit is `active` (and, for `restart`, is a new process) or
/// fail when it is not by then. Returns the lines to show.
pub fn apply(b: &dyn Backend, servers: &[ServerInfo], info: &ServerInfo, verb: Verb, yes: bool, wait_secs: Option<u64>) -> Result<Vec<String>, String> {
    let mut say = Vec::new();
    match gate(info, verb, yes) {
        Gate::Refuse(why) => return Err(why),
        Gate::Noop(why) => return Ok(vec![why]),
        Gate::Warn(w) => say.push(w),
        Gate::Go => {}
    }
    if verb != Verb::Stop && !info.running() {
        if let Some(other) = port_conflict(servers, info) {
            return Err(format!(
                "{} cannot start: {} is already serving UDP port {} (stop it first, or give one of them another RED_PORT)",
                info.name, other.name, info.port
            ));
        }
    }
    b.control(&info.unit, verb.word())?;
    say.push(format!("{}: {} requested", info.name, verb.word()));
    if let (Some(secs), Verb::Start | Verb::Restart) = (wait_secs, verb) {
        let mut waited = 0u64;
        loop {
            let shown = parse_show(&b.show(std::slice::from_ref(&info.unit))?);
            let m = shown.first().cloned().unwrap_or_default();
            let state = m.get("ActiveState").map(String::as_str).unwrap_or("?");
            let pid = m.get("MainPID").and_then(|p| p.parse::<u32>().ok()).filter(|p| *p != 0);
            if state == "failed" {
                return Err(format!("{} failed to {} (result: {})", info.name, verb.word(), m.get("Result").map(String::as_str).unwrap_or("?")));
            }
            if state == "active" && (verb == Verb::Start || pid != info.pid) {
                say.push(format!("{}: active{}", info.name, pid.map(|p| format!(" (pid {p})")).unwrap_or_default()));
                return Ok(say);
            }
            if waited >= secs * 2 {
                return Err(format!("{} is still {state} after {secs} s", info.name));
            }
            b.pause(Duration::from_millis(500));
            waited += 1;
        }
    }
    Ok(say)
}

/// Stops an unmanaged `red_server` by pid (SIGTERM); the pid must be a stray one this tool found, never a managed server's (stop those by name) and never another program.
pub fn stop_unmanaged(b: &dyn Backend, fleet: &Fleet, pid: u32, yes: bool) -> Result<String, String> {
    let u = fleet
        .unmanaged
        .iter()
        .find(|u| u.pid == pid)
        .ok_or_else(|| format!("pid {pid} is not an unmanaged red_server (managed servers are stopped by name; others are not ours to stop)"))?;
    if !yes {
        return Err(format!("pid {pid} is an unmanaged red_server (`{}`): stopping it needs --yes", u.cmdline));
    }
    b.terminate(pid)?;
    Ok(format!("sent SIGTERM to red_server pid {pid}"))
}

/// Prints a unit's recent journal lines through `sink`, secret-looking values masked; with `follow`, keeps going until interrupted. (The real journal only: the tests cover
/// [`redact`], the part that could leak.)
pub fn log_lines(unit: &str, lines: usize, follow: bool, sink: &mut dyn FnMut(&str)) -> Result<(), String> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let mut cmd = Command::new("journalctl");
    cmd.args(["--user", "-u", unit, "-n", &lines.to_string(), "--no-pager", "-o", "short-iso"]);
    if follow {
        cmd.arg("--follow");
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("cannot run journalctl: {e}"))?;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            sink(&redact(&line));
        }
    }
    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}

/// `2d3h`, `5h07m`, `12m05s`, `42s`.
pub fn human_duration(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m{:02}s", secs / 60, secs % 60),
        3600..=86399 => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d{}h", secs / 86400, secs % 86400 / 3600),
    }
}

fn human_bytes(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.0} MB", b as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", b >> 10)
    }
}

/// The table `servers` prints.
pub fn render(f: &Fleet) -> String {
    if f.servers.is_empty() && f.unmanaged.is_empty() {
        return format!("no Red game servers found: {}\n", names(&f.servers));
    }
    let mut rows = vec![["NAME", "STATE", "UPTIME", "PLAYERS", "PORT", "MAP", "MEM", "BOOT"].map(String::from).to_vec()];
    for s in &f.servers {
        let players = match (&s.players, s.running()) {
            (Some(p), _) => format!("{} ({})", p.count, p.phase),
            (None, true) => "?".to_string(),
            (None, false) => "-".to_string(),
        };
        rows.push(vec![
            s.name.clone(),
            if s.running() { s.substate.clone() } else { s.state.clone() },
            s.uptime_secs.map(human_duration).unwrap_or_else(|| "-".into()),
            players,
            s.port.to_string(),
            s.map.as_deref().map(|m| m.rsplit('/').next().unwrap_or(m).to_string()).unwrap_or_else(|| "-".into()),
            s.memory_bytes.map(human_bytes).unwrap_or_else(|| "-".into()),
            if s.enabled { "on" } else { "off" }.to_string(),
        ]);
    }
    let widths: Vec<usize> = (0..8).map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for r in &rows {
        out.push_str(r.iter().enumerate().map(|(i, c)| format!("{c:<w$}", w = widths[i])).collect::<Vec<_>>().join("  ").trim_end());
        out.push('\n');
    }
    for u in &f.unmanaged {
        out.push_str(&format!("unmanaged red_server pid {} up {}: {}\n", u.pid, u.uptime_secs.map(human_duration).unwrap_or_else(|| "?".into()), u.cmdline));
    }
    out
}

/// The fleet as JSON.
pub fn to_json(f: &Fleet) -> Value {
    json!({
        "servers": f.servers.iter().map(|s| json!({
            "name": s.name, "unit": s.unit, "description": s.description, "state": s.state, "substate": s.substate, "enabled": s.enabled,
            "pid": s.pid, "uptime_secs": s.uptime_secs, "port": s.port, "map": s.map, "memory_bytes": s.memory_bytes, "restarts": s.restarts,
            "binary": s.binary, "stats_secs": s.stats_secs,
            "players": s.players.as_ref().map(|p| json!({"count": p.count, "phase": p.phase, "tick_worst_us": p.tick_worst_us, "age_secs": p.age_secs})),
        })).collect::<Vec<_>>(),
        "unmanaged": f.unmanaged.iter().map(|u| json!({"pid": u.pid, "uptime_secs": u.uptime_secs, "cmdline": u.cmdline})).collect::<Vec<_>>(),
    })
}

/// Everything about one server, as text.
pub fn render_status(s: &ServerInfo) -> String {
    let mut out = format!("{} ({})\n  {}\n", s.name, s.unit, s.description);
    out.push_str(&format!("  state     {} / {}{}\n", s.state, s.substate, if s.enabled { "  (starts at boot)" } else { "  (does not start at boot)" }));
    out.push_str(&format!(
        "  process   {}  up {}  restarts {}\n",
        s.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()),
        s.uptime_secs.map(human_duration).unwrap_or_else(|| "-".into()),
        s.restarts.map(|r| r.to_string()).unwrap_or_else(|| "-".into())
    ));
    out.push_str(&format!("  serves    UDP {}  map {}\n  binary    {}\n", s.port, s.map.as_deref().unwrap_or("-"), s.binary));
    out.push_str(&format!("  memory    {}\n", s.memory_bytes.map(human_bytes).unwrap_or_else(|| "-".into())));
    out.push_str(&match &s.players {
        Some(p) => format!("  players   {} (phase {}, worst tick {} us, as of {} s ago)\n", p.count, p.phase, p.tick_worst_us, p.age_secs),
        None => "  players   unknown (no recent stats line; the server prints one every RED_STATS_SECS)\n".to_string(),
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// A machine in a struct: nothing here can reach a real service.
    struct Fake {
        show_text: RefCell<String>,
        journal: RefCell<BTreeMap<String, String>>,
        procs: Vec<(u32, Vec<String>)>,
        calls: RefCell<Vec<String>>,
        now: u64,
        pauses: Cell<u32>,
        /// After this many `show` calls the unit reports active with a new pid.
        becomes_active_after: Cell<u32>,
        shows: Cell<u32>,
    }

    impl Fake {
        fn new(show_text: &str) -> Fake {
            Fake {
                show_text: RefCell::new(show_text.to_string()),
                journal: RefCell::new(BTreeMap::new()),
                procs: Vec::new(),
                calls: RefCell::new(Vec::new()),
                now: 1_000_000,
                pauses: Cell::new(0),
                becomes_active_after: Cell::new(u32::MAX),
                shows: Cell::new(0),
            }
        }
    }

    impl Backend for Fake {
        fn unit_files(&self) -> Result<Vec<String>, String> {
            Ok(parse_show(&self.show_text.borrow()).iter().filter_map(|m| m.get("Id").cloned()).collect())
        }
        fn show(&self, units: &[String]) -> Result<String, String> {
            self.shows.set(self.shows.get() + 1);
            if units.len() == 1 && self.becomes_active_after.get() != u32::MAX {
                let active = self.shows.get() > self.becomes_active_after.get();
                return Ok(format!(
                    "Id={}\nActiveState={}\nMainPID={}\nResult=success\n",
                    units[0],
                    if active { "active" } else { "activating" },
                    if active { 999 } else { 0 }
                ));
            }
            Ok(self.show_text.borrow().clone())
        }
        fn control(&self, unit: &str, verb: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("{verb} {unit}"));
            Ok(())
        }
        fn journal(&self, unit: &str, _lines: usize) -> Result<String, String> {
            Ok(self.journal.borrow().get(unit).cloned().unwrap_or_default())
        }
        fn uptime_of(&self, pid: u32) -> Option<u64> {
            Some(pid as u64)
        }
        fn red_server_processes(&self) -> Vec<(u32, Vec<String>)> {
            self.procs.clone()
        }
        fn terminate(&self, pid: u32) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("kill {pid}"));
            Ok(())
        }
        fn now_unix(&self) -> u64 {
            self.now
        }
        fn pause(&self, _d: Duration) {
            self.pauses.set(self.pauses.get() + 1);
        }
    }

    const SHOW: &str = "Id=great-outdoors.service
Description=Great Outdoors kart race server
ActiveState=active
SubState=running
MainPID=321
ExecStart={ path=/home/u/.local/share/great-outdoors/bin/red_server ; argv[]=/home/u/.local/share/great-outdoors/bin/red_server ; ignore_errors=no ; pid=321 }
Environment=RED_MAP=/home/u/maps/main.json RED_PORT=27015 RED_STATS_SECS=60 RED_JOIN_KEY=hunter2 RED_NAME=\"two words\"
MemoryCurrent=8261632
NRestarts=0
UnitFileState=enabled

Id=red-server.service
Description=Red server
ActiveState=inactive
SubState=dead
MainPID=0
ExecStart={ path=/opt/red/red_server ; argv[]=/opt/red/red_server --map /m/lab.json --port 28000 ; ignore_errors=no }
Environment=
MemoryCurrent=[not set]
NRestarts=2
UnitFileState=disabled

Id=gnome-shell.service
Description=GNOME
ActiveState=active
SubState=running
MainPID=77
ExecStart={ path=/usr/bin/gnome-shell ; argv[]=/usr/bin/gnome-shell ; ignore_errors=no }
Environment=
UnitFileState=enabled
";

    fn stats_line(ts: u64, players: u32) -> String {
        format!("{ts}.5 host red_server[321]: [100.00s] stats: players {players} | phase waiting | tick avg 4 us worst 181 us | promoted props 0 | in 0.0 KB/s out 0.0 KB/s | snapshots 9 | bad packets 0 bad tags 0\n")
    }

    fn fleet_with_players(players: u32, age: u64) -> (Fake, Fleet) {
        let f = Fake::new(SHOW);
        f.journal.borrow_mut().insert("great-outdoors.service".into(), format!("noise line\n{}", stats_line(f.now - age, players)));
        let fleet = discover(&f).unwrap();
        (f, fleet)
    }

    #[test]
    fn only_units_that_run_red_server_are_game_servers_and_their_details_come_from_systemd() {
        let (_, fleet) = fleet_with_players(0, 10);
        assert_eq!(fleet.servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["great-outdoors", "red-server"], "gnome-shell is not a game server");
        let go = &fleet.servers[0];
        assert_eq!((go.state.as_str(), go.substate.as_str(), go.enabled, go.pid, go.restarts), ("active", "running", true, Some(321), Some(0)));
        assert_eq!((go.port, go.map.as_deref(), go.memory_bytes, go.uptime_secs), (27015, Some("/home/u/maps/main.json"), Some(8261632), Some(321)));
        let rs = &fleet.servers[1];
        assert_eq!(
            (rs.state.as_str(), rs.enabled, rs.pid, rs.port, rs.map.as_deref(), rs.memory_bytes, rs.restarts),
            ("inactive", false, None, 28000, Some("/m/lab.json"), None, Some(2)),
            "port and map fall back to the command line"
        );
        assert!(rs.players.is_none());
    }

    #[test]
    fn players_come_from_the_newest_stats_line_with_its_age_and_go_unknown_when_stale() {
        let (_, fleet) = fleet_with_players(3, 20);
        assert_eq!(fleet.servers[0].players, Some(Players { count: 3, phase: "waiting".into(), tick_worst_us: 181, age_secs: 20 }));
        let (_, stale) = fleet_with_players(3, 200);
        assert_eq!(stale.servers[0].players, None, "older than three intervals (3 x 60 s) is unknown, not a number");
        assert_eq!(parse_players("no stats here\n", 1000, None), None);
        let two = format!("{}{}", stats_line(900, 5), stats_line(990, 1));
        assert_eq!(parse_players(&two, 1000, Some(60)).map(|p| (p.count, p.age_secs)), Some((1, 10)), "the last line wins");
    }

    #[test]
    fn secrets_are_masked_in_every_line_the_tool_prints() {
        assert_eq!(redact("RED_JOIN_KEY=hunter2 RED_PORT=27015 --tls-key=/p/k.pem x"), "RED_JOIN_KEY=*** RED_PORT=27015 --tls-key=*** x");
        let env = parse_environment("RED_MAP=/m RED_NAME=\"two words\" RED_JOIN_KEY=hunter2");
        assert_eq!(env[1], ("RED_NAME".to_string(), "two words".to_string()), "quoted values stay whole");
        let mut f2 = Fake::new(SHOW);
        f2.procs = vec![(500, vec!["/x/red_server".into(), "--join-key=abc123".into(), "--port".into(), "1".into()])];
        let fleet = discover(&f2).unwrap();
        assert_eq!(fleet.unmanaged[0].cmdline, "/x/red_server --join-key=*** --port 1");
        assert!(!render(&fleet).contains("abc123") && !serde_json::to_string(&to_json(&fleet)).unwrap().contains("abc123"));
        let shown = discover(&Fake::new(SHOW)).unwrap();
        assert!(
            !serde_json::to_string(&to_json(&shown)).unwrap().contains("hunter2"),
            "a secret in a unit's Environment never reaches the output (only port, map and stats are read)"
        );
    }

    #[test]
    fn a_stray_red_server_is_unmanaged_only_if_no_unit_owns_its_pid() {
        let mut f = Fake::new(SHOW);
        f.procs = vec![(321, vec!["/x/red_server".into()]), (500, vec!["/y/red_server".into()])];
        let fleet = discover(&f).unwrap();
        assert_eq!(fleet.unmanaged.iter().map(|u| u.pid).collect::<Vec<_>>(), [500], "pid 321 belongs to great-outdoors.service");
    }

    #[test]
    fn names_resolve_exactly_or_by_unique_prefix_and_say_what_is_wrong_otherwise() {
        let (_, fleet) = fleet_with_players(0, 5);
        assert_eq!(resolve(&fleet.servers, "great-outdoors").unwrap().name, "great-outdoors");
        assert_eq!(resolve(&fleet.servers, "great-outdoors.service").unwrap().name, "great-outdoors");
        assert_eq!(resolve(&fleet.servers, "gr").unwrap().name, "great-outdoors");
        assert!(resolve(&fleet.servers, "nope").unwrap_err().contains("servers: great-outdoors, red-server"));
        let mut twins = fleet.servers.clone();
        twins.push(ServerInfo { name: "great-lakes".into(), ..twins[0].clone() });
        assert!(resolve(&twins, "great").unwrap_err().contains("ambiguous"));
        assert!(resolve(&[], "x").unwrap_err().contains("ExecStart is red_server"));
    }

    #[test]
    fn stopping_or_restarting_a_server_with_players_needs_yes_and_an_unknown_count_only_warns() {
        let (_, busy) = fleet_with_players(3, 10);
        let go = &busy.servers[0];
        for verb in [Verb::Stop, Verb::Restart] {
            let Gate::Refuse(why) = gate(go, verb, false) else { panic!("{verb:?} must refuse") };
            assert!(why.contains("3 player(s)") && why.contains("--yes"), "{why}");
            assert_eq!(gate(go, verb, true), Gate::Go);
        }
        let (_, empty) = fleet_with_players(0, 10);
        assert_eq!(gate(&empty.servers[0], Verb::Stop, false), Gate::Go, "nobody connected");
        let (_, stale) = fleet_with_players(3, 500);
        assert!(matches!(gate(&stale.servers[0], Verb::Stop, false), Gate::Warn(_)), "unknown is a warning, so a server without stats can still be stopped");
        let off = &busy.servers[1];
        assert!(matches!(gate(off, Verb::Stop, false), Gate::Noop(_)) && gate(off, Verb::Start, false) == Gate::Go);
        assert!(matches!(gate(go, Verb::Start, false), Gate::Noop(_)), "starting a running server does nothing");
    }

    #[test]
    fn apply_calls_systemctl_only_when_the_policy_allows_and_waits_for_a_new_active_process() {
        let (f, busy) = fleet_with_players(2, 10);
        let go = &busy.servers[0];
        assert!(apply(&f, &busy.servers, go, Verb::Stop, false, None).unwrap_err().contains("--yes"));
        assert!(f.calls.borrow().is_empty(), "a refusal never reaches systemctl");
        assert_eq!(apply(&f, &busy.servers, go, Verb::Start, false, None).unwrap(), vec!["great-outdoors is already running".to_string()]);
        assert!(f.calls.borrow().is_empty());
        let said = apply(&f, &busy.servers, &busy.servers[1], Verb::Start, false, None).unwrap();
        assert_eq!(f.calls.borrow().as_slice(), ["start red-server.service"]);
        assert!(said[0].contains("start requested"));
        // --wait: the unit goes activating -> active with a new pid; the wait polls on the fake clock.
        f.becomes_active_after.set(3);
        let said = apply(&f, &busy.servers, go, Verb::Restart, true, Some(10)).unwrap();
        assert!(said.iter().any(|l| l.contains("active (pid 999)")), "{said:?}");
        assert!(f.pauses.get() >= 1, "it really waited");
        // A unit that never comes up fails the wait instead of hanging.
        let g = Fake::new(SHOW);
        g.becomes_active_after.set(u32::MAX - 1);
        let (g2, fl) = (g, discover(&Fake::new(SHOW)).unwrap());
        let err = apply(&g2, &fl.servers, &fl.servers[1], Verb::Start, false, Some(1)).unwrap_err();
        assert!(err.contains("still") || err.contains("failed"), "{err}");
    }

    #[test]
    fn starting_a_server_on_a_port_another_running_server_uses_is_refused_before_systemd_is_asked() {
        let (f, fleet) = fleet_with_players(0, 5);
        // red-server (inactive, port 28000) is moved onto great-outdoors' port 27015.
        let mut servers = fleet.servers.clone();
        servers[1].port = 27015;
        let err = apply(&f, &servers, &servers[1], Verb::Start, false, None).unwrap_err();
        assert!(err.contains("great-outdoors is already serving UDP port 27015"), "{err}");
        assert!(f.calls.borrow().is_empty(), "systemctl was never called");
        assert!(port_conflict(&fleet.servers, &fleet.servers[1]).is_none(), "distinct ports do not conflict");
        assert!(port_conflict(&servers, &servers[0]).is_none(), "a server does not conflict with itself, and the stopped one holds no port");
        // Stopping is never blocked by a port, and starting after the other one is stopped is fine.
        servers[0].state = "inactive".into();
        assert!(apply(&f, &servers, &servers[1], Verb::Start, false, None).is_ok());
    }

    #[test]
    fn an_unmanaged_process_is_stopped_only_by_its_pid_with_yes_and_never_a_managed_or_foreign_one() {
        let mut f = Fake::new(SHOW);
        f.procs = vec![(500, vec!["/y/red_server".into()])];
        let fleet = discover(&f).unwrap();
        assert!(stop_unmanaged(&f, &fleet, 500, false).unwrap_err().contains("--yes"));
        assert!(stop_unmanaged(&f, &fleet, 321, true).unwrap_err().contains("not an unmanaged red_server"), "a managed server is stopped by name");
        assert!(stop_unmanaged(&f, &fleet, 1, true).is_err(), "an arbitrary pid is not ours");
        assert!(f.calls.borrow().is_empty());
        assert_eq!(stop_unmanaged(&f, &fleet, 500, true).unwrap(), "sent SIGTERM to red_server pid 500");
        assert_eq!(f.calls.borrow().as_slice(), ["kill 500"]);
    }

    #[test]
    fn the_table_and_json_show_the_same_facts() {
        let (_, fleet) = fleet_with_players(2, 15);
        let table = render(&fleet);
        let lines: Vec<&str> = table.lines().collect();
        assert!(lines[0].starts_with("NAME") && lines[0].contains("PLAYERS"), "{table}");
        assert!(
            lines[1].starts_with("great-outdoors")
                && lines[1].contains("running")
                && lines[1].contains("2 (waiting)")
                && lines[1].contains("27015")
                && lines[1].contains("main.json")
                && lines[1].contains("8 MB")
                && lines[1].ends_with("on"),
            "{table}"
        );
        assert!(lines[2].starts_with("red-server") && lines[2].contains("inactive") && lines[2].contains("28000") && lines[2].ends_with("off"), "{table}");
        let j = to_json(&fleet);
        assert_eq!(j["servers"][0]["players"]["count"], 2);
        assert_eq!(j["servers"][1]["state"], "inactive");
        assert!(j["servers"][1]["players"].is_null());
        assert_eq!(human_duration(42), "42s");
        assert_eq!(human_duration(725), "12m05s");
        assert_eq!(human_duration(3 * 3600 + 7 * 60), "3h07m");
        assert_eq!(human_duration(2 * 86400 + 3 * 3600), "2d3h");
        assert!(render(&Fleet::default()).contains("no Red game servers found"));
        let st = render_status(&fleet.servers[0]);
        assert!(st.contains("UDP 27015") && st.contains("players   2") && st.contains("starts at boot"), "{st}");
    }
}
