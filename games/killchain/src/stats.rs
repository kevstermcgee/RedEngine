//! A player's lifetime statistics, kept on their own machine between sessions (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! The game counts what happened to *this* player (kills, deaths, headshots, shots, rounds, time) and writes one small JSON file in the
//! user's data folder. Pure data and file handling: no window, no GPU. A missing or damaged file starts a fresh record (the damaged one is
//! kept beside it as `stats.json.bad`), and every write goes through a temporary file so a crash never leaves half a record.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A game the player joined before, so JOIN can offer it again (ADR 2026-10-20-join-by-short-code-survives-a-wrong-map-and-a-slow-relay).
///
/// The code is kept exactly as it worked, **join key included**: that is what makes one click enough for a server whose code is a hundred and fifty
/// characters. It lives in this player's own data folder, like everything else here, and nowhere else: it is never in the game's files, never sent
/// anywhere, and deleting `stats.json` forgets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RecentJoin {
    /// The code, as `JoinTarget::parse` reads it back.
    pub code: String,
    /// The file name of the map the host turned out to play (`quarry.json`), tried first next time.
    pub map: String,
}

impl RecentJoin {
    /// A few characters that say which game this is and give nothing away: the relay's six characters, or `HOST:PORT` of a direct code. Never the
    /// identity or the key.
    pub fn label(&self) -> String {
        use red_engine2::net::join::JoinTarget;
        match JoinTarget::parse(&self.code) {
            Ok(JoinTarget::Relay(short)) => red_engine2::net::relay::code_to_string(&short.code),
            Ok(JoinTarget::Direct(direct)) => direct.address,
            Err(_) => "A SAVED GAME".to_string(),
        }
    }

    /// What REJOIN puts in the JOIN screen's code field: the [`label`](Self::label), never the code. The field draws what it holds (the end of it, when
    /// it is long), and the code carries the key; [`Stats::code_to_join`] reads the label back as this code.
    pub fn field_text(&self) -> String {
        self.label()
    }
}

/// How many games JOIN remembers.
pub const RECENT_JOINS: usize = 3;

/// Everything remembered about a player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Stats {
    /// Layout of this file (for future changes).
    pub version: u32,
    /// The name the player last used.
    pub name: String,
    /// Unix seconds of the first time the game was played (`0` = never).
    pub first_played: u64,
    /// Unix seconds of the most recent session.
    pub last_played: u64,
    /// Times the game was started.
    pub sessions: u64,
    /// Kills scored.
    pub kills: u64,
    /// Times killed.
    pub deaths: u64,
    /// Kills that were headshots.
    pub headshots: u64,
    /// Bullets, rockets, grenades and swings started.
    pub shots_fired: u64,
    /// Attacks that damaged somebody.
    pub shots_hit: u64,
    /// Matches finished (a match that ended with the player in it).
    pub rounds_played: u64,
    /// Matches the player's team won.
    pub rounds_won: u64,
    /// Matches the player's team lost.
    pub rounds_lost: u64,
    /// Matches that ended level.
    pub rounds_drawn: u64,
    /// Seconds spent alive or dead inside a running match.
    pub time_in_matches_secs: f64,
    /// Seconds the game was open.
    pub time_in_game_secs: f64,
    /// Most kills in a row without dying.
    pub best_streak: u32,
    /// Most kills in one match.
    pub best_match_kills: u32,
    /// Kills by weapon name.
    pub weapon_kills: BTreeMap<String, u64>,
    /// Kills made with a melee weapon.
    pub melee_kills: u64,
    /// Kills made with a grenade, rocket or other explosive.
    pub explosive_kills: u64,
    /// The games joined before, newest first (at most [`RECENT_JOINS`]), for JOIN to offer again.
    pub recent_joins: Vec<RecentJoin>,
}

/// A derived, displayable line of the stats screen.
#[derive(Debug, Clone, PartialEq)]
pub struct StatLine {
    /// What it is.
    pub label: String,
    /// Its value, ready to print.
    pub value: String,
}

