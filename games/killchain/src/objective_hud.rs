//! What Killchain's HUD says about the objective modes (ADR 2026-10-07-killchain-game-modes-free-for-all-capture-the-flag): the status lines for capture the flag and search
//! and destroy, the one-line banners for each event, and the markers drawn over flags, the bomb and the sites.
//!
//! Pure functions of the newest [`ObjSnap`] and a little context, so every sentence is tested without a window.

use red_engine2::net::protocol::ObjSnap;
use red_engine2::sim::objective::{ev, WinHow};

/// Green: good news for our team.
pub const GOOD: [u8; 4] = [150, 232, 120, 255];
/// Red: bad news.
pub const BAD: [u8; 4] = [255, 112, 92, 255];
/// Amber: something to do.
pub const CALL: [u8; 4] = [255, 226, 160, 255];

/// What the status lines under the score say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ObjectiveHud {
    /// The first line (round and sides, or the two flags).
    pub line: String,
    /// The second line: what to do now.
    pub sub: String,
    /// The second line's colour.
    pub sub_color: [u8; 4],
    /// A progress bar (what is being done, 0 to 1): planting or defusing.
    pub bar: Option<(String, f32)>,
}

/// A label over something in the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    /// Where on the screen, in the HUD's pixels.
    pub x: f32,
    /// Where on the screen, in the HUD's pixels.
    pub y: f32,
    /// The text.
    pub label: String,
    /// Its colour.
    pub color: [u8; 4],
}

/// Everything [`objective_hud`] needs.
pub struct ObjectiveInput<'a> {
    /// The mode's wire byte (`2` capture the flag, `3` search and destroy).
    pub mode: u8,
    /// Our team (`1` or `2`) and our player id.
    pub my_team: u8,
    /// Our player id.
    pub me: u8,
    /// The newest objective snapshot.
    pub obj: &'a ObjSnap,
    /// Players still alive on each team (index = team - 1).
    pub alive: [u8; 2],
    /// The names of the bomb sites, in order.
    pub sites: &'a [char],
}

fn clock(ticks: u16) -> String {
    let s = ticks.div_ceil(60);
    format!("{}:{:02}", s / 60, s % 60)
}

/// The two status lines and the progress bar for the current mode, `None` outside the objective modes.
pub fn objective_hud(i: &ObjectiveInput) -> Option<ObjectiveHud> {
    let o = i.obj;
    let mine = i.my_team.clamp(1, 2) as usize - 1;
    match (i.mode, o.kind) {
        (2, 1) => {
            let (own, enemy) = (o.flags[mine], o.flags[1 - mine]);
            let status = |f: &red_engine2::net::protocol::FlagSnap| match f.state {
                0 => "HOME".to_string(),
                1 => "TAKEN".to_string(),
                _ => format!("DOWN {}", f.left_ticks.div_ceil(60)),
            };
            let line = format!("YOUR FLAG {}   ENEMY FLAG {}", status(&own), status(&enemy));
            let (sub, sub_color) = if enemy.state == 1 && enemy.carrier == i.me {
                ("YOU HAVE THE FLAG - BRING IT HOME".to_string(), CALL)
            } else if own.state == 1 {
                ("THEY HAVE YOUR FLAG - STOP THE CARRIER".to_string(), BAD)
            } else if enemy.state == 1 {
                ("ESCORT YOUR CARRIER".to_string(), GOOD)
            } else if own.state == 2 {
                ("RETURN YOUR FLAG".to_string(), CALL)
            } else {
                (String::new(), CALL)
            };
            Some(ObjectiveHud { line, sub, sub_color, bar: None })
        }
        (3, 2) => {
            let s = o.snd;
            let attacking = s.attackers == i.my_team;
            let (my_alive, their_alive) = (i.alive[mine], i.alive[1 - mine]);
            // The round's own clock while it runs (it stops mattering once the bomb is planted: the fuse takes over).
            let round_clock = if s.phase == 1 && s.bomb < 2 { format!("   {}", clock(s.left_ticks)) } else { String::new() };
            let line = format!("ROUND {}   {}   {} V {}{round_clock}", s.round, if attacking { "ATTACKING" } else { "DEFENDING" }, my_alive, their_alive);
            let site = |idx: u8| i.sites.get(idx as usize).copied().unwrap_or('A');
            let (sub, sub_color) = match s.phase {
                0 => ("GET READY".to_string(), CALL),
                2 => (if s.winner == i.my_team { "ROUND WON".to_string() } else { "ROUND LOST".to_string() }, if s.winner == i.my_team { GOOD } else { BAD }),
                _ => match (s.bomb, attacking) {
                    (0, true) if s.carrier == i.me => ("YOU HAVE THE BOMB - HOLD E ON A SITE".to_string(), CALL),
                    (0, true) => ("ESCORT THE BOMB".to_string(), GOOD),
                    (1, true) => ("PICK UP THE BOMB".to_string(), CALL),
                    (2, true) => (format!("BOMB PLANTED AT {}   {}   DEFEND IT", site(s.site), clock(s.fuse_ticks)), GOOD),
                    (2, false) => (format!("DEFUSE THE BOMB AT {}   {}", site(s.site), clock(s.fuse_ticks)), BAD),
                    (_, false) => ("STOP THE BOMB BEING PLANTED".to_string(), CALL),
                    _ => (String::new(), CALL),
                },
            };
            let bar = (s.progress > 0 && s.phase == 1).then(|| (if s.bomb == 2 { "DEFUSING" } else { "PLANTING" }.to_string(), s.progress as f32 / 255.0));
            Some(ObjectiveHud { line, sub, sub_color, bar })
        }
        _ => None,
    }
}

