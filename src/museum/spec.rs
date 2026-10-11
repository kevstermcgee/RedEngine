//! The `museum` scene block and the collection file it names: what a museum is made of, as data.
//!
//! ```json
//! "museum": {"collection": "collection.json", "art": "shaders/art.wgsl", "seed": "launch"}
//! ```
//!
//! The collection holds everything that makes the museum a particular museum: the art styles its shader knows (by name, in the shader's order), the
//! emotional themes of its wings (colours, palettes, which styles and sculpture families they favour, which music they lean toward) and the words its
//! pieces are written with (titles, origin stories, artists, media, curator's hints), plus a few hand-written masterpieces. The engine supplies the
//! architecture, the sculpture families and the rules (doors, keys, paintings within paintings); see [`super`].

use super::sculpt::Family;
use crate::atmosphere::LaunchSeed;
use crate::fields::check_keys;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// `museum` keys.
pub const MUSEUM_KEYS: &[&str] = &["collection", "art", "seed", "start_theme"];
/// Collection root keys.
pub const COLLECTION_KEYS: &[&str] = &["styles", "materials", "words", "artists", "media", "stories", "object_stories", "themes", "masterpieces"];
/// `themes[]` keys.
pub const THEME_KEYS: &[&str] = &["id", "name", "wing", "colors", "palettes", "styles", "sculptures", "music", "words", "titles", "stories", "hints"];
/// `themes[].colors` keys.
pub const THEME_COLOR_KEYS: &[&str] = &["wall", "floor", "trim", "ceiling", "light"];
/// `media` keys: one list of media per kind of piece.
pub const MEDIA_KEYS: &[&str] = &["painting", "tapestry", "sculpture", "installation"];
/// `masterpieces[]` keys.
pub const MASTERPIECE_KEYS: &[&str] = &["title", "artist", "year", "medium", "story", "theme", "style"];

/// A colour as linear RGB.
pub type Rgb = [f32; 3];

/// The scene's museum: the collection, its seed and the game's painted-surface shader.
#[derive(Debug, Clone)]
pub struct MuseumSpec {
    /// The collection file, as the scene names it.
    pub collection_file: String,
    /// What the museum is made of.
    pub collection: Collection,
    /// The painted-surface shader file (`art`), as the scene names it.
    pub art_file: Option<String>,
    /// Its WGSL (`fn surface(s: SurfaceIn) -> vec4<f32>`), composed into the object shader and in front of the sky.
    pub art: Option<String>,
    /// The museum's seed: the same seed is the same building, the same art and the same words.
    pub seed: LaunchSeed,
    /// The theme of the first room (by index), when the scene names one.
    pub start_theme: Option<usize>,
}

/// A wing's mood: how its rooms look and sound, and what its art is about.
#[derive(Debug, Clone)]
pub struct Theme {
    pub id: String,
    /// The emotion, one word ("Longing").
    pub name: String,
    /// The wing's name on the map and at its doors ("The Tidewater Wing").
    pub wing: String,
    pub wall: Rgb,
    pub floor: Rgb,
    pub trim: Rgb,
    pub ceiling: Rgb,
    /// The colour of the rooms' light.
    pub light: Rgb,
    /// Four-colour palettes its paintings draw from.
    pub palettes: Vec<[Rgb; 4]>,
    /// Art styles it favours (indices into [`Collection::styles`]).
    pub styles: Vec<usize>,
    /// Sculpture and installation families it favours.
    pub sculptures: Vec<Family>,
    /// Which of the journey's scores (indices into `audio.journey.scores`) its rooms lean toward.
    pub music: Vec<usize>,
    /// Its own word lists, tried before the collection's.
    pub words: BTreeMap<String, Vec<String>>,
    /// Title templates.
    pub titles: Vec<String>,
    /// Origin-story templates.
    pub stories: Vec<String>,
    /// Curator's hints for the piece that keeps a room's key (`{title}`, `{artist}`, `{kind}` name it).
    pub hints: Vec<String>,
}