impl Stats {
    /// The layout version this code writes.
    pub const VERSION: u32 = 1;

    /// Kills per death (kills when there are no deaths yet).
    pub fn kd(&self) -> f64 {
        if self.deaths == 0 {
            self.kills as f64
        } else {
            self.kills as f64 / self.deaths as f64
        }
    }

    /// Share of attacks that hit, 0..1 (0 before any shot).
    pub fn accuracy(&self) -> f64 {
        if self.shots_fired == 0 {
            0.0
        } else {
            (self.shots_hit as f64 / self.shots_fired as f64).min(1.0)
        }
    }

    /// Share of kills that were headshots, 0..1.
    pub fn headshot_rate(&self) -> f64 {
        if self.kills == 0 {
            0.0
        } else {
            (self.headshots as f64 / self.kills as f64).min(1.0)
        }
    }

    /// Share of finished matches won, 0..1.
    pub fn win_rate(&self) -> f64 {
        if self.rounds_played == 0 {
            0.0
        } else {
            self.rounds_won as f64 / self.rounds_played as f64
        }
    }

    /// The weapon with the most kills, if any kill was made.
    pub fn favourite_weapon(&self) -> Option<(&str, u64)> {
        self.weapon_kills.iter().max_by_key(|(name, n)| (**n, std::cmp::Reverse((*name).clone()))).map(|(name, n)| (name.as_str(), *n))
    }

    /// Records one kill with `weapon` (by name; `melee` / `explosive` say what kind it was).
    pub fn add_kill(&mut self, weapon: &str, headshot: bool, melee: bool, explosive: bool) {
        self.kills += 1;
        self.headshots += u64::from(headshot);
        self.melee_kills += u64::from(melee);
        self.explosive_kills += u64::from(explosive);
        *self.weapon_kills.entry(weapon.to_string()).or_insert(0) += 1;
    }

    /// Remembers a join that worked: `code` (as `JoinTarget::parse` reads it) goes first, an older entry for the same code is replaced, and only the
    /// newest [`RECENT_JOINS`] are kept. `map` is the file name of the map the host plays.
    pub fn remember_join(&mut self, code: &str, map: &str) {
        let code = code.trim();
        if code.is_empty() {
            return;
        }
        self.recent_joins.retain(|r| r.code != code);
        self.recent_joins.insert(0, RecentJoin { code: code.to_string(), map: map.to_string() });
        self.recent_joins.truncate(RECENT_JOINS);
    }

    /// The code JOIN connects with for what the code field holds (`field`): the saved code of the newest remembered game whose
    /// [`label`](RecentJoin::label) it is (what REJOIN puts there; typed by hand it works too, in any case), otherwise the field as it is.
    pub fn code_to_join(&self, field: &str) -> String {
        let typed = field.trim();
        self.recent_joins.iter().find(|r| !typed.is_empty() && r.label().eq_ignore_ascii_case(typed)).map_or_else(|| field.to_string(), |r| r.code.clone())
    }

    /// The lines the stats screen shows, most interesting first.
    pub fn lines(&self) -> Vec<StatLine> {
        let line = |label: &str, value: String| StatLine { label: label.to_string(), value };
        let mut out = vec![
            line("KILLS", self.kills.to_string()),
            line("DEATHS", self.deaths.to_string()),
            line("K/D RATIO", format!("{:.2}", self.kd())),
            line("HEADSHOTS", format!("{} ({:.0}%)", self.headshots, self.headshot_rate() * 100.0)),
            line("ACCURACY", format!("{:.0}%", self.accuracy() * 100.0)),
            line("MATCHES PLAYED", self.rounds_played.to_string()),
            line("WON / LOST / DRAWN", format!("{} / {} / {}", self.rounds_won, self.rounds_lost, self.rounds_drawn)),
            line("WIN RATE", format!("{:.0}%", self.win_rate() * 100.0)),
            line("TIME IN MATCHES", format_duration(self.time_in_matches_secs)),
            line("TIME IN GAME", format_duration(self.time_in_game_secs)),
            line("BEST KILL STREAK", self.best_streak.to_string()),
            line("MOST KILLS IN A MATCH", self.best_match_kills.to_string()),
            line("KNIFE AND MELEE KILLS", self.melee_kills.to_string()),
            line("EXPLOSIVE KILLS", self.explosive_kills.to_string()),
        ];
        out.push(match self.favourite_weapon() {
            Some((name, n)) => line("FAVOURITE WEAPON", format!("{} ({n})", name.to_uppercase())),
            None => line("FAVOURITE WEAPON", "NONE YET".to_string()),
        });
        out.push(line("SESSIONS", self.sessions.to_string()));
        out
    }
}

