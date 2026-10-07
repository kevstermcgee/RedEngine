//! The drivers and items of a racing game as validated data (ADR 2026-10-07-racing-rosters-are-authored-data).
//!
//! The engine owns the *mechanics* a kart can have ([`Ability`]: float over water, hop, glide, bulldoze, slipstream, build, ...), the three item behaviours ([`Item`]: a speed
//! burst, a thrown projectile, a shield) and the numbers every kart carries ([`KartSpec`]). A game owns who is driving: each driver's stable id, display name, colours, numbers
//! and ability, and what its three items are called. They are written in the map's `race.roster` (or left out for the Great Outdoors roster, which is the data file
//! `assets/racing/great-outdoors-roster.json` embedded here), so a new racing game defines its own characters without editing an engine enum.
//!
//! A [`Driver`] is a *wire number*, the index into the roster (`0..roster.len()`), exactly as it was when the roster was a fixed enum of eight. The list order is therefore part of
//! a game's network and save contract: reorder it and old values mean other drivers. Everything is bounded (at most [`MAX_DRIVERS`] drivers, every number inside a sane range) so a
//! typo cannot make an unbeatable or invisible kart.

use crate::sim::kart::{Ability, Item, KartSpec};
use serde_json::{Map, Value};
use std::sync::{Arc, OnceLock};

/// The most drivers a roster may have (a driver's number is one byte on the wire, and a race holds [`crate::sim::match_sim::MAX_PLAYERS`] karts).
pub const MAX_DRIVERS: usize = 16;

/// A driver: its number on the wire, which is its index in the [`Roster`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Driver(pub u8);

impl Driver {
    /// The driver's number on the wire (its index in the roster).
    pub fn wire(self) -> u8 {
        self.0
    }
}

/// One driver of a roster.
#[derive(Debug, Clone, PartialEq)]
pub struct DriverDef {
    /// Stable identifier: lowercase letters, digits and `_`/`-`. The model object a map may bring is `kart_<id>`.
    pub id: String,
    /// The name shown to players.
    pub name: String,
    /// One short line for the choice screen (capitals, at most 34 characters).
    pub blurb: String,
    /// The plain kart's colours when the map brings no art: `[body, head]` as `#rrggbb`.
    pub colors: [String; 2],
    /// The kart's numbers and ability.
    pub spec: KartSpec,
}

/// The drivers of a racing game and the display names of its items.
#[derive(Debug, Clone, PartialEq)]
pub struct Roster {
    drivers: Vec<DriverDef>,
    /// Names of the boost, projectile and shield items.
    items: [String; 3],
}

const GREAT_OUTDOORS: &str = include_str!("../../assets/racing/great-outdoors-roster.json");

/// The numbers a kart definition may carry, with the range each must lie in (`top_speed`, ...). Unknown keys are refused.
const KART_FIELDS: &[(&str, f64, f64)] = &[
    ("top_speed", 5.0, 60.0),
    ("accel", 1.0, 60.0),
    ("brake", 1.0, 80.0),
    ("reverse_speed", 0.0, 20.0),
    ("steer_rate", 20.0, 240.0),
    ("grip", 0.1, 1.0),
    ("mass", 0.3, 4.0),
    ("dirt", 0.1, 1.5),
    ("mud", 0.1, 1.5),
    ("water", 0.1, 1.5),
    ("boost_mult", 1.0, 3.0),
    ("drift_rate", 0.2, 3.0),
    ("hop_speed", 0.0, 10.0),
];

fn ident_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 24
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

fn color_ok(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn text<'a>(o: &'a Map<String, Value>, key: &str, path: &str, max: usize) -> Result<&'a str, String> {
    let s = o.get(key).and_then(Value::as_str).ok_or_else(|| format!("{path}.{key}: a text is required"))?;
    if s.trim().is_empty() || s.chars().count() > max || s.chars().any(char::is_control) {
        return Err(format!("{path}.{key}: 1..{max} printable characters, got {s:?}"));
    }
    Ok(s)
}

fn closed(o: &Map<String, Value>, path: &str, allowed: &[&str]) -> Result<(), String> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        None => Ok(()),
        Some(k) => {
            let hint = crate::suggest::suggest(k, allowed.iter().copied()).first().map(|n| format!(" - did you mean `{n}`?")).unwrap_or_default();
            Err(format!("{path}.{k}: not a field here{hint} (known: {})", allowed.join(", ")))
        }
    }
}

