//! What a game says it is, and what RedEngine can actually deliver for it.
//!
//! A game declares `capabilities`: how it is presented (`2d`/`3d`), where it runs (`web`, `windows`, `linux`), how it networks (`offline`/`authoritative`), how it is
//! played (`keyboard`, `mouse`, `touch`, `gamepad`) and what it keeps (`settings`, `progress`). [`check`] holds that declaration to the support matrix in [`support`]:
//! every combination is `Supported` (built and verified), `Unverified` (built, but nothing here has run it), `Prepared` (the architecture is ready and the feature is
//! not built) or `NotSupported`. Only `Supported` and `Unverified` pass; the others fail early with the reason and the way out. Nothing is ever downgraded silently: a
//! request for a target RedEngine cannot deliver is an error that names the target, never a smaller build.

use crate::fields::{check_keys, describe_value};
use serde_json::{Map, Value};

/// How a game is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presentation {
    /// Flat: sprites, shapes, text, drawn by the CPU renderer (`red2d`).
    TwoD,
    /// The 3D renderer (wgpu) and the authoritative simulation.
    ThreeD,
}

/// Where a game runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// A browser, as a static WebAssembly package.
    Web,
    /// Windows, as an installer.
    Windows,
    /// Linux.
    Linux,
    /// macOS (no build or test exists for it).
    MacOs,
}

/// How a game uses the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Networking {
    /// One player on one machine, nothing sent anywhere.
    Offline,
    /// A server that owns the game state, clients that predict (native UDP/QUIC).
    Authoritative,
}

/// How a game is controlled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Input {
    /// Keys.
    Keyboard,
    /// Pointer position and buttons.
    Mouse,
    /// Touch screens (the browser reports touches as pointer events).
    Touch,
    /// A game controller.
    Gamepad,
}

/// What a game keeps between sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Persistence {
    /// Options such as music on or off.
    Settings,
    /// Scores and unlocks.
    Progress,
}

macro_rules! names {
    ($t:ident { $($v:ident => $n:literal),+ $(,)? }) => {
        impl $t {
            /// Every value.
            pub const ALL: &'static [$t] = &[$($t::$v),+];
            /// The name used in JSON.
            pub fn name(self) -> &'static str { match self { $($t::$v => $n),+ } }
            /// Parses a JSON name.
            pub fn parse(s: &str) -> Option<$t> { match s { $($n => Some($t::$v),)+ _ => None } }
            /// All names, for messages.
            pub fn names() -> Vec<&'static str> { vec![$($n),+] }
        }
    };
}
names!(Presentation { TwoD => "2d", ThreeD => "3d" });
names!(Platform { Web => "web", Windows => "windows", Linux => "linux", MacOs => "macos" });
names!(Networking { Offline => "offline", Authoritative => "authoritative" });
names!(Input { Keyboard => "keyboard", Mouse => "mouse", Touch => "touch", Gamepad => "gamepad" });
names!(Persistence { Settings => "settings", Progress => "progress" });

/// A game's declared capabilities.
#[derive(Debug, Clone, PartialEq)]
pub struct Capabilities {
    /// How it is drawn.
    pub presentation: Presentation,
    /// Where it must run: every one is a promise.
    pub platforms: Vec<Platform>,
    /// How it networks.
    pub networking: Networking,
    /// How it is controlled.
    pub input: Vec<Input>,
    /// What it keeps.
    pub persistence: Vec<Persistence>,
}

/// How far a combination is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Support {
    /// Built, and verified by a test that runs it.
    Supported,
    /// Built, but nothing in this repository runs it (the note says what is missing).
    Unverified(&'static str),
    /// The design allows it and the code does not exist (the note says what is missing).
    Prepared(&'static str),
    /// Not supported, and why.
    NotSupported(&'static str),
}

impl Support {
    /// Whether a game may declare this combination.
    pub fn allowed(&self) -> bool {
        matches!(self, Support::Supported | Support::Unverified(_))
    }
    /// A short label for tables.
    pub fn label(&self) -> &'static str {
        match self {
            Support::Supported => "SUPPORTED",
            Support::Unverified(_) => "UNVERIFIED",
            Support::Prepared(_) => "PREPARED",
            Support::NotSupported(_) => "NOT SUPPORTED",
        }
    }
    /// The explanation, if there is one.
    pub fn note(&self) -> &'static str {
        match self {
            Support::Supported => "",
            Support::Unverified(n) | Support::Prepared(n) | Support::NotSupported(n) => n,
        }
    }
}