/// A hand-written piece, hung once in a while instead of a generated one.
#[derive(Debug, Clone)]
pub struct Masterpiece {
    pub title: String,
    pub artist: String,
    pub year: String,
    pub medium: String,
    pub story: String,
    pub theme: usize,
    pub style: usize,
}

/// Everything a museum is made of (the collection file).
#[derive(Debug, Clone)]
pub struct Collection {
    /// Art style names, in the order the shader numbers them (style `i` is `i + 1` in `SurfaceIn.params.y`).
    pub styles: Vec<String>,
    /// Sculpture material names, in the order the shader numbers them (material `i` is `i + 1` in `SurfaceIn.params.y` of a solid).
    pub materials: Vec<String>,
    /// Shared word lists (`{first}`, `{city}`, ...).
    pub words: BTreeMap<String, Vec<String>>,
    /// Artist-name templates ("{first} {last}", "Anonymous ({epithet}) // {epithet}"): the label, and after ` // ` how a story names them (the
    /// label when there is none).
    pub artists: Vec<String>,
    /// Origin stories any theme's paintings may be given (they may use the theme's words).
    pub stories: Vec<String>,
    /// Origin stories for sculptures, installations and tapestries (the theme's own stories are about paintings).
    pub object_stories: Vec<String>,
    /// Media per kind of piece.
    pub media: BTreeMap<String, Vec<String>>,
    pub themes: Vec<Theme>,
    pub masterpieces: Vec<Masterpiece>,
}

fn strings(errs: &mut Vec<String>, path: &str, v: Option<&Value>, required: bool) -> Vec<String> {
    match v {
        None if !required => Vec::new(),
        None => {
            errs.push(format!("{path}: missing (a list of strings)"));
            Vec::new()
        }
        Some(Value::Array(a)) => {
            let out: Vec<String> = a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            if out.len() != a.len() {
                errs.push(format!("{path}: every entry must be a string"));
            }
            if required && out.is_empty() {
                errs.push(format!("{path}: must not be empty"));
            }
            out
        }
        Some(_) => {
            errs.push(format!("{path}: must be a list of strings"));
            Vec::new()
        }
    }
}

fn word_lists(errs: &mut Vec<String>, path: &str, v: Option<&Value>) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    match v {
        None => {}
        Some(Value::Object(o)) => {
            for (k, list) in o {
                if crate::fields::is_extension_key(k) {
                    continue;
                }
                let l = strings(errs, &format!("{path}.{k}"), Some(list), true);
                out.insert(k.clone(), l);
            }
        }
        Some(_) => errs.push(format!("{path}: must be an object of word lists, like {{\"noun\": [\"tide\", \"lamp\"]}}")),
    }
    out
}

fn colour(errs: &mut Vec<String>, path: &str, v: Option<&Value>) -> Rgb {
    match v.and_then(Value::as_str).map(crate::color::parse_hex_to_linear) {
        Some(Ok(c)) => c.to_array(),
        Some(Err(e)) => {
            errs.push(format!("{path}: {e}"));
            [1.0, 0.0, 1.0]
        }
        None => {
            errs.push(format!("{path}: missing (a colour like \"#a0b0c0\")"));
            [1.0, 0.0, 1.0]
        }
    }
}

fn index_of(errs: &mut Vec<String>, path: &str, name: &str, names: &[String], what: &str) -> usize {
    match names.iter().position(|n| n == name) {
        Some(i) => i,
        None => {
            let near = crate::suggest::suggest(name, names.iter().map(String::as_str));
            let hint = if near.is_empty() { String::new() } else { format!(" (did you mean `{}`?)", near[0]) };
            errs.push(format!("{path}: `{name}` is not one of the collection's {what}{hint}"));
            0
        }
    }
}

/// Every `{slot}` a template uses (inline choices `{a|b}` and literal braces excluded).
pub fn slots(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else { break };
        let inner = &rest[open + 1..open + close];
        if !inner.contains('|') && !inner.is_empty() {
            out.push(inner.to_ascii_lowercase());
        }
        rest = &rest[open + close + 1..];
    }
    out
}