impl Roster {
    /// The Great Outdoors roster (`assets/racing/great-outdoors-roster.json`): what every race map without a `race.roster` uses.
    pub fn great_outdoors() -> Arc<Roster> {
        great_outdoors::shared().clone()
    }

    /// Parses and validates a roster (`{"drivers": [...], "items": {...}}`); `path` names it in messages (`race.roster`).
    pub fn from_json(v: &Value, path: &str) -> Result<Roster, String> {
        let o = v.as_object().ok_or_else(|| format!("{path}: an object with `drivers` is required"))?;
        closed(o, path, &["about", "drivers", "items"])?;
        let list = o.get("drivers").and_then(Value::as_array).ok_or_else(|| format!("{path}.drivers: give a list of drivers"))?;
        if list.is_empty() || list.len() > MAX_DRIVERS {
            return Err(format!("{path}.drivers: 1..{MAX_DRIVERS} drivers, got {}", list.len()));
        }
        let mut drivers: Vec<DriverDef> = Vec::new();
        for (i, d) in list.iter().enumerate() {
            let p = format!("{path}.drivers[{i}]");
            let d = d.as_object().ok_or_else(|| format!("{p}: an object is required"))?;
            closed(d, &p, &["id", "name", "blurb", "ability", "colors", "kart"])?;
            let id = text(d, "id", &p, 24)?;
            if !ident_ok(id) {
                return Err(format!("{p}.id: lowercase letters, digits, `_` and `-`, not starting with a digit, got {id:?}"));
            }
            if let Some(j) = drivers.iter().position(|x| x.id == id) {
                return Err(format!("{p}.id: `{id}` is already drivers[{j}]"));
            }
            let name = text(d, "name", &p, 16)?;
            if let Some(j) = drivers.iter().position(|x| x.name.eq_ignore_ascii_case(name)) {
                return Err(format!("{p}.name: `{name}` is already drivers[{j}]'s name"));
            }
            let ability_name = text(d, "ability", &p, 24)?;
            let ability = Ability::parse(ability_name).ok_or_else(|| {
                let hint = crate::suggest::suggest(ability_name, Ability::NAMES.iter().copied())
                    .first()
                    .map(|n| format!(" - did you mean `{n}`?"))
                    .unwrap_or_default();
                format!("{p}.ability: no ability `{ability_name}`{hint} (known: {})", Ability::NAMES.join(", "))
            })?;
            let blurb = match d.get("blurb") {
                None => ability.blurb().to_string(),
                Some(_) => {
                    let b = text(d, "blurb", &p, 34)?;
                    b.to_string()
                }
            };
            let colors = match d.get("colors") {
                None => ["#cccccc".to_string(), "#eeeeee".to_string()],
                Some(c) => {
                    let c = c.as_object().ok_or_else(|| format!("{p}.colors: {{\"body\": \"#rrggbb\", \"head\": \"#rrggbb\"}}"))?;
                    closed(c, &format!("{p}.colors"), &["body", "head"])?;
                    let one = |k: &str| -> Result<String, String> {
                        let s = c.get(k).and_then(Value::as_str).ok_or_else(|| format!("{p}.colors.{k}: a `#rrggbb` colour is required"))?;
                        if color_ok(s) {
                            Ok(s.to_string())
                        } else {
                            Err(format!("{p}.colors.{k}: `#rrggbb`, got {s:?}"))
                        }
                    };
                    [one("body")?, one("head")?]
                }
            };
            let kart = d.get("kart").and_then(Value::as_object).ok_or_else(|| format!("{p}.kart: the kart's numbers are required"))?;
            let kp = format!("{p}.kart");
            closed(kart, &kp, &KART_FIELDS.iter().map(|f| f.0).collect::<Vec<_>>())?;
            let mut n = [0f32; 13];
            for (k, (name, lo, hi)) in KART_FIELDS.iter().enumerate() {
                let x = kart.get(*name).and_then(Value::as_f64).ok_or_else(|| format!("{kp}.{name}: a number is required"))?;
                if !x.is_finite() || x < *lo || x > *hi {
                    return Err(format!("{kp}.{name}: must be {lo}..{hi}, got {x}"));
                }
                n[k] = x as f32;
            }
            let spec = KartSpec {
                top_speed: n[0],
                accel: n[1],
                brake: n[2],
                reverse_speed: n[3],
                steer_rate: n[4],
                grip: n[5],
                mass: n[6],
                dirt: n[7],
                mud: n[8],
                water: n[9],
                boost_mult: n[10],
                drift_rate: n[11],
                hop_speed: n[12],
                ability,
            };
            drivers.push(DriverDef { id: id.to_string(), name: name.to_string(), blurb, colors, spec });
        }
        let mut items = [Item::Mushroom.name().to_string(), Item::Acorn.name().to_string(), Item::Bubble.name().to_string()];
        if let Some(it) = o.get("items") {
            let ip = format!("{path}.items");
            let it = it.as_object().ok_or_else(|| format!("{ip}: {{\"boost\": \"..\", \"projectile\": \"..\", \"shield\": \"..\"}}"))?;
            closed(it, &ip, &["boost", "projectile", "shield"])?;
            for (k, key) in ["boost", "projectile", "shield"].iter().enumerate() {
                if it.contains_key(*key) {
                    items[k] = text(it, key, &ip, 12)?.to_string();
                }
            }
        }
        Ok(Roster { drivers, items })
    }