/// `3h 07m`, `12m 05s`, `45s`.
pub fn format_duration(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {sec:02}s")
    } else {
        format!("{sec}s")
    }
}

/// Seconds since the Unix epoch (0 if the clock is before it).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Where the record lives: `%APPDATA%\Killchain` on Windows, `$XDG_DATA_HOME/killchain` or `~/.local/share/killchain` elsewhere. `KILLCHAIN_DATA` overrides.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("KILLCHAIN_DATA").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|d| Path::new(&d).join("Killchain"));
    }
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(Path::new(&x).join("killchain"));
    }
    std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share/killchain"))
}

/// A stats file on disk.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// The store at `path`.
    pub fn at(path: impl Into<PathBuf>) -> Store {
        Store { path: path.into() }
    }

    /// The store in the user's data folder, if the machine has one.
    pub fn default_location() -> Option<Store> {
        data_dir().map(|d| Store::at(d.join("stats.json")))
    }

    /// Where the file is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the record. A missing file is a fresh record; a damaged one is set aside as `.bad` and also starts fresh.
    pub fn load(&self) -> Stats {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return Stats { version: Stats::VERSION, ..Default::default() } };
        match serde_json::from_str::<Stats>(&text) {
            Ok(mut s) => {
                s.version = Stats::VERSION;
                s
            }
            Err(_) => {
                let _ = std::fs::rename(&self.path, self.path.with_extension("json.bad"));
                Stats { version: Stats::VERSION, ..Default::default() }
            }
        }
    }

    /// Writes the record (through a temporary file, so an interrupted write keeps the old one).
    pub fn save(&self, stats: &Stats) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(stats).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("re2_stats_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_record_survives_a_restart() {
        let dir = temp("roundtrip");
        let store = Store::at(dir.join("stats.json"));
        assert_eq!(store.load().kills, 0, "no file: a fresh record");
        let mut s = store.load();
        s.add_kill("Redline rifle", true, false, false);
        s.add_kill("combat knife", false, true, false);
        s.deaths = 1;
        s.rounds_played = 1;
        s.rounds_won = 1;
        s.time_in_game_secs = 3725.0;
        s.best_streak = 2;
        store.save(&s).unwrap();
        let back = Store::at(dir.join("stats.json")).load();
        assert_eq!(back, Stats { version: Stats::VERSION, ..s });
        assert_eq!((back.kills, back.headshots, back.melee_kills), (2, 1, 1));
        assert!(!dir.join("stats.json.tmp").exists(), "the temporary file is gone");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_file_is_kept_aside_and_a_fresh_record_starts() {
        let dir = temp("damaged");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stats.json"), "{ not json").unwrap();
        let store = Store::at(dir.join("stats.json"));
        assert_eq!(store.load().kills, 0);
        assert!(dir.join("stats.json.bad").exists(), "the damaged file is not thrown away");
        store.save(&Stats::default()).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let dir = temp("fields");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stats.json"), r#"{"kills": 7, "from_the_future": true}"#).unwrap();
        let s = Store::at(dir.join("stats.json")).load();
        assert_eq!((s.kills, s.deaths), (7, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_ratios_and_the_favourite_are_derived() {
        let mut s = Stats::default();
        assert_eq!((s.kd(), s.accuracy(), s.win_rate(), s.favourite_weapon()), (0.0, 0.0, 0.0, None));
        for _ in 0..3 {
            s.add_kill("Rook carbine", false, false, false);
        }
        s.add_kill("Frag grenade", false, false, true);
        s.deaths = 2;
        s.shots_fired = 40;
        s.shots_hit = 10;
        assert_eq!(s.kd(), 2.0);
        assert_eq!(s.accuracy(), 0.25);
        assert_eq!(s.favourite_weapon(), Some(("Rook carbine", 3)));
        assert_eq!(s.explosive_kills, 1);
        let lines = s.lines();
        assert!(lines.iter().any(|l| l.label == "K/D RATIO" && l.value == "2.00"));
        assert!(lines.iter().any(|l| l.label == "FAVOURITE WEAPON" && l.value.contains("ROOK CARBINE")));
        assert!(lines.len() >= 12, "useful stats, not three numbers");
    }

    #[test]
    fn a_join_that_worked_is_offered_first_next_time_and_only_the_newest_are_kept() {
        let mut s = Stats::default();
        let relay = "H3PQXR-K7Q2-MZ4P-WTXA";
        let direct = format!("red-engine.duckdns.org:27015#sha256:{}#K7Q2MZ4P", "ab".repeat(32));
        s.remember_join(&direct, "main.json");
        s.remember_join(relay, "quarry.json");
        assert_eq!(s.recent_joins.iter().map(|r| r.map.as_str()).collect::<Vec<_>>(), ["quarry.json", "main.json"], "newest first");
        // The same game again moves up and keeps one entry (and the map that worked this time).
        s.remember_join(&direct, "terminus.json");
        assert_eq!(s.recent_joins.len(), 2);
        assert_eq!((s.recent_joins[0].code.as_str(), s.recent_joins[0].map.as_str()), (direct.as_str(), "terminus.json"));
        for n in 0..5 {
            s.remember_join(&format!("{relay}{n}"), "main.json");
        }
        assert_eq!(s.recent_joins.len(), RECENT_JOINS);
        s.remember_join("   ", "main.json");
        assert_eq!(s.recent_joins.len(), RECENT_JOINS, "nothing to remember in a blank code");
    }

    #[test]
    fn a_remembered_game_is_labelled_without_giving_its_key_away() {
        let key = "K7Q2MZ4PWTXA";
        let direct = RecentJoin { code: format!("red-engine.duckdns.org:27015#sha256:{}#{key}", "ab".repeat(32)), map: String::new() };
        let relay = RecentJoin { code: format!("H3PQXR-{key}"), map: String::new() };
        assert_eq!(direct.label(), "red-engine.duckdns.org:27015");
        assert_eq!(relay.label(), "H3PQXR");
        for r in [&direct, &relay] {
            assert!(!r.label().contains(key) && !r.label().contains("sha256"), "{}", r.label());
        }
        assert_eq!(RecentJoin { code: "not a code".into(), map: String::new() }.label(), "A SAVED GAME");
    }

    #[test]
    fn remembered_games_survive_a_restart_and_an_older_file_without_them_loads() {
        let dir = temp("recent");
        let store = Store::at(dir.join("stats.json"));
        let mut s = store.load();
        s.remember_join("H3PQXR-K7Q2-MZ4P-WTXA", "quarry.json");
        store.save(&s).unwrap();
        assert_eq!(Store::at(dir.join("stats.json")).load().recent_joins, s.recent_joins);
        std::fs::write(dir.join("stats.json"), r#"{"kills": 3}"#).unwrap();
        let old = store.load();
        assert_eq!((old.kills, old.recent_joins.len()), (3, 0));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(45.0), "45s");
        assert_eq!(format_duration(725.0), "12m 05s");
        assert_eq!(format_duration(11_220.0), "3h 07m");
    }
}
