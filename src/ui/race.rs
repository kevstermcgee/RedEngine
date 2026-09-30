//! The race HUD for Great Outdoors: place, lap, time, speed, the held item, shield and boost, the Beaver's Build cooldown, the countdown numerals and the
//! results banner, as the same audited [`Layout`] the rest of the client UI uses (so `ui-shot race-hud out.png` shows it with no window and `ui-check`
//! audits it at every size). A pure function of what the race and the local kart say; the client fills [`RaceHud`] from the newest snapshot and its prediction.

use super::{fit_scale, text_height, Layout};
use crate::sim::kart::Item;

const TEXT: [u8; 4] = [246, 244, 240, 255];
const DIM: [u8; 4] = [190, 190, 200, 255];
const PANEL: [u8; 4] = [34, 30, 52, 190];
const EDGE: [u8; 4] = [255, 255, 255, 120];
const GOLD: [u8; 4] = [255, 214, 92, 255];
const MINT: [u8; 4] = [150, 232, 196, 255];
const SKY: [u8; 4] = [140, 200, 255, 255];
const PEACH: [u8; 4] = [255, 178, 128, 255];

/// Everything the race HUD shows, in plain numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct RaceHud {
    /// `0` countdown, `1` racing, `2` finished.
    pub phase: u8,
    /// Seconds of countdown left (phase 0).
    pub countdown_secs: f32,
    /// Seconds since the light went green.
    pub race_secs: f32,
    /// The lap the local kart is on (from 1) and the laps in the race.
    pub lap: u8,
    /// Laps in the race.
    pub laps: u8,
    /// The local kart's place (from 1; `0` = unknown) and how many are racing.
    pub place: u8,
    /// How many karts are racing.
    pub racers: u8,
    /// Speed, m/s.
    pub speed: f32,
    /// The pickup held.
    pub item: Item,
    /// Under a Bubble.
    pub shielded: bool,
    /// Boosting.
    pub boosting: bool,
    /// Drift charge tier reached (0 = none, 1-3).
    pub drift_tier: u8,
    /// The driver's ability cooldown, seconds, if the driver has a cooldown ability (the Beaver's Build): `Some(0.0)` = ready.
    pub ability_cooldown: Option<f32>,
    /// Whether the local kart has finished.
    pub finished: bool,
    /// The standings once the race is over: `(place, driver name, how they finished)`.
    pub standings: Vec<(u8, String, Finish)>,
}

/// How one racer's race ended, as far as this client knows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Finish {
    /// Finished in this many seconds.
    Time(f32),
    /// Finished, time not known to this client (only the racer's own is on the wire).
    Done,
    /// Had not finished when the race ended.
    DidNotFinish,
}

impl Default for RaceHud {
    fn default() -> Self {
        RaceHud {
            phase: 1,
            countdown_secs: 0.0,
            race_secs: 0.0,
            lap: 1,
            laps: 3,
            place: 0,
            racers: 0,
            speed: 0.0,
            item: Item::None,
            shielded: false,
            boosting: false,
            drift_tier: 0,
            ability_cooldown: None,
            finished: false,
            standings: Vec::new(),
        }
    }
}

/// `1ST`, `2ND`, `3RD`, `4TH` ... (the teens are all `TH`).
pub fn ordinal(n: u8) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "TH",
        (1, _) => "ST",
        (2, _) => "ND",
        (3, _) => "RD",
        _ => "TH",
    };
    format!("{n}{suffix}")
}

/// `1:23.4`.
pub fn clock(secs: f32) -> String {
    let t = secs.max(0.0);
    let (m, s) = ((t / 60.0) as u32, t % 60.0);
    format!("{m}:{s:04.1}")
}

fn item_look(item: Item) -> (&'static str, [u8; 4]) {
    match item {
        Item::None => ("", [70, 66, 90, 255]),
        Item::Mushroom => ("MUSHROOM", [255, 120, 120, 255]),
        Item::Acorn => ("ACORN", [214, 160, 104, 255]),
        Item::Bubble => ("BUBBLE", [150, 214, 255, 255]),
    }
}

