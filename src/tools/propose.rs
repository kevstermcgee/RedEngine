//! `red_engine2 propose "<idea>"`: a plan for a game before any file is written, so the choices that decide cost (3D or 2D, browser or native, alone or with others) are made
//! on purpose and can be changed.
//!
//! It never defaults to 3D. It recommends the simplest presentation that can deliver the idea, says why, and checks the proposal against the capability matrix (`capabilities`):
//! a request the engine cannot deliver (for example browser play with others) is reported as unsupported with what to do instead, never quietly shrunk. The cost lines are
//! **sizes taken from the example games and the loop's command list**, not token counts: tokens are not measured here and are not invented.

use red2d::caps::{self, Capabilities, Input, Networking, Persistence, Platform, Presentation};
use serde_json::{json, Value};

/// What the caller insists on (each overrides the heuristic and is then checked, not trusted).
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    /// The title.
    pub title: Option<String>,
    /// `2d` or `3d`.
    pub presentation: Option<Presentation>,
    /// Targets.
    pub platforms: Vec<Platform>,
    /// Input methods.
    pub input: Vec<Input>,
    /// Networking.
    pub networking: Option<Networking>,
    /// Session length in minutes.
    pub session_minutes: Option<u32>,
}

/// A plan.
#[derive(Debug, Clone)]
pub struct Proposal {
    /// Title.
    pub title: String,
    /// The kind of game it reads as.
    pub genre: &'static str,
    /// The declaration.
    pub caps: Capabilities,
    /// Minutes per session (low, high).
    pub session: (u32, u32),
    /// `small`, `medium` or `large`.
    pub complexity: &'static str,
    /// Why each choice was made.
    pub reasons: Vec<String>,
    /// What the engine cannot deliver as asked, with the way out.
    pub problems: Vec<String>,
    /// Facts that qualify the plan (unverified input paths).
    pub warnings: Vec<String>,
}

/// Whether the idea mentions any of the terms: a single word must match a whole word (so "shop" is not "hop"), a phrase matches as written.
fn has(text: &str, words: &[&str]) -> bool {
    let tokens: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();
    words.iter().any(|w| {
        if w.contains(' ') || w.contains('-') {
            text.contains(w)
        } else {
            tokens.iter().any(|t| t == w || t.strip_suffix('s') == Some(*w) || t.strip_suffix("es") == Some(*w))
        }
    })
}

fn title_of(idea: &str, genre: &str) -> String {
    const SKIP: &[&str] = &[
        "a",
        "an",
        "the",
        "small",
        "little",
        "simple",
        "tiny",
        "game",
        "where",
        "you",
        "your",
        "that",
        "with",
        "and",
        "of",
        "to",
        "in",
        "on",
        "make",
        "create",
        "build",
        "me",
        "for",
        "my",
        "is",
        "it",
        "browser",
        "playable",
        "2d",
        "3d",
        "publish",
        "publishing",
        "arcade",
        "puzzle",
        "platformer",
        "mouse",
        "keyboard",
        "web",
        "url",
        "just",
        "like",
        "want",
        "please",
        "can",
        "be",
        "so",
        "about",
    ];
    let words: Vec<String> = idea
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !SKIP.contains(&w.to_lowercase().as_str()))
        .take(3)
        .map(|w| w.chars().next().map(|c| c.to_uppercase().collect::<String>() + &w[1..].to_lowercase()).unwrap_or_default())
        .collect();
    if words.is_empty() {
        format!("Untitled {}", genre.split(' ').next().map(|g| g[..1].to_uppercase() + &g[1..]).unwrap_or_default())
    } else {
        words.join(" ")
    }
}