/// Slots filled by the engine itself rather than a word list.
pub const ENGINE_SLOTS: &[&str] = &["title", "artist", "kind", "year", "when", "later", "theme", "wing", "medium", "n", "room"];

/// Parses a collection file's JSON.
pub fn parse_collection(v: &Value, journey_scores: Option<usize>) -> Result<Collection, Vec<String>> {
    let mut errs = Vec::new();
    let Some(root) = v.as_object() else { return Err(vec!["collection: must be a JSON object".to_string()]) };
    check_keys(&mut errs, "collection", root, COLLECTION_KEYS);
    let styles = strings(&mut errs, "collection.styles", root.get("styles"), true);
    let materials = strings(&mut errs, "collection.materials", root.get("materials"), true);
    let words = word_lists(&mut errs, "collection.words", root.get("words"));
    let artists = strings(&mut errs, "collection.artists", root.get("artists"), true);
    let stories = strings(&mut errs, "collection.stories", root.get("stories"), false);
    let object_stories = strings(&mut errs, "collection.object_stories", root.get("object_stories"), true);
    let mut media = BTreeMap::new();
    match root.get("media").and_then(Value::as_object) {
        Some(m) => {
            check_keys(&mut errs, "collection.media", m, MEDIA_KEYS);
            for k in MEDIA_KEYS {
                media.insert(k.to_string(), strings(&mut errs, &format!("collection.media.{k}"), m.get(*k), true));
            }
        }
        None => errs.push("collection.media: missing (an object with painting, tapestry, sculpture and installation lists)".to_string()),
    }
    let mut themes = Vec::new();
    let theme_ids: Vec<String> = root
        .get("themes")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|t| t.get("id").and_then(Value::as_str).map(str::to_string)).collect())
        .unwrap_or_default();
    match root.get("themes").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => {
            for (i, t) in list.iter().enumerate() {
                let path = format!("collection.themes[{i}]");
                let Some(t) = t.as_object() else {
                    errs.push(format!("{path}: must be an object"));
                    continue;
                };
                check_keys(&mut errs, &path, t, THEME_KEYS);
                let text = |errs: &mut Vec<String>, k: &str| match t.get(k).and_then(Value::as_str) {
                    Some(s) if !s.trim().is_empty() => s.to_string(),
                    _ => {
                        errs.push(format!("{path}.{k}: missing (a string)"));
                        String::new()
                    }
                };
                let id = text(&mut errs, "id");
                let name = text(&mut errs, "name");
                let wing = text(&mut errs, "wing");
                let empty = Map::new();
                let colors = t.get("colors").and_then(Value::as_object).unwrap_or(&empty);
                if t.get("colors").is_none() {
                    errs.push(format!("{path}.colors: missing (wall, floor, trim, ceiling, light)"));
                }
                check_keys(&mut errs, &format!("{path}.colors"), colors, THEME_COLOR_KEYS);
                let c = |errs: &mut Vec<String>, k: &str| colour(errs, &format!("{path}.colors.{k}"), colors.get(k));
                let (wall, floor, trim, ceiling, light) =
                    (c(&mut errs, "wall"), c(&mut errs, "floor"), c(&mut errs, "trim"), c(&mut errs, "ceiling"), c(&mut errs, "light"));
                let mut palettes = Vec::new();
                match t.get("palettes").and_then(Value::as_array) {
                    Some(ps) if !ps.is_empty() => {
                        for (j, p) in ps.iter().enumerate() {
                            match p.as_array() {
                                Some(cs) if cs.len() == 4 => {
                                    let mut pal = [[0.0; 3]; 4];
                                    for (k, cv) in cs.iter().enumerate() {
                                        pal[k] = colour(&mut errs, &format!("{path}.palettes[{j}][{k}]"), Some(cv));
                                    }
                                    palettes.push(pal);
                                }
                                _ => errs.push(format!("{path}.palettes[{j}]: four colours, darkest first")),
                            }
                        }
                    }
                    _ => errs.push(format!("{path}.palettes: a non-empty list of four-colour palettes")),
                }
                let style_names = strings(&mut errs, &format!("{path}.styles"), t.get("styles"), true);
                let styles_ix: Vec<usize> =
                    style_names.iter().enumerate().map(|(j, s)| index_of(&mut errs, &format!("{path}.styles[{j}]"), s, &styles, "styles")).collect();
                let fam_names = strings(&mut errs, &format!("{path}.sculptures"), t.get("sculptures"), true);
                let mut sculptures = Vec::new();
                for (j, f) in fam_names.iter().enumerate() {
                    match Family::from_name(f) {
                        Some(fam) => sculptures.push(fam),
                        None => {
                            let names: Vec<&str> = Family::ALL.iter().map(|f| f.name()).collect();
                            errs.push(format!("{path}.sculptures[{j}]: `{f}` is not a sculpture family (known: {})", names.join(", ")));
                        }
                    }
                }
                let mut music = Vec::new();
                match t.get("music") {
                    None => {}
                    Some(Value::Array(a)) => {
                        for (j, m) in a.iter().enumerate() {
                            match m.as_u64() {
                                Some(n) if journey_scores.is_none_or(|k| (n as usize) < k) => music.push(n as usize),
                                _ => errs.push(format!(
                                    "{path}.music[{j}]: the index of one of `audio.journey.scores` (0..{})",
                                    journey_scores.map_or("?".to_string(), |k| k.saturating_sub(1).to_string())
                                )),
                            }
                        }
                    }
                    Some(_) => errs.push(format!("{path}.music: a list of indices into audio.journey.scores")),
                }
                let words = word_lists(&mut errs, &format!("{path}.words"), t.get("words"));
                let titles = strings(&mut errs, &format!("{path}.titles"), t.get("titles"), true);
                let stories = strings(&mut errs, &format!("{path}.stories"), t.get("stories"), true);
                let hints = strings(&mut errs, &format!("{path}.hints"), t.get("hints"), true);
                themes.push(Theme {
                    id,
                    name,
                    wing,
                    wall,
                    floor,
                    trim,
                    ceiling,
                    light,
                    palettes,
                    styles: styles_ix,
                    sculptures,
                    music,
                    words,
                    titles,
                    stories,
                    hints,
                });
            }
        }
        _ => errs.push("collection.themes: a non-empty list of themes".to_string()),
    }
    let mut masterpieces = Vec::new();
    if let Some(list) = root.get("masterpieces") {
        match list.as_array() {
            Some(a) => {
                for (i, m) in a.iter().enumerate() {
                    let path = format!("collection.masterpieces[{i}]");
                    let Some(m) = m.as_object() else {
                        errs.push(format!("{path}: must be an object"));
                        continue;
                    };
                    check_keys(&mut errs, &path, m, MASTERPIECE_KEYS);
                    let s = |errs: &mut Vec<String>, k: &str| match m.get(k).and_then(Value::as_str) {
                        Some(v) => v.to_string(),
                        None => {
                            errs.push(format!("{path}.{k}: missing (a string)"));
                            String::new()
                        }
                    };
                    let (title, artist, year, medium, story) =
                        (s(&mut errs, "title"), s(&mut errs, "artist"), s(&mut errs, "year"), s(&mut errs, "medium"), s(&mut errs, "story"));
                    let theme_name = s(&mut errs, "theme");
                    let style_name = s(&mut errs, "style");
                    let theme = index_of(&mut errs, &format!("{path}.theme"), &theme_name, &theme_ids, "themes");
                    let style = index_of(&mut errs, &format!("{path}.style"), &style_name, &styles, "styles");
                    masterpieces.push(Masterpiece { title, artist, year, medium, story, theme, style });
                }
            }
            None => errs.push("collection.masterpieces: must be a list".to_string()),
        }
    }
    let collection = Collection { styles, materials, words, artists, stories, object_stories, media, themes, masterpieces };
    check_templates(&collection, &mut errs);
    if errs.is_empty() {
        Ok(collection)
    } else {
        Err(errs)
    }
}