/// The race HUD for a `w` x `h` window.
pub fn race_hud_layout(w: u32, h: u32, hud: &RaceHud) -> Layout {
    let mut l = Layout::new(w, h);
    let (wi, hi) = (w as i32, h as i32);
    let s = (hi / 240).max(1);
    let pad = 4 * s;

    // Countdown numerals, big and central, then GO!
    if hud.phase == 0 {
        let n = hud.countdown_secs.ceil().max(1.0) as u32;
        let text = n.to_string();
        let scale = 8 * s;
        let bw = (34 * s).min(wi - 8);
        let bh = text_height(scale) + 8 * s;
        let x0 = (wi - bw) / 2;
        let y0 = hi / 4;
        let panel = l.panel("countdown", (x0, y0, x0 + bw, y0 + bh), None, Some(PANEL), Some((GOLD, (s / 2).max(1))));
        l.label("countdown_number", Some(panel), wi / 2, y0 + 4 * s, &text, scale.min((bw - 4 * s) / 6).max(s), GOLD);
    } else if hud.phase == 1 && hud.race_secs < 1.2 {
        let bw = (78 * s).min(wi - 8);
        let bh = text_height(6 * s) + 8 * s;
        let x0 = (wi - bw) / 2;
        let y0 = hi / 4;
        let panel = l.panel("go", (x0, y0, x0 + bw, y0 + bh), None, Some(PANEL), Some((MINT, (s / 2).max(1))));
        l.label_fit("go_text", Some(panel), wi / 2, y0 + 4 * s, "GO!", 6 * s, bw - 8 * s, MINT);
    }

    // Top left: place.
    if hud.place > 0 {
        let pw = (58 * s).min(wi / 2 - pad);
        let ph = text_height(4 * s) + text_height(s) + 10 * s;
        let panel = l.panel("place_panel", (pad, pad, pad + pw, pad + ph), None, Some(PANEL), Some((EDGE, 1)));
        l.label_fit("place", Some(panel), pad + pw / 2, pad + 3 * s, &ordinal(hud.place), 4 * s, pw - 6 * s, GOLD);
        let of = format!("OF {}", hud.racers.max(hud.place));
        l.label_fit("place_of", Some(panel), pad + pw / 2, pad + 5 * s + text_height(4 * s), &of, s, pw - 6 * s, DIM);
    }

    // Top right: lap and time.
    {
        let pw = (62 * s).min(wi / 2 - pad);
        let ph = text_height(2 * s) + text_height(s) + 10 * s;
        let x0 = wi - pad - pw;
        let panel = l.panel("lap_panel", (x0, pad, x0 + pw, pad + ph), None, Some(PANEL), Some((EDGE, 1)));
        let lap = format!("LAP {}/{}", hud.lap.min(hud.laps).max(1), hud.laps);
        l.label_fit("lap", Some(panel), x0 + pw / 2, pad + 3 * s, &lap, 2 * s, pw - 6 * s, TEXT);
        l.label_fit("time", Some(panel), x0 + pw / 2, pad + 5 * s + text_height(2 * s), &clock(hud.race_secs), s, pw - 6 * s, DIM);
    }

    // Bottom right: speed.
    {
        let pw = (56 * s).min(wi / 2 - pad);
        let ph = text_height(3 * s) + text_height(s) + 10 * s;
        let (x0, y0) = (wi - pad - pw, hi - pad - ph);
        let panel = l.panel("speed_panel", (x0, y0, x0 + pw, y0 + ph), None, Some(PANEL), Some((EDGE, 1)));
        l.label_fit("speed", Some(panel), x0 + pw / 2, y0 + 3 * s, &format!("{:.0}", hud.speed * 3.6), 3 * s, pw - 6 * s, TEXT);
        l.label_fit("speed_unit", Some(panel), x0 + pw / 2, y0 + 5 * s + text_height(3 * s), "KM/H", s, pw - 6 * s, DIM);
    }

    // Bottom centre: the held item.
    {
        let (name, color) = item_look(hud.item);
        let pw = (46 * s).min(wi - 8);
        let ph = text_height(s) * 2 + 10 * s;
        let x0 = (wi - pw) / 2;
        let y0 = hi - pad - ph;
        let panel =
            l.panel("item_panel", (x0, y0, x0 + pw, y0 + ph), None, Some(PANEL), Some((if hud.item == Item::None { EDGE } else { color }, (s / 2).max(1))));
        l.label_fit("item_title", Some(panel), wi / 2, y0 + 3 * s, "ITEM", s, pw - 6 * s, DIM);
        let text = if name.is_empty() { "-" } else { name };
        l.label_fit("item_name", Some(panel), wi / 2, y0 + 5 * s + text_height(s), text, s, pw - 6 * s, color);
    }

    // Status chips just above the item panel: shield, boost, drift tier, build.
    {
        let mut chips: Vec<(&str, String, [u8; 4])> = Vec::new();
        if hud.shielded {
            chips.push(("chip_shield", "SHIELD".into(), SKY));
        }
        if hud.boosting {
            chips.push(("chip_boost", "BOOST".into(), PEACH));
        }
        if hud.drift_tier > 0 {
            chips.push(("chip_drift", format!("DRIFT {}", "*".repeat(hud.drift_tier.min(3) as usize)), MINT));
        }
        if let Some(cool) = hud.ability_cooldown {
            let text = if cool <= 0.0 { "BUILD READY".to_string() } else { format!("BUILD {cool:.1}S") };
            chips.push(("chip_build", text, if cool <= 0.0 { GOLD } else { DIM }));
        }
        let item_top = hi - pad - (text_height(s) * 2 + 10 * s);
        let ch = text_height(s) + 4 * s;
        let mut y = item_top - pad - ch;
        for (id, text, color) in chips {
            let cw = (84 * s).min(wi - 8);
            let x0 = (wi - cw) / 2;
            if y < pad {
                break;
            }
            let panel = l.panel(id, (x0, y, x0 + cw, y + ch), None, Some(PANEL), Some((color, 1)));
            l.label_fit(&format!("{id}_text"), Some(panel), wi / 2, y + 2 * s, &text, s, cw - 4 * s, color);
            y -= ch + s;
        }
    }

    // Finished: a banner with the local place, and the standings once the race is over.
    if hud.finished || hud.phase == 2 {
        let rows = hud.standings.len().min(8) as i32;
        let row_h = text_height(s) + 3 * s;
        let bw = (150 * s).min(wi - 8);
        let bh = text_height(3 * s) + 8 * s + rows * row_h + if rows > 0 { 6 * s } else { 0 };
        let x0 = (wi - bw) / 2;
        let y0 = ((hi - bh) / 2).max(pad);
        let banner = l.panel("results", (x0, y0, x0 + bw, y0 + bh), None, Some([26, 22, 44, 225]), Some((GOLD, (s / 2).max(1))));
        let title = if hud.place > 0 { format!("{} PLACE!", ordinal(hud.place)) } else { "FINISHED".to_string() };
        l.label_fit("results_title", Some(banner), wi / 2, y0 + 4 * s, &title, 3 * s, bw - 10 * s, GOLD);
        let mut y = y0 + 8 * s + text_height(3 * s);
        for (i, (place, name, secs)) in hud.standings.iter().take(8).enumerate() {
            let text = match secs {
                Finish::Time(t) => format!("{}  {}  {}", ordinal(*place), name.to_uppercase(), clock(*t)),
                Finish::Done => format!("{}  {}", ordinal(*place), name.to_uppercase()),
                Finish::DidNotFinish => format!("{}  {}  DNF", ordinal(*place), name.to_uppercase()),
            };
            let scale = fit_scale(&text, bw - 10 * s, s);
            l.label_fit(&format!("row_{i}"), Some(banner), wi / 2, y, &text, scale, bw - 10 * s, if i == 0 { GOLD } else { TEXT });
            y += row_h;
        }
    }
    l
}