/// Plans a game from an idea.
pub fn propose(idea: &str, o: &Overrides) -> Proposal {
    let t = idea.to_lowercase();
    let mut reasons = Vec::new();
    let mut problems = Vec::new();

    // A 3D *part* (a boss model, a minimap, the world in perspective) in an otherwise 2D game is hybrid; a 3D *world* to walk or drive through is 3D.
    let wants_hybrid = has(
        &t,
        &[
            "hybrid",
            "minimap",
            "mini-map",
            "mini map",
            "map overlay",
            "3d boss",
            "3d model",
            "3d element",
            "3d hud",
            "3d portrait",
            "3d backdrop",
            "2d and 3d",
            "3d and 2d",
            "2d with 3d",
            "in perspective",
            "rotating model",
            "spinning model",
        ],
    );
    let wants_3d_world = has(
        &t,
        &[
            "first person",
            "first-person",
            "fps",
            "open world",
            "walk around",
            "explore a house",
            "kart",
            "driving",
            "racing in 3d",
            "third person",
            "third-person",
        ],
    );
    let wants_plain_3d = has(&t, &["3d"]);
    let wants_others = has(&t, &["multiplayer", "online", "with friends", "co-op", "coop", "versus", "pvp", "play with my", "with my cousin", "with others"]);
    let wants_web = has(&t, &["browser", "webgl", "wasm", "webassembly", "in the web", "web game", "web app"]);

    // Genre -> default input, session, mechanics weight.
    let (genre, input_default, session, weight): (&'static str, &[Input], (u32, u32), u32) =
        if has(&t, &["platformer", "jump", "side-scroll", "side scroll", "hop"]) {
            ("platformer", &[Input::Keyboard], (5, 10), 3)
        } else if has(
            &t,
            &[
                "strategy",
                "management",
                "tycoon",
                "manage",
                "simulation",
                "build a station",
                "city",
                "farm",
                "idle",
                "clicker",
                "card",
                "tower defense",
                "tower defence",
            ],
        ) {
            ("management", &[Input::Mouse, Input::Keyboard], (8, 15), 5)
        } else if has(&t, &["puzzle", "match", "sokoban", "word", "sudoku", "memory"]) {
            ("puzzle", &[Input::Mouse], (5, 15), 3)
        } else if has(&t, &["shooter", "shmup", "space invaders", "asteroid", "dodge", "survive", "arena"]) {
            ("arcade shooter", &[Input::Keyboard, Input::Mouse], (2, 6), 3)
        } else if has(&t, &["arcade", "collect", "coin", "catch", "chase", "maze", "snake", "pong", "breakout"]) {
            ("arcade", &[Input::Keyboard], (2, 5), 2)
        } else {
            ("arcade", &[Input::Keyboard], (3, 8), 2)
        };
    let mechanics = [
        "enemy",
        "enemies",
        "boss",
        "inventory",
        "crafting",
        "shop",
        "levels",
        "score",
        "timer",
        "lives",
        "power-up",
        "powerup",
        "upgrade",
        "quest",
        "dialog",
        "story",
        "procedural",
        "random",
        "physics",
        "gravity",
        "particles",
    ]
    .iter()
    .filter(|w| has(&t, &[w]))
    .count() as u32
        + weight;
    let complexity = if mechanics <= 3 {
        "small"
    } else if mechanics <= 6 {
        "medium"
    } else {
        "large"
    };

    // Presentation: the simplest one that delivers the idea.
    let presentation = match o.presentation {
        Some(p) => {
            reasons.push(format!("presentation {} was requested; it is checked below, not assumed", p.name()));
            p
        }
        None if wants_hybrid && !wants_3d_world && !wants_others => {
            reasons.push(
                "the idea wants 3D parts (a model, a perspective view, a minimap) inside an otherwise 2D game: hybrid keeps one JSON file, the 2D loop and native play, and draws only those parts in 3D".to_string(),
            );
            Presentation::Hybrid
        }
        None if wants_3d_world || wants_plain_3d || wants_others => {
            reasons.push(if wants_3d_world || wants_plain_3d {
                "the idea needs a 3D world (first-person, driving or exploration), which 2D cannot show".to_string()
            } else {
                "playing with others needs the authoritative server, which only the 3D engine has".to_string()
            });
            Presentation::ThreeD
        }
        None => {
            reasons.push("2D is the simplest presentation that can deliver this idea: one JSON file, deterministic headless tests, and it plays in a native window; choose 3D only for a 3D world".to_string());
            Presentation::TwoD
        }
    };

    let networking = o.networking.unwrap_or(if wants_others { Networking::Authoritative } else { Networking::Offline });
    if networking == Networking::Authoritative && o.networking.is_none() {
        reasons.push("the idea involves other players, so networking is authoritative".to_string());
    }

    let platforms: Vec<Platform> = if !o.platforms.is_empty() {
        o.platforms.clone()
    } else {
        reasons.push("every game is a native executable: windows and linux".to_string());
        vec![Platform::Windows, Platform::Linux]
    };
    if wants_web {
        problems.push(
            "RedEngine has no browser target: games are native executables (`red_engine2 play2d` for 2D, the 3D client for 3D); drop the browser from the idea"
                .to_string(),
        );
    }

    let input: Vec<Input> =
        if o.input.is_empty() { input_default.iter().copied().filter(|i| !presentation.portable() || *i != Input::Gamepad).collect() } else { o.input.clone() };
    let mut persistence = Vec::new();
    if has(&t, &["score", "high score", "best", "progress", "save", "unlock", "level"]) || genre != "puzzle" && complexity != "small" {
        persistence.push(Persistence::Progress);
    }
    if has(&t, &["music", "volume", "settings", "sound"]) || session.1 >= 8 {
        persistence.push(Persistence::Settings);
    }
    if persistence.is_empty() {
        persistence.push(Persistence::Settings);
    }
    let minutes = o.session_minutes.map(|m| (m, m)).unwrap_or(session);

    let distribution = Capabilities::default_distribution(&platforms);
    let c = Capabilities { presentation, platforms, networking, input, persistence, distribution };
    for p in caps::check(&c) {
        problems.push(p.to_string());
    }
    let warnings = caps::warnings(&c);
    Proposal { title: o.title.clone().unwrap_or_else(|| title_of(idea, genre)), genre, caps: c, session: minutes, complexity, reasons, problems, warnings }
}

impl Proposal {
    /// Whether the plan can be built as stated.
    pub fn buildable(&self) -> bool {
        self.problems.is_empty()
    }

    /// The `capabilities` block this plan would put in the game file.
    pub fn capabilities_json(&self) -> Value {
        let c = &self.caps;
        json!({
            "presentation": c.presentation.name(),
            "platforms": c.platforms.iter().map(|p| p.name()).collect::<Vec<_>>(),
            "networking": c.networking.name(),
            "input": c.input.iter().map(|i| i.name()).collect::<Vec<_>>(),
            "persistence": c.persistence.iter().map(|p| p.name()).collect::<Vec<_>>(),
        })
    }

    /// The plan as JSON.
    pub fn to_json(&self) -> Value {
        let two_d = self.caps.presentation.portable();
        json!({
            "title": self.title,
            "genre": self.genre,
            "capabilities": self.capabilities_json(),
            "session_minutes": [self.session.0, self.session.1],
            "complexity": self.complexity,
            "reasons": self.reasons,
            "problems": self.problems,
            "warnings": self.warnings,
            "buildable": self.buildable(),
            "cost": self.cost(),
            "next": self.next(two_d),
        })
    }

    /// Sizes to expect. The 2D figures are the example games' (a small one is Coin Dash's size, a large one Tiny Station's); the 3D figures are the walk/race starters'.
    pub fn cost(&self) -> Value {
        let two_d = self.caps.presentation.portable();
        let (bytes, files): (&str, u32) = match (two_d, self.complexity) {
            (true, "small") => ("6-9 KB of JSON", 1),
            (true, "medium") => ("9-16 KB of JSON", 1),
            (true, _) => ("16-30 KB of JSON", 1),
            (false, "small") => ("a blueprint of 2-4 KB plus scene rules", 2),
            (false, "medium") => ("a blueprint and rules of 5-15 KB", 3),
            (false, _) => ("15 KB+ across blueprint, rules, audio and a hosting setup", 5),
        };
        json!({
            "files_to_write": files,
            "size": bytes,
            "files_to_read_first": if self.caps.presentation == Presentation::Hybrid { json!(["`describe 2d` (one page, ~8 KB)", "`describe hybrid` (the 3D parts, ~4 KB)"]) } else if two_d { json!(["`describe 2d` (one page, ~8 KB)"]) } else { json!(["`describe --brief`", "`describe rules`", "`recipe`/`catalog`"]) },
            "loop_commands": if two_d { json!(["validate", "sim", "verify", "frame", "play2d"]) } else { json!(["validate", "lint", "verify", "plan/frame", "playtest"]) },
            "basis": "sizes of the example games and starters in this repository; no token counts are claimed",
        })
    }

    fn next(&self, two_d: bool) -> Vec<String> {
        if !self.buildable() {
            return vec!["change the plan so it is buildable (the problems say how), then run `propose` again with the overrides".into()];
        }
        if two_d {
            vec![
                "red_engine2 new-game DIR --kind 2d        # a verified starter, then replace its game with this plan".into(),
                "red_engine2 describe 2d                   # the file format on one page".into(),
                "red_engine2 verify DIR/NAME.game2d.json   # after every change".into(),
                "red_engine2 play2d DIR/NAME.game2d.json   # then play it in a window".into(),
            ]
        } else {
            vec![
                "red_engine2 new-game DIR [--kind race]  # a verified 3D starter".into(),
                "red_engine2 describe rules   # game logic as data".into(),
                "scripts/red check          # after every change".into(),
            ]
        }
    }

    /// The plan as text.
    pub fn render(&self) -> String {
        let c = &self.caps;
        let list = |v: Vec<&str>| v.join(", ");
        let mut t = format!(
            "PROPOSAL: {}  ({} game, {} complexity, {}-{} minutes a session)\n  presentation  {}\n  targets       {}\n  networking    {}\n  input         {}\n  saves         {}\n",
            self.title,
            self.genre,
            self.complexity,
            self.session.0,
            self.session.1,
            c.presentation.name(),
            list(c.platforms.iter().map(|p| p.name()).collect()),
            c.networking.name(),
            list(c.input.iter().map(|i| i.name()).collect()),
            list(c.persistence.iter().map(|p| p.name()).collect())
        );
        for r in &self.reasons {
            t.push_str(&format!("  why: {r}\n"));
        }
        let cost = self.cost();
        t.push_str(&format!(
            "  cost: {} file(s) to write, {}; read first: {}; loop: {}\n        ({})\n",
            cost["files_to_write"],
            cost["size"].as_str().unwrap_or(""),
            cost["files_to_read_first"].as_array().map_or(String::new(), |a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
            cost["loop_commands"].as_array().map_or(String::new(), |a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" > ")),
            cost["basis"].as_str().unwrap_or("")
        ));
        for p in &self.problems {
            t.push_str(&format!("  CANNOT BUILD AS STATED: {p}\n"));
        }
        for w in &self.warnings {
            t.push_str(&format!("  note: {w}\n"));
        }
        t.push_str(&format!("  capabilities: {}\n  next:\n", self.capabilities_json()));
        for n in self.next(c.presentation.portable()) {
            t.push_str(&format!("    {n}\n"));
        }
        t.push_str("  Change any choice with --presentation/--platform/--input/--networking/--session/--title and run again; each is checked, not trusted.\n");
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(idea: &str) -> Proposal {
        propose(idea, &Overrides::default())
    }

    #[test]
    fn a_small_arcade_idea_is_a_2d_native_game_and_never_defaults_to_3d() {
        let r = p("Create a small 2D arcade game where you collect coins and dodge ghosts");
        assert_eq!(r.caps.presentation, Presentation::TwoD);
        assert_eq!(r.caps.platforms, vec![Platform::Windows, Platform::Linux]);
        assert_eq!(r.caps.networking, Networking::Offline);
        assert!(r.buildable() && r.caps.input.contains(&Input::Keyboard));
        assert!(r.reasons.iter().any(|x| x.contains("simplest presentation")));
        assert_eq!(p("a game about a cat").caps.presentation, Presentation::TwoD, "no hint of a 3D world means 2D");
        assert_eq!(r.complexity, "small");
    }

    #[test]
    fn a_3d_part_in_a_2d_game_is_hybrid_and_a_3d_world_is_3d() {
        let r = p("a top-down arena with a 3D boss and a minimap");
        assert_eq!(r.caps.presentation, Presentation::Hybrid, "{}", r.render());
        assert_eq!(r.caps.platforms, vec![Platform::Windows, Platform::Linux], "hybrid ships natively like 2D");
        assert!(r.buildable() && r.reasons.iter().any(|x| x.contains("hybrid")), "{}", r.render());
        assert!(r.to_json()["cost"]["files_to_read_first"].to_string().contains("describe hybrid"));
        assert!(r.render().contains("red_engine2 describe 2d"), "hybrid is planned with the 2D loop");
        assert_eq!(p("a 3D racing game").caps.presentation, Presentation::ThreeD);
        assert_eq!(p("a first-person game with a minimap").caps.presentation, Presentation::ThreeD, "a world to walk through is 3D even with a flat map");
        assert_eq!(p("a puzzle game with a spinning model as the prize").caps.presentation, Presentation::Hybrid);
    }

    #[test]
    fn genres_choose_input_session_length_and_saves() {
        let m = p("a space station management game with a shop, upgrades and a timer");
        assert_eq!((m.genre, m.complexity), ("management", "large"));
        assert!(m.caps.input.contains(&Input::Mouse) && m.session.1 >= 10 && m.caps.persistence.contains(&Persistence::Progress));
        let pl = p("a platformer where you jump over spikes and keep your high score");
        assert!(pl.caps.input == vec![Input::Keyboard] && pl.caps.persistence.contains(&Persistence::Progress));
        assert_eq!(p("a sliding puzzle").caps.input, vec![Input::Mouse]);
    }

    #[test]
    fn a_3d_world_is_proposed_only_when_the_idea_needs_one() {
        let r = p("a first-person game where you explore a haunted house");
        assert_eq!(r.caps.presentation, Presentation::ThreeD);
        assert_eq!(r.caps.platforms, vec![Platform::Windows, Platform::Linux]);
        assert!(r.buildable());
        assert!(r.reasons.iter().any(|x| x.contains("3D world")));
    }

    #[test]
    fn a_browser_is_reported_unsupported_with_the_way_out_not_shrunk() {
        let r = p("a small arcade game I can play in the browser");
        assert!(!r.buildable(), "{}", r.render());
        let text = r.render();
        assert!(text.contains("CANNOT BUILD AS STATED") && text.contains("no browser target") && text.contains("native executables"), "{text}");
    }

    #[test]
    fn other_players_make_a_3d_game_with_the_authoritative_server() {
        let r = p("an online co-op shooter to play with my cousin");
        assert_eq!(r.caps.presentation, Presentation::ThreeD, "{}", r.render());
        assert_eq!(r.caps.networking, Networking::Authoritative);
        assert!(r.buildable(), "{}", r.render());
    }

    #[test]
    fn overrides_are_checked_not_trusted() {
        let r =
            propose("a small arcade game", &Overrides { presentation: Some(Presentation::ThreeD), platforms: vec![Platform::MacOs], ..Overrides::default() });
        assert!(!r.buildable() && r.problems.iter().any(|x| x.contains("cannot target `macos`")), "{:?}", r.problems);
        let r = propose(
            "a small arcade game",
            &Overrides { title: Some("Zip".into()), session_minutes: Some(4), input: vec![Input::Gamepad], ..Overrides::default() },
        );
        assert!(!r.buildable() && r.problems.iter().any(|x| x.contains("gamepad") && x.contains("prepared")), "{:?}", r.problems);
        let r = propose("a small arcade game", &Overrides { title: Some("Zip".into()), session_minutes: Some(4), ..Overrides::default() });
        assert!(r.buildable() && r.title == "Zip" && r.session == (4, 4));
        assert!(r.warnings.iter().any(|w| w.contains("install") && w.contains("unverified")), "{:?}", r.warnings);
    }

    #[test]
    fn the_cost_is_stated_in_sizes_with_its_basis_and_the_json_is_complete() {
        let r = p("a small arcade game");
        let j = r.to_json();
        for k in ["title", "genre", "capabilities", "session_minutes", "complexity", "reasons", "problems", "buildable", "cost", "next"] {
            assert!(j.get(k).is_some(), "{k}");
        }
        assert!(j["cost"]["basis"].as_str().unwrap().contains("no token counts are claimed"));
        assert!(!r.render().to_lowercase().contains("tokens saved"));
        assert_eq!(title_of("Create a small 2D arcade game", "arcade"), "Untitled Arcade");
        assert_eq!(title_of("a game about space snails", "arcade"), "Space Snails");
    }
}