/// The banner for one objective event, as seen by a player of `my_team` (`me` is their id). `None` for events not worth a banner.
pub fn event_banner(kind: u8, team: u8, slot: u8, my_team: u8, me: u8) -> Option<(String, [u8; 4])> {
    let ours = team == my_team;
    Some(match kind {
        // `team` is the flag's team: a flag of ours taken is bad, theirs taken is good.
        ev::FLAG_TAKEN if slot == me => ("YOU TOOK THE FLAG".to_string(), GOOD),
        ev::FLAG_TAKEN if ours => ("ENEMY HAS YOUR FLAG".to_string(), BAD),
        ev::FLAG_TAKEN => ("YOUR TEAM TOOK THE FLAG".to_string(), GOOD),
        ev::FLAG_DROPPED if ours => ("YOUR FLAG WAS DROPPED".to_string(), CALL),
        ev::FLAG_DROPPED => ("ENEMY FLAG DROPPED".to_string(), CALL),
        ev::FLAG_RETURNED if ours => ("YOUR FLAG IS BACK".to_string(), GOOD),
        ev::FLAG_RETURNED => ("ENEMY FLAG RETURNED".to_string(), BAD),
        // `team` is the scoring team.
        ev::FLAG_CAPTURED if ours => ("YOUR TEAM CAPTURED THE FLAG".to_string(), GOOD),
        ev::FLAG_CAPTURED => ("ENEMY CAPTURED YOUR FLAG".to_string(), BAD),
        ev::BOMB_PICKED if slot == me => ("YOU HAVE THE BOMB".to_string(), CALL),
        ev::BOMB_DROPPED => ("THE BOMB WAS DROPPED".to_string(), CALL),
        // `team` is the attacking team.
        ev::BOMB_PLANTED if ours => ("BOMB PLANTED - DEFEND IT".to_string(), GOOD),
        ev::BOMB_PLANTED => ("BOMB PLANTED - DEFUSE IT".to_string(), BAD),
        ev::BOMB_DEFUSED if ours => ("THE BOMB WAS DEFUSED".to_string(), BAD),
        ev::BOMB_DEFUSED => ("THE BOMB WAS DEFUSED".to_string(), GOOD),
        ev::BOMB_EXPLODED => ("THE BOMB EXPLODED".to_string(), CALL),
        ev::ROUND_LIVE if ours => ("ATTACK THE SITES".to_string(), CALL),
        ev::ROUND_LIVE => ("DEFEND THE SITES".to_string(), CALL),
        // `team` is the winner, `slot` how.
        ev::ROUND_WON => {
            let how = match slot {
                s if s == WinHow::Exploded as u8 => "BOMB EXPLODED",
                s if s == WinHow::Defused as u8 => "BOMB DEFUSED",
                s if s == WinHow::Eliminated as u8 => "ELIMINATED",
                _ => "TIME UP",
            };
            (format!("{}  {how}", if ours { "ROUND WON" } else { "ROUND LOST" }), if ours { GOOD } else { BAD })
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use red_engine2::net::protocol::{FlagSnap, SndSnap};

    fn input<'a>(mode: u8, obj: &'a ObjSnap, team: u8, me: u8) -> ObjectiveInput<'a> {
        ObjectiveInput { mode, my_team: team, me, obj, alive: [4, 3], sites: &['A', 'B'] }
    }

    #[test]
    fn capture_the_flag_says_who_holds_what() {
        let mut o = ObjSnap { kind: 1, ..Default::default() };
        o.flags[0] = FlagSnap { state: 0, carrier: 255, ..Default::default() };
        o.flags[1] = FlagSnap { state: 1, carrier: 7, ..Default::default() };
        // Team 1, I am player 7: I carry team 2's flag.
        let h = objective_hud(&input(2, &o, 1, 7)).unwrap();
        assert_eq!(h.line, "YOUR FLAG HOME   ENEMY FLAG TAKEN");
        assert!(h.sub.contains("YOU HAVE THE FLAG"));
        // A teammate carries: escort. An enemy carries ours: stop them.
        assert_eq!(objective_hud(&input(2, &o, 1, 3)).unwrap().sub, "ESCORT YOUR CARRIER");
        o.flags[0] = FlagSnap { state: 1, carrier: 9, ..Default::default() };
        o.flags[1] = FlagSnap { state: 2, left_ticks: 600, ..Default::default() };
        let h = objective_hud(&input(2, &o, 1, 3)).unwrap();
        assert_eq!((h.line.as_str(), h.sub_color), ("YOUR FLAG TAKEN   ENEMY FLAG DOWN 10", BAD));
        assert!(h.sub.starts_with("THEY HAVE YOUR FLAG"));
    }

    #[test]
    fn search_and_destroy_says_the_round_the_side_and_the_job() {
        let snd = |bomb, carrier, progress, phase| ObjSnap {
            kind: 2,
            snd: SndSnap { phase, round: 3, attackers: 1, bomb, carrier, site: 1, fuse_ticks: 1500, progress, ..Default::default() },
            ..Default::default()
        };
        let o = snd(0, 5, 0, 1);
        let h = objective_hud(&input(3, &o, 1, 5)).unwrap();
        assert_eq!(h.line, "ROUND 3   ATTACKING   4 V 3   0:00", "the round clock runs while live and unplanted");
        assert!(h.sub.starts_with("YOU HAVE THE BOMB"));
        assert_eq!(objective_hud(&input(3, &o, 1, 6)).unwrap().sub, "ESCORT THE BOMB");
        assert_eq!(objective_hud(&input(3, &o, 2, 8)).unwrap().sub, "STOP THE BOMB BEING PLANTED");
        let planted = snd(2, 255, 100, 1);
        let defend = objective_hud(&input(3, &planted, 2, 8)).unwrap();
        assert_eq!(defend.sub, "DEFUSE THE BOMB AT B   0:25");
        assert_eq!(defend.bar.as_ref().map(|b| b.0.as_str()), Some("DEFUSING"));
        assert!(objective_hud(&input(3, &snd(0, 5, 128, 1), 1, 5)).unwrap().bar.is_some_and(|b| b.0 == "PLANTING" && (b.1 - 0.5).abs() < 0.01));
        assert_eq!(objective_hud(&input(3, &snd(0, 5, 0, 0), 1, 5)).unwrap().sub, "GET READY");
        // The kill modes have no objective lines.
        assert!(objective_hud(&input(0, &ObjSnap::default(), 1, 5)).is_none());
    }

    #[test]
    fn banners_read_from_each_teams_side() {
        assert_eq!(event_banner(ev::FLAG_TAKEN, 2, 4, 1, 4).unwrap().0, "YOU TOOK THE FLAG");
        assert_eq!(event_banner(ev::FLAG_TAKEN, 1, 9, 1, 4).unwrap(), ("ENEMY HAS YOUR FLAG".to_string(), BAD));
        assert_eq!(event_banner(ev::FLAG_CAPTURED, 1, 4, 2, 9).unwrap(), ("ENEMY CAPTURED YOUR FLAG".to_string(), BAD));
        assert_eq!(event_banner(ev::BOMB_PLANTED, 1, 4, 2, 9).unwrap().0, "BOMB PLANTED - DEFUSE IT");
        assert_eq!(event_banner(ev::ROUND_WON, 2, WinHow::Defused as u8, 2, 9).unwrap().0, "ROUND WON  BOMB DEFUSED");
        assert!(event_banner(ev::BOMB_PICKED, 1, 4, 1, 9).is_none(), "someone else picking the bomb up is not news");
        assert!(event_banner(250, 1, 1, 1, 1).is_none());
    }
}