/// A mid-race demo for `ui-shot`: second of eight on lap 2 of 3, a Mushroom in hand, drifting, the Beaver's Build cooling down.
pub fn demo_racing() -> RaceHud {
    RaceHud {
        race_secs: 63.4,
        lap: 2,
        place: 2,
        racers: 8,
        speed: 24.0,
        item: Item::Mushroom,
        boosting: true,
        drift_tier: 2,
        ability_cooldown: Some(2.4),
        ..RaceHud::default()
    }
}

/// The start of a race for `ui-shot`.
pub fn demo_countdown() -> RaceHud {
    RaceHud { phase: 0, countdown_secs: 2.4, place: 5, racers: 8, lap: 1, ..RaceHud::default() }
}

/// The end of a race for `ui-shot`.
pub fn demo_results() -> RaceHud {
    let names = ["Deer", "Duck", "Hawk", "Wolf", "Coyote", "Bunny", "Bear", "Beaver"];
    RaceHud {
        phase: 2,
        finished: true,
        place: 2,
        racers: 8,
        lap: 3,
        race_secs: 151.2,
        standings: names
            .iter()
            .enumerate()
            .map(|(i, n)| (i as u8 + 1, n.to_string(), if i < 6 { Finish::Time(148.0 + i as f32 * 3.7) } else { Finish::DidNotFinish }))
            .collect(),
        ..RaceHud::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(l: &Layout) -> Vec<&str> {
        l.widgets.iter().filter_map(|w| w.text.as_deref()).collect()
    }

    #[test]
    fn ordinals_and_clock_read_naturally() {
        let names: Vec<String> = [1u8, 2, 3, 4, 8, 11, 12, 13, 21, 22].iter().map(|n| ordinal(*n)).collect();
        assert_eq!(names, ["1ST", "2ND", "3RD", "4TH", "8TH", "11TH", "12TH", "13TH", "21ST", "22ND"]);
        assert_eq!((clock(0.0), clock(63.4), clock(151.2), clock(-5.0)), ("0:00.0".into(), "1:03.4".into(), "2:31.2".into(), "0:00.0".into()));
    }

    #[test]
    fn the_racing_hud_shows_place_lap_time_speed_item_and_chips() {
        let l = race_hud_layout(1280, 720, &demo_racing());
        let t = texts(&l);
        for want in ["2ND", "OF 8", "LAP 2/3", "1:03.4", "86", "KM/H", "MUSHROOM", "BOOST", "DRIFT **", "BUILD 2.4S"] {
            assert!(t.contains(&want), "{want} missing from {t:?}");
        }
        assert!(!t.contains(&"SHIELD"), "no shield, no chip");
        assert!(!l.widgets.iter().any(|w| w.id == "results"), "and no results banner mid-race");
    }

    #[test]
    fn the_countdown_and_go_and_results_each_appear_in_their_moment() {
        let start = race_hud_layout(1280, 720, &demo_countdown());
        assert!(texts(&start).contains(&"3"), "2.4 s left rounds up to 3: {:?}", texts(&start));
        let one = race_hud_layout(1280, 720, &RaceHud { phase: 0, countdown_secs: 0.2, ..RaceHud::default() });
        assert!(texts(&one).contains(&"1"));
        let go = race_hud_layout(1280, 720, &RaceHud { race_secs: 0.4, ..RaceHud::default() });
        assert!(texts(&go).contains(&"GO!"));
        let later = race_hud_layout(1280, 720, &RaceHud { race_secs: 5.0, ..RaceHud::default() });
        assert!(!texts(&later).contains(&"GO!"), "GO! is only for the first moments");
        let end = race_hud_layout(1280, 720, &demo_results());
        let t = texts(&end);
        assert!(t.contains(&"2ND PLACE!") && t.iter().any(|x| x.contains("DEER")) && t.iter().any(|x| x.contains("DNF")), "{t:?}");
    }

    #[test]
    fn a_hand_with_nothing_shows_a_dash_and_a_build_that_is_ready_says_so() {
        let l = race_hud_layout(640, 360, &RaceHud { ability_cooldown: Some(0.0), shielded: true, ..RaceHud::default() });
        let t = texts(&l);
        assert!(t.contains(&"-") && t.contains(&"BUILD READY") && t.contains(&"SHIELD"), "{t:?}");
    }

    #[test]
    fn every_race_hud_passes_the_audit_at_every_supported_size() {
        for (w, h) in crate::ui::screens::CHECK_SIZES {
            for (name, hud) in [
                ("racing", demo_racing()),
                ("countdown", demo_countdown()),
                ("results", demo_results()),
                ("everything", RaceHud { shielded: true, boosting: true, drift_tier: 3, ability_cooldown: Some(0.0), item: Item::Bubble, ..demo_racing() }),
                ("idle", RaceHud::default()),
            ] {
                let l = race_hud_layout(w, h, &hud);
                assert!(l.check().is_empty(), "{name} at {w}x{h}: {:?}", l.check());
            }
        }
    }
}