    /// How many drivers there are.
    pub fn len(&self) -> usize {
        self.drivers.len()
    }

    /// Whether there are none (never true for a parsed roster).
    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }

    /// Every driver, in wire order.
    pub fn drivers(&self) -> impl Iterator<Item = Driver> + '_ {
        (0..self.drivers.len() as u8).map(Driver)
    }

    /// The definition of `driver`, if it is in this roster.
    pub fn get(&self, driver: Driver) -> Option<&DriverDef> {
        self.drivers.get(driver.0 as usize)
    }

    /// The driver for a wire number, if it is one.
    pub fn from_wire(&self, n: u8) -> Option<Driver> {
        ((n as usize) < self.drivers.len()).then_some(Driver(n))
    }

    /// The first driver, which a bad wire number falls back to.
    pub fn first(&self) -> Driver {
        Driver(0)
    }

    /// The driver whose name or id is `name` (any case).
    pub fn parse(&self, name: &str) -> Option<Driver> {
        let n = name.trim();
        self.drivers.iter().position(|d| d.name.eq_ignore_ascii_case(n) || d.id.eq_ignore_ascii_case(n)).map(|i| Driver(i as u8))
    }

    /// The names of all drivers, comma separated (for messages).
    pub fn names(&self) -> String {
        self.drivers.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// The driver's name as shown to players (`?` outside the roster).
    pub fn name(&self, driver: Driver) -> &str {
        self.get(driver).map_or("?", |d| d.name.as_str())
    }

    /// The driver's stable id (`?` outside the roster).
    pub fn id(&self, driver: Driver) -> &str {
        self.get(driver).map_or("?", |d| d.id.as_str())
    }

    /// The driver's kart numbers (the first driver's for a number outside the roster, so a corrupt byte can never panic a tick).
    pub fn spec(&self, driver: Driver) -> KartSpec {
        self.get(driver).unwrap_or(&self.drivers[0]).spec
    }

    /// The choice-screen line for the driver.
    pub fn blurb(&self, driver: Driver) -> &str {
        self.get(driver).map_or("", |d| d.blurb.as_str())
    }

    /// The plain kart's `[body, head]` colours.
    pub fn colors(&self, driver: Driver) -> &[String; 2] {
        &self.get(driver).unwrap_or(&self.drivers[0]).colors
    }

    /// The name of an item as shown to players.
    pub fn item_name(&self, item: Item) -> &str {
        match item {
            Item::None => "none",
            Item::Mushroom => &self.items[0],
            Item::Acorn => &self.items[1],
            Item::Bubble => &self.items[2],
        }
    }
}

/// The Great Outdoors roster as a shared static, with the wire numbers of its eight animals as named constants for the game's own content, tools and tests. Nothing in the
/// engine's simulation, protocol or UI names an animal: a race map without `race.roster` simply gets this one.
pub mod great_outdoors {
    use super::*;

    /// Duck: balanced, floats over water.
    pub const DUCK: Driver = Driver(0);
    /// Bunny: quick off the line, hops.
    pub const BUNNY: Driver = Driver(1);
    /// Deer: highest top speed, drifts well.
    pub const DEER: Driver = Driver(2);
    /// Coyote: fine on dirt.
    pub const COYOTE: Driver = Driver(3);
    /// Hawk: light, glides.
    pub const HAWK: Driver = Driver(4);
    /// Bear: heavy, bumps others aside.
    pub const BEAR: Driver = Driver(5);
    /// Wolf: draws a boost from slipstreams.
    pub const WOLF: Driver = Driver(6);
    /// Beaver: wooden kart, builds.
    pub const BEAVER: Driver = Driver(7);