/// What RedEngine delivers for a presentation on a platform. (Networking is judged by [`networking_support`].)
pub fn support(presentation: Presentation, platform: Platform) -> Support {
    use Platform::*;
    use Presentation::*;
    match (presentation, platform) {
        (TwoD, Web) => Support::Supported,
        (TwoD, Windows | Linux) => Support::Prepared(
            "a native window for 2D games is not built yet. `red_engine2 frame`/`sim`/`verify` already run a 2D game natively with no window, and the same game plays in a browser; \
             a windowed player belongs to the app layer (`red_engine2::app::shell`), where a 2D game would show the CPU frame",
        ),
        (TwoD, MacOs) | (ThreeD, MacOs) => Support::NotSupported("no macOS build or test exists; supported platforms are web (2D games), windows and linux (3D games)"),
        (ThreeD, Windows | Linux) => Support::Supported,
        (ThreeD, Web) => Support::NotSupported(
            "3D games need the wgpu renderer and the engine library, neither of which builds for WebAssembly yet. Browser games are 2D: declare `presentation: \"2d\"`, \
             or drop `web` from `platforms` and ship the 3D game for windows/linux",
        ),
    }
}

/// What RedEngine delivers for a networking mode in a presentation on a platform.
pub fn networking_support(presentation: Presentation, platform: Platform, networking: Networking) -> Support {
    match (networking, presentation, platform) {
        (Networking::Offline, _, _) => Support::Supported,
        (Networking::Authoritative, _, Platform::Web) => Support::NotSupported(
            "Browser target cannot use the native UDP transport (or QUIC). Supported networking for web games: offline. A browser-compatible authoritative transport \
             (WebTransport, or a WebSocket relay in front of the existing server) is architecturally prepared, since the simulation does not depend on the transport, but it is \
             not implemented yet",
        ),
        (Networking::Authoritative, Presentation::TwoD, _) => Support::NotSupported(
            "2D games are offline: the authoritative server runs the 3D simulation (`MatchSim`), and there is no 2D netcode. Declare `networking: \"offline\"`, or make a 3D game",
        ),
        (Networking::Authoritative, Presentation::ThreeD, _) => Support::Supported,
    }
}

/// What RedEngine delivers for an input method on a platform.
pub fn input_support(presentation: Presentation, platform: Platform, input: Input) -> Support {
    match (input, presentation, platform) {
        (Input::Keyboard | Input::Mouse, _, _) => Support::Supported,
        (Input::Touch, Presentation::TwoD, Platform::Web) => {
            Support::Unverified("pointer events cover touch, and the pointer path is tested with a mouse; no touch device or emulated touch run exists here")
        }
        (Input::Touch, _, _) => Support::NotSupported("touch input exists only for 2D games in a browser"),
        (Input::Gamepad, Presentation::TwoD, Platform::Web) => {
            Support::Unverified("the browser's Gamepad API is mapped to the game's actions (move, action), but no controller was available to test it")
        }
        (Input::Gamepad, Presentation::ThreeD, Platform::Windows | Platform::Linux) => Support::Supported,
        (Input::Gamepad, _, _) => Support::NotSupported("gamepads work in 2D browser games and in native 3D games"),
    }
}

/// What RedEngine delivers for saving on a platform (the same two kinds of data everywhere; only the place differs).
pub fn persistence_support(presentation: Presentation, platform: Platform, _kind: Persistence) -> Support {
    match (presentation, platform) {
        (Presentation::TwoD, Platform::Web) => Support::Supported,
        (Presentation::ThreeD, Platform::Windows | Platform::Linux) => Support::Supported,
        _ => Support::NotSupported("this presentation/platform pair is not supported, so there is nowhere to save"),
    }
}