/// Every slot a template names must be a word list (the theme's or the collection's) or one the engine fills; word lists may name slots too.
fn check_templates(c: &Collection, errs: &mut Vec<String>) {
    for (ti, t) in c.themes.iter().enumerate() {
        let known = |slot: &str| ENGINE_SLOTS.contains(&slot) || t.words.contains_key(slot) || c.words.contains_key(slot);
        let lists = [("titles", &t.titles), ("stories", &t.stories), ("hints", &t.hints)];
        let mut check = |path: String, tpl: &str| {
            for s in slots(tpl) {
                if !known(&s) {
                    errs.push(format!("{path}: `{{{s}}}` names no word list (add `words.{s}` to the theme or the collection)"));
                }
            }
        };
        for (name, list) in lists {
            for (i, tpl) in list.iter().enumerate() {
                check(format!("collection.themes[{ti}].{name}[{i}]"), tpl);
            }
        }
        for (k, list) in t.words.iter().chain(c.words.iter()) {
            for (i, tpl) in list.iter().enumerate() {
                check(format!("collection.themes[{ti}] words.{k}[{i}]"), tpl);
            }
        }
        for (i, tpl) in c.artists.iter().enumerate() {
            check(format!("collection.artists[{i}]"), tpl);
        }
        for (i, tpl) in c.stories.iter().enumerate() {
            check(format!("collection.stories[{i}]"), tpl);
        }
        for (i, tpl) in c.object_stories.iter().enumerate() {
            check(format!("collection.object_stories[{i}]"), tpl);
        }
    }
}