    /// The built-in roster.
    pub fn shared() -> &'static Arc<Roster> {
        static R: OnceLock<Arc<Roster>> = OnceLock::new();
        R.get_or_init(|| match serde_json::from_str::<Value>(GREAT_OUTDOORS).map_err(|e| e.to_string()).and_then(|v| Roster::from_json(&v, "roster")) {
            Ok(r) => Arc::new(r),
            // The file is this crate's own constant and a test parses it: a mistake fails a test, never a race.
            #[allow(clippy::panic)]
            Err(e) => panic!("built-in Great Outdoors roster: {e}"),
        })
    }

    /// The eight drivers in wire order.
    pub fn all() -> Vec<Driver> {
        shared().drivers().collect()
    }

    /// A Great Outdoors driver's kart numbers.
    pub fn spec(d: Driver) -> KartSpec {
        shared().spec(d)
    }

    /// A Great Outdoors driver's name.
    pub fn name(d: Driver) -> &'static str {
        shared().get(d).map_or("?", |x| x.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The numbers the eight drivers had when they were a Rust enum (`Driver::spec`): the embedded data file must reproduce them exactly.
    #[test]
    fn the_great_outdoors_roster_keeps_the_numbers_and_wire_order_the_enum_had() {
        let r = Roster::great_outdoors();
        assert_eq!(r.names(), "Duck, Bunny, Deer, Coyote, Hawk, Bear, Wolf, Beaver");
        assert_eq!((0..8).map(|i| r.name(Driver(i)).to_string()).collect::<Vec<_>>(), ["Duck", "Bunny", "Deer", "Coyote", "Hawk", "Bear", "Wolf", "Beaver"]);
        let base = KartSpec {
            top_speed: 24.0,
            accel: 12.0,
            brake: 26.0,
            reverse_speed: 6.0,
            steer_rate: 90.0,
            grip: 0.8,
            mass: 1.0,
            dirt: 0.85,
            mud: 0.6,
            water: 0.55,
            boost_mult: 1.3,
            drift_rate: 1.0,
            hop_speed: 4.0,
            ability: Ability::Float,
        };
        let want = [
            KartSpec { water: 1.0, ability: Ability::Float, ..base },
            KartSpec { top_speed: 22.0, accel: 16.0, steer_rate: 110.0, grip: 0.75, mass: 0.8, drift_rate: 1.2, hop_speed: 5.5, ability: Ability::Hop, ..base },
            KartSpec { top_speed: 27.0, accel: 10.0, steer_rate: 85.0, mass: 0.9, drift_rate: 1.3, ability: Ability::DriftBoost, ..base },
            KartSpec { top_speed: 25.0, steer_rate: 90.0, grip: 0.7, mass: 0.9, dirt: 1.0, ability: Ability::Dirt, ..base },
            KartSpec { top_speed: 25.0, accel: 11.0, mass: 0.7, ability: Ability::Glide, ..base },
            KartSpec {
                top_speed: 22.0,
                accel: 9.0,
                steer_rate: 70.0,
                grip: 0.85,
                mass: 1.6,
                drift_rate: 0.8,
                hop_speed: 3.0,
                ability: Ability::Bulldoze,
                ..base
            },
            KartSpec { top_speed: 25.0, steer_rate: 92.0, grip: 0.78, ability: Ability::Slipstream, ..base },
            KartSpec {
                top_speed: 21.0,
                accel: 9.0,
                steer_rate: 78.0,
                grip: 0.85,
                mass: 1.3,
                dirt: 1.0,
                mud: 1.0,
                water: 1.0,
                drift_rate: 0.9,
                ability: Ability::Build,
                ..base
            },
        ];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(r.spec(Driver(i as u8)), *w, "driver {i} ({})", r.name(Driver(i as u8)));
        }
        assert_eq!(r.id(Driver(3)), "coyote");
        assert_eq!(r.item_name(Item::Mushroom), "Mushroom");
        assert_eq!([Item::Mushroom.wire(), Item::Acorn.wire(), Item::Bubble.wire()], [1, 2, 3], "item wire numbers are part of the protocol");
        assert_eq!(r.parse("BEAVER"), Some(Driver(7)));
        assert_eq!(r.from_wire(8), None);
    }

    fn two(extra: Value) -> Value {
        let mut d = json!({"drivers": [
            {"id": "ace", "name": "Ace", "ability": "hop", "kart": {"top_speed": 30.0, "accel": 14.0, "brake": 26.0, "reverse_speed": 6.0, "steer_rate": 95.0, "grip": 0.8, "mass": 1.0, "dirt": 0.8, "mud": 0.6, "water": 0.5, "boost_mult": 1.3, "drift_rate": 1.0, "hop_speed": 4.0}},
            {"id": "tank", "name": "Tank", "ability": "bulldoze", "blurb": "HEAVY AND PROUD", "colors": {"body": "#445566", "head": "#99aabb"}, "kart": {"top_speed": 20.0, "accel": 8.0, "brake": 26.0, "reverse_speed": 6.0, "steer_rate": 70.0, "grip": 0.9, "mass": 2.0, "dirt": 0.8, "mud": 0.6, "water": 0.5, "boost_mult": 1.3, "drift_rate": 0.8, "hop_speed": 3.0}}]});
        if let (Some(m), Some(e)) = (d.as_object_mut(), extra.as_object()) {
            m.extend(e.clone());
        }
        d
    }

    #[test]
    fn a_small_alternative_roster_is_valid_and_its_items_can_be_renamed() {
        let r = Roster::from_json(&two(json!({"items": {"boost": "Turbo", "shield": "Barrier"}})), "race.roster").unwrap();
        assert_eq!((r.len(), r.name(Driver(1)), r.blurb(Driver(1)), r.id(Driver(0))), (2, "Tank", "HEAVY AND PROUD", "ace"));
        assert_eq!(r.blurb(Driver(0)), Ability::Hop.blurb(), "no blurb: the ability's own line");
        assert_eq!((r.item_name(Item::Mushroom), r.item_name(Item::Acorn), r.item_name(Item::Bubble)), ("Turbo", "Acorn", "Barrier"));
        assert_eq!(r.spec(Driver(1)).mass, 2.0);
        assert_eq!(r.spec(Driver(9)).top_speed, 30.0, "outside the roster is the first driver, never a panic");
        assert_eq!(r.from_wire(2), None);
    }

    #[test]
    fn mistakes_name_the_path_the_range_and_what_exists() {
        let bad = |edit: fn(&mut Value)| {
            let mut v = two(json!({}));
            edit(&mut v);
            Roster::from_json(&v, "race.roster").unwrap_err()
        };
        assert!(bad(|v| v["drivers"][0]["kart"]["top_speed"] = json!(900.0)).contains("race.roster.drivers[0].kart.top_speed: must be 5..60"));
        assert!(bad(|v| v["drivers"][0]["kart"]["topspeed"] = json!(9.0)).contains("did you mean `top_speed`"));
        assert!(bad(|v| v["drivers"][0]["kart"].as_object_mut().unwrap().remove("grip").map(|_| ()).unwrap_or(())).contains("kart.grip: a number is required"));
        assert!(bad(|v| v["drivers"][1]["ability"] = json!("glid")).contains("did you mean `glide`"));
        assert!(bad(|v| v["drivers"][1]["id"] = json!("ace")).contains("already drivers[0]"));
        assert!(bad(|v| v["drivers"][1]["id"] = json!("Bad Id")).contains(".id: lowercase"));
        assert!(bad(|v| v["drivers"][1]["name"] = json!("ACE")).contains("already drivers[0]'s name"));
        assert!(bad(|v| v["drivers"][1]["colors"]["body"] = json!("red")).contains("`#rrggbb`"));
        assert!(bad(|v| v["drivers"] = json!([])).contains("1..16 drivers"));
        assert!(bad(|v| v["drivers"][1]["blurb"] = json!("A".repeat(40))).contains("1..34"));
        assert!(bad(|v| v["roster"] = json!(1)).contains("not a field here"));
        let mut many = two(json!({}));
        let first = many["drivers"][0].clone();
        many["drivers"] = Value::Array(
            (0..17)
                .map(|i| {
                    let mut d = first.clone();
                    d["id"] = json!(format!("d{i}"));
                    d["name"] = json!(format!("D{i}"));
                    d
                })
                .collect(),
        );
        assert!(Roster::from_json(&many, "r").unwrap_err().contains("got 17"));
    }
}