/// One thing wrong with a declaration: where, and what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The path in the game file, like `capabilities.platforms[1]`.
    pub path: String,
    /// What is wrong and how to fix it.
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

const KEYS: &[&str] = &["presentation", "platforms", "networking", "input", "persistence"];

fn list_of<T: Copy>(obj: &Map<String, Value>, key: &str, parse: fn(&str) -> Option<T>, names: &[&str], problems: &mut Vec<Problem>) -> Vec<T> {
    let Some(v) = obj.get(key) else { return Vec::new() };
    let Some(items) = v.as_array() else {
        problems.push(Problem { path: format!("capabilities.{key}"), message: format!("expected a list like [\"{}\"], got {}", names[0], describe_value(v)) });
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match item.as_str().and_then(parse) {
            Some(t) => out.push(t),
            None => {
                let shown = item.as_str().map_or_else(|| describe_value(item), |s| format!("`{s}`"));
                let near = item.as_str().map(|s| crate::suggest::suggest(s, names.iter().copied())).unwrap_or_default();
                let did = near.first().map_or(String::new(), |n| format!(" — did you mean `{n}`?"));
                problems.push(Problem { path: format!("capabilities.{key}[{i}]"), message: format!("{shown} is not one of {}{did}", names.join(", ")) });
            }
        }
    }
    out
}

/// Parses a `capabilities` object. Every mistake is reported (unknown keys with a likely fix, values that are not names, empty `platforms`), and the
/// returned declaration holds whatever did parse, so the caller can show all problems at once. `platforms` is required: a game must say where it runs.
pub fn parse(v: &Value) -> (Option<Capabilities>, Vec<Problem>) {
    let mut problems = Vec::new();
    let Some(obj) = v.as_object() else {
        return (
            None,
            vec![Problem {
                path: "capabilities".into(),
                message: format!(
                    "expected an object like {{\"presentation\": \"2d\", \"platforms\": [\"web\"], \"networking\": \"offline\"}}, got {}",
                    describe_value(v)
                ),
            }],
        );
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "capabilities", obj, KEYS);
    problems.extend(errs.into_iter().map(|e| match e.split_once(": ") {
        Some((path, message)) => Problem { path: path.to_string(), message: message.to_string() },
        None => Problem { path: "capabilities".into(), message: e },
    }));
    let one = |key: &str, parse: fn(&str) -> Option<u8>, names: Vec<&str>, default: Option<u8>, problems: &mut Vec<Problem>| -> Option<u8> {
        match obj.get(key) {
            None => default,
            Some(v) => match v.as_str().and_then(parse) {
                Some(x) => Some(x),
                None => {
                    let shown = v.as_str().map_or_else(|| describe_value(v), |s| format!("`{s}`"));
                    let near = v.as_str().map(|s| crate::suggest::suggest(s, names.iter().copied())).unwrap_or_default();
                    let did = near.first().map_or(String::new(), |n| format!(" — did you mean `{n}`?"));
                    problems.push(Problem { path: format!("capabilities.{key}"), message: format!("{shown} is not one of {}{did}", names.join(", ")) });
                    None
                }
            },
        }
    };
    let presentation = one("presentation", |s| Presentation::parse(s).map(|p| p as u8), Presentation::names(), Some(Presentation::TwoD as u8), &mut problems)
        .map(|p| Presentation::ALL[p as usize]);
    let networking = one("networking", |s| Networking::parse(s).map(|p| p as u8), Networking::names(), Some(Networking::Offline as u8), &mut problems)
        .map(|p| Networking::ALL[p as usize]);
    let platforms = list_of(obj, "platforms", Platform::parse, &Platform::names(), &mut problems);
    if !obj.contains_key("platforms") {
        problems.push(Problem {
            path: "capabilities".into(),
            message: "needs `platforms`: say where the game must run, like [\"web\"] (the games RedEngine can build for the web are 2D games)".into(),
        });
    } else if platforms.is_empty() && problems.iter().all(|p| !p.path.starts_with("capabilities.platforms")) {
        problems.push(Problem { path: "capabilities.platforms".into(), message: "is empty: list at least one of web, windows, linux".into() });
    }
    let input = list_of(obj, "input", Input::parse, &Input::names(), &mut problems);
    let persistence = list_of(obj, "persistence", Persistence::parse, &Persistence::names(), &mut problems);
    let caps = match (presentation, networking) {
        (Some(presentation), Some(networking)) => Some(Capabilities { presentation, platforms, networking, input, persistence }),
        _ => None,
    };
    (caps, problems)
}