/// Parses the scene's `museum` block, reading the collection and the art shader next to the scene (`base`; `None` only checks the keys).
pub fn parse_museum(root: &Map<String, Value>, base: Option<&std::path::Path>) -> Result<Option<MuseumSpec>, Vec<String>> {
    let Some(v) = root.get("museum") else { return Ok(None) };
    let Some(o) = v.as_object() else {
        return Err(vec!["museum: must be an object like {\"collection\": \"collection.json\", \"art\": \"shaders/art.wgsl\"}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "museum", o, MUSEUM_KEYS);
    let seed = LaunchSeed::parse(o.get("seed"), "museum.seed").unwrap_or_else(|e| {
        errs.push(e);
        LaunchSeed::Fixed(0)
    });
    let collection_file = match o.get("collection").and_then(Value::as_str) {
        Some(f) if f.ends_with(".json") => f.to_string(),
        _ => {
            errs.push("museum.collection: the path of the collection file next to the scene, like \"collection.json\"".to_string());
            String::new()
        }
    };
    let art_file = match o.get("art") {
        None => None,
        Some(Value::String(f)) if f.ends_with(".wgsl") => Some(f.clone()),
        Some(_) => {
            errs.push("museum.art: the path of a .wgsl file next to the scene, like \"shaders/art.wgsl\"".to_string());
            None
        }
    };
    let journey_scores = root.get("audio").and_then(|a| a.get("journey")).and_then(|j| j.get("scores")).and_then(Value::as_array).map(Vec::len);
    let mut collection = Collection {
        styles: Vec::new(),
        materials: Vec::new(),
        words: BTreeMap::new(),
        artists: Vec::new(),
        stories: Vec::new(),
        object_stories: Vec::new(),
        media: BTreeMap::new(),
        themes: Vec::new(),
        masterpieces: Vec::new(),
    };
    let mut art = None;
    if let Some(base) = base {
        if !collection_file.is_empty() {
            let path = base.join(&collection_file);
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())) {
                Ok(v) => match parse_collection(&v, journey_scores) {
                    Ok(c) => collection = c,
                    Err(e) => errs.extend(e.into_iter().map(|m| format!("museum.collection ({collection_file}): {m}"))),
                },
                Err(e) => errs.push(format!("museum.collection: `{collection_file}` cannot be read (looked for {}): {e}", path.display())),
            }
        }
        if let Some(f) = &art_file {
            let path = base.join(f);
            match std::fs::read_to_string(&path) {
                Ok(code) => match crate::atmosphere::check_art_shader(f, &code) {
                    Ok(()) => art = Some(code),
                    Err(e) => errs.push(e),
                },
                Err(e) => errs.push(format!("museum.art: `{f}` cannot be read (looked for {}): {e}", path.display())),
            }
        }
    }
    let start_theme = match o.get("start_theme") {
        None => None,
        Some(Value::String(id)) => match collection.themes.iter().position(|t| &t.id == id) {
            Some(i) => Some(i),
            None if base.is_none() => None,
            None => {
                errs.push(format!("museum.start_theme: `{id}` is not one of the collection's themes"));
                None
            }
        },
        Some(_) => {
            errs.push("museum.start_theme: a theme id".to_string());
            None
        }
    };
    if errs.is_empty() {
        Ok(Some(MuseumSpec { collection_file, collection, art_file, art, seed, start_theme }))
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn tiny() -> Value {
        json!({
            "styles": ["ink", "field"],
            "materials": ["marble", "bronze"],
            "words": {"first": ["Ada", "Ines"], "last": ["Moreau", "Kalnins"], "epithet": ["the Salt Hand"], "noun": ["lamp"], "city": ["Riga"]},
            "artists": ["{first} {last}", "Anonymous ({epithet}) // {epithet}"],
            "object_stories": ["{artist} made it."],
            "media": {"painting": ["oil on linen"], "tapestry": ["wool"], "sculpture": ["bronze"], "installation": ["glass and wire"]},
            "themes": [{
                "id": "longing", "name": "Longing", "wing": "The Tidewater Wing",
                "colors": {"wall": "#33445a", "floor": "#2a2420", "trim": "#c8a060", "ceiling": "#202830", "light": "#ffd8a8"},
                "palettes": [["#101820", "#305070", "#80a0c0", "#f0e0c0"]],
                "styles": ["ink"], "sculptures": ["vessel", "figure"], "music": [0],
                "words": {"adj": ["far"]},
                "titles": ["The {adj} {noun}"],
                "stories": ["{artist} made it in {city}, in {year}."],
                "hints": ["Ask {title}."]
            }]
        })
    }

    #[test]
    fn a_small_collection_parses_and_its_templates_resolve() {
        let c = parse_collection(&tiny(), Some(1)).expect("parses");
        assert_eq!(c.themes.len(), 1);
        assert_eq!(c.themes[0].styles, vec![0]);
        assert_eq!(c.themes[0].sculptures, vec![Family::Vessel, Family::Figure]);
    }

    #[test]
    fn an_unknown_slot_style_or_family_is_an_error_with_its_path() {
        let mut v = tiny();
        v["themes"][0]["titles"] = json!(["The {colour} {noun}"]);
        v["themes"][0]["styles"] = json!(["inc"]);
        v["themes"][0]["sculptures"] = json!(["vase"]);
        v["themes"][0]["music"] = json!([3]);
        let errs = parse_collection(&v, Some(1)).unwrap_err().join("\n");
        assert!(errs.contains("titles[0]: `{colour}` names no word list"), "{errs}");
        assert!(errs.contains("styles[0]: `inc` is not one of the collection's styles (did you mean `ink`?)"), "{errs}");
        assert!(errs.contains("sculptures[0]: `vase` is not a sculpture family"), "{errs}");
        assert!(errs.contains("music[0]"), "{errs}");
    }

    #[test]
    fn slots_skip_inline_choices() {
        assert_eq!(slots("{Adj} {a|b} {noun}"), vec!["adj", "noun"]);
    }
}