/// Holds a declaration to the support matrix: for each platform it names, the presentation, networking, input and persistence it asks for must be allowed. Each
/// refusal says which declared value caused it, what RedEngine does support, and what to change.
pub fn check(c: &Capabilities) -> Vec<Problem> {
    let mut out = Vec::new();
    for (i, &platform) in c.platforms.iter().enumerate() {
        let path = format!("capabilities.platforms[{i}]");
        let s = support(c.presentation, platform);
        if !s.allowed() {
            out.push(Problem {
                path,
                message: format!("{} cannot target `{}` ({}): {}", c.presentation.name().to_uppercase(), platform.name(), s.label().to_lowercase(), s.note()),
            });
            continue;
        }
        let n = networking_support(c.presentation, platform, c.networking);
        if !n.allowed() {
            out.push(Problem {
                path: "capabilities.networking".into(),
                message: format!("`{}` networking on `{}` is {}: {}", c.networking.name(), platform.name(), n.label().to_lowercase(), n.note()),
            });
        }
        for input in &c.input {
            let s = input_support(c.presentation, platform, *input);
            if !s.allowed() {
                out.push(Problem {
                    path: "capabilities.input".into(),
                    message: format!("`{}` input on `{}` is {}: {}", input.name(), platform.name(), s.label().to_lowercase(), s.note()),
                });
            }
        }
        for kind in &c.persistence {
            let s = persistence_support(c.presentation, platform, *kind);
            if !s.allowed() {
                out.push(Problem {
                    path: "capabilities.persistence".into(),
                    message: format!("`{}` saving on `{}` is {}: {}", kind.name(), platform.name(), s.label().to_lowercase(), s.note()),
                });
            }
        }
    }
    out.dedup();
    out
}

/// The warnings a passing declaration still carries: everything `Unverified` that it relies on.
pub fn warnings(c: &Capabilities) -> Vec<String> {
    let mut out = Vec::new();
    for &platform in &c.platforms {
        for input in &c.input {
            if let Support::Unverified(note) = input_support(c.presentation, platform, *input) {
                out.push(format!("`{}` input on `{}` is built but unverified: {note}", input.name(), platform.name()));
            }
        }
        if let Support::Unverified(note) = support(c.presentation, platform) {
            out.push(format!("{} on `{}` is built but unverified: {note}", c.presentation.name().to_uppercase(), platform.name()));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The whole matrix as text, for `describe capabilities`: every presentation x platform, then what networking, input and saving support.
pub fn matrix_text() -> String {
    let mut s = String::from("PRESENTATION x PLATFORM\n");
    for &pres in Presentation::ALL {
        for &plat in Platform::ALL {
            let sup = support(pres, plat);
            s.push_str(&format!("  {:<3} on {:<8} {:<13} {}\n", pres.name(), plat.name(), sup.label(), sup.note()));
        }
    }
    s.push_str("NETWORKING (offline works everywhere a presentation does)\n");
    for &pres in Presentation::ALL {
        let sup = networking_support(pres, Platform::Web, Networking::Authoritative);
        s.push_str(&format!("  {:<3} authoritative on web    {:<13} {}\n", pres.name(), sup.label(), sup.note()));
    }
    let sup = networking_support(Presentation::TwoD, Platform::Windows, Networking::Authoritative);
    s.push_str(&format!("  2d  authoritative on native {:<13} {}\n", sup.label(), sup.note()));
    s.push_str("  3d  authoritative on native SUPPORTED\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn caps(v: Value) -> (Option<Capabilities>, Vec<Problem>, Vec<Problem>) {
        let (c, mut p) = parse(&v);
        let refused = c.as_ref().map(check).unwrap_or_default();
        p.extend(Vec::new());
        (c, p, refused)
    }

    fn all(v: Value) -> String {
        let (_, p, r) = caps(v);
        p.iter().chain(r.iter()).map(Problem::to_string).collect::<Vec<_>>().join(" | ")
    }

    #[test]
    fn a_2d_browser_game_is_supported_and_a_3d_native_one_too() {
        assert_eq!(
            all(
                json!({"presentation": "2d", "platforms": ["web"], "networking": "offline", "input": ["keyboard", "mouse"], "persistence": ["settings", "progress"]})
            ),
            ""
        );
        assert_eq!(all(json!({"presentation": "3d", "platforms": ["windows", "linux"], "networking": "authoritative", "input": ["keyboard", "gamepad"]})), "");
    }

    /// The plausible mistakes, each refused with what RedEngine does support.
    #[test]
    fn unsupported_combinations_fail_early_with_the_way_out() {
        let cases = [
            ("3D in a browser", json!({"presentation": "3d", "platforms": ["web"]}), "3D cannot target `web`", "declare `presentation: \"2d\"`"),
            (
                "a browser server",
                json!({"presentation": "2d", "platforms": ["web"], "networking": "authoritative"}),
                "Browser target cannot use the native UDP transport",
                "Supported networking for web games: offline",
            ),
            (
                "2D netcode",
                json!({"presentation": "2d", "platforms": ["windows"], "networking": "authoritative"}),
                "2D cannot target `windows`",
                "native window for 2D games is not built yet",
            ),
            ("macOS", json!({"presentation": "3d", "platforms": ["macos"]}), "cannot target `macos`", "no macOS build"),
            (
                "touch on a native 3D game",
                json!({"presentation": "3d", "platforms": ["windows"], "input": ["touch"]}),
                "`touch` input on `windows` is not supported",
                "browser",
            ),
            (
                "a typo in a platform",
                json!({"presentation": "2d", "platforms": ["wev"]}),
                "capabilities.platforms[0]: `wev` is not one of",
                "did you mean `web`",
            ),
            ("a typo in the key", json!({"presentation": "2d", "platform": ["web"]}), "capabilities.platform: unknown field", "platforms"),
            ("no platforms", json!({"presentation": "2d"}), "needs `platforms`", "where the game must run"),
            ("empty platforms", json!({"presentation": "2d", "platforms": []}), "capabilities.platforms: is empty", "at least one"),
            ("a string for a list", json!({"presentation": "2d", "platforms": "web"}), "capabilities.platforms: expected a list", "got string"),
            (
                "a wrong presentation",
                json!({"presentation": "isometric", "platforms": ["web"]}),
                "capabilities.presentation: `isometric` is not one of 2d, 3d",
                "",
            ),
        ];
        for (what, v, a, b) in cases {
            let got = all(v);
            assert!(got.contains(a) && got.contains(b), "{what}: wanted `{a}` and `{b}` in `{got}`");
        }
    }

    #[test]
    fn unverified_input_is_allowed_but_warned_about() {
        let (c, p, r) = caps(json!({"presentation": "2d", "platforms": ["web"], "input": ["gamepad"]}));
        assert!(p.is_empty() && r.is_empty());
        let w = warnings(&c.unwrap());
        assert!(w.len() == 1 && w[0].contains("gamepad") && w[0].contains("unverified"), "{w:?}");
    }

    #[test]
    fn the_matrix_text_names_every_pair_and_never_calls_prepared_supported() {
        // Columns are padded for reading; compare with single spaces.
        let t = matrix_text().lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n");
        for (pres, plat) in [("2d", "web"), ("2d", "windows"), ("3d", "web"), ("3d", "windows"), ("3d", "macos")] {
            assert!(t.contains(&format!("{pres} on {plat}")), "{t}");
        }
        assert!(t.lines().any(|l| l.starts_with("2d on web") && l.contains("SUPPORTED")));
        assert!(t.lines().any(|l| l.starts_with("2d on windows") && l.contains("PREPARED")));
        assert!(t.lines().any(|l| l.starts_with("3d on web") && l.contains("NOT SUPPORTED")));
        assert!(!Support::Prepared("x").allowed() && !Support::NotSupported("x").allowed() && Support::Unverified("x").allowed());
    }
}
