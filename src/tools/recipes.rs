//! Recipes: complete, known-good example maps to imitate. Each file in `recipes/` is a real scene
//! (embedded in the binary) whose top-level `"recipe"` block says what it teaches. They are held
//! to the same bar as user maps: `tests::every_recipe_is_lint_clean_and_passes_its_checks`
//! validates, lints and runs the `checks` of every recipe, so a recipe can never rot.
//!
//! `red_engine2 recipe` lists them; `recipe <name>` explains one; `recipe <name> --new out.json`
//! copies it as a starting point; `recipe <name> --print` dumps the JSON.
//!
//! The same front door also holds the **2D mechanics** (`examples/patterns/*.game2d.json`): each is a complete tiny browser game that shows ONE reusable mechanic (a key and a door,
//! a countdown that ends the game, checkpoints, a capped spawner, a whole collect-power-survive-escape game) together with the scenarios that prove it works. An agent asked for
//! "a door that opens with a key" finds it by name or by search, copies it, and changes the look: it does not re-derive the rules or invent how to test them.

use serde_json::Value;

/// Embedded recipe files by name; a new `recipes/*.json` is added here.
pub const FILES: &[(&str, &str)] = &[
    ("rooms_and_door", include_str!("../../recipes/rooms_and_door.json")),
    ("two_floor_house", include_str!("../../recipes/two_floor_house.json")),
    ("convenience_store", include_str!("../../recipes/convenience_store.json")),
    ("classroom_wing", include_str!("../../recipes/classroom_wing.json")),
    ("coin_run", include_str!("../../recipes/coin_run.json")),
    ("gated_garden", include_str!("../../recipes/gated_garden.json")),
];

/// Embedded 2D mechanic patterns by name (a complete `*.game2d.json` each); a new `examples/patterns/*.game2d.json` is added here (a test fails until it is).
pub const PATTERNS: &[(&str, &str)] = &[
    ("key-door", include_str!("../../examples/patterns/key-door.game2d.json")),
    ("timer-lose", include_str!("../../examples/patterns/timer-lose.game2d.json")),
    ("collect-then-exit", include_str!("../../examples/patterns/collect-then-exit.game2d.json")),
    ("health-damage", include_str!("../../examples/patterns/health-damage.game2d.json")),
    ("checkpoint-respawn", include_str!("../../examples/patterns/checkpoint-respawn.game2d.json")),
    ("spawner-waves", include_str!("../../examples/patterns/spawner-waves.game2d.json")),
    ("survive-then-escape", include_str!("../../examples/patterns/survive-then-escape.game2d.json")),
    ("shared-effect", include_str!("../../examples/patterns/shared-effect.game2d.json")),
];

/// One 2D mechanic pattern, read from its own game file (the game's `title`, `description`, rule ids and scenario names are the documentation: nothing to keep in step).
pub struct Pattern {
    pub name: &'static str,
    pub text: &'static str,
    pub json: Value,
}

impl Pattern {
    /// The one-line title (`Pattern: key and door` without the prefix).
    pub fn title(&self) -> String {
        let t = self.json["title"].as_str().unwrap_or(self.name);
        t.strip_prefix("Pattern: ").unwrap_or(t).to_string()
    }
    /// What it shows and the mechanic in a sentence.
    pub fn description(&self) -> &str {
        self.json["description"].as_str().unwrap_or("")
    }
    /// The ids of its rules, in order.
    pub fn rules(&self) -> Vec<&str> {
        self.json["rules"].as_array().map(|a| a.iter().filter_map(|r| r["id"].as_str()).collect()).unwrap_or_default()
    }
    /// The names of the scenarios that prove it.
    pub fn scenarios(&self) -> Vec<&str> {
        self.json["checks"]["scenarios"].as_array().map(|a| a.iter().filter_map(|r| r["name"].as_str()).collect()).unwrap_or_default()
    }
}

/// All 2D mechanic patterns.
pub fn patterns() -> Vec<Pattern> {
    PATTERNS.iter().map(|(name, text)| Pattern { name, text, json: serde_json::from_str(text).unwrap_or(Value::Null) }).collect()
}

/// A known-good example map, parsed, with its `recipe` metadata block.
pub struct Recipe {
    pub name: &'static str,
    pub text: &'static str,
    pub json: Value,
}

impl Recipe {
    fn meta(&self, key: &str) -> Option<&Value> {
        self.json.get("recipe").and_then(|r| r.get(key))
    }
    /// The recipe's one-line title.
    pub fn title(&self) -> &str {
        self.meta("title").and_then(Value::as_str).unwrap_or(self.name)
    }
    /// The recipe's summary.
    pub fn summary(&self) -> &str {
        self.meta("summary").and_then(Value::as_str).unwrap_or("")
    }
    fn list(&self, key: &str) -> Vec<&str> {
        self.meta(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
    }
}

/// All recipes.
pub fn all() -> Vec<Recipe> {
    FILES.iter().map(|(name, text)| Recipe { name, text, json: serde_json::from_str(text).unwrap_or(Value::Null) }).collect()
}

/// The `recipe` listing text.
pub fn render_list() -> String {
    let mut out = String::from("Recipes — complete known-good maps to copy from (`recipe <name>` explains one, `--new out.json` starts from it):\n");
    for r in all() {
        out.push_str(&format!("  {:<18} {}\n", r.name, r.title()));
    }
    out.push_str("\nEvery recipe passes `lint` and its own `verify` checks (enforced by `cargo test`).\n");
    out.push_str(
        "\n2D mechanics (a complete tiny browser game each, with the scenarios that prove it; `recipe <name>` explains, `--new g.game2d.json` copies):\n",
    );
    for p in patterns() {
        out.push_str(&format!("  {:<20} {}\n", p.name, p.title()));
    }
    out
}

/// The `recipe <name>` explanation text.
pub fn render_one(name: &str) -> Result<String, String> {
    if let Some(p) = patterns().into_iter().find(|p| p.name == name) {
        let mut out = format!("{} — {}\n{}\n\nRules:\n", p.name, p.title(), p.description());
        for r in p.rules() {
            out.push_str(&format!("  - {r}\n"));
        }
        out.push_str("\nScenarios that prove it (copy their shape for your own game):\n");
        for sc in p.scenarios() {
            out.push_str(&format!("  - {sc}\n"));
        }
        out.push_str(&format!(
            "\nStart from it:  red_engine2 recipe {name} --new mygame.game2d.json\nThen:           red_engine2 verify mygame.game2d.json  (change the look and numbers; keep the rules and the scenarios; add your own)\n"
        ));
        return Ok(out);
    }
    let all = all();
    let Some(r) = all.iter().find(|r| r.name == name) else {
        let hint = crate::prefabs::suggest(name, all.iter().map(|r| r.name).chain(PATTERNS.iter().map(|(n, _)| *n)));
        return Err(format!(
            "no recipe '{name}'{} — run `red_engine2 recipe`",
            if hint.is_empty() { String::new() } else { format!(" (did you mean {}?)", hint.join(", ")) }
        ));
    };
    let n_obj = r.json["objects"].as_array().map(Vec::len).unwrap_or(0);
    let zones: Vec<&str> = r.json["zones"].as_array().map(|z| z.iter().filter_map(|z| z["id"].as_str()).collect()).unwrap_or_default();
    let mut out = format!("{} — {}\n{}\n\n", r.name, r.title(), r.summary());
    out.push_str(&format!("{n_obj} objects; zones: {}\n\nTeaches:\n", zones.join(", ")));
    for t in r.list("teaches") {
        out.push_str(&format!("  - {t}\n"));
    }
    out.push_str("\nTips:\n");
    for t in r.list("tips") {
        out.push_str(&format!("  - {t}\n"));
    }
    out.push_str(&format!(
        "\nStart from it:  red_engine2 recipe {name} --new mymap.json\nThen:           red_engine2 ls mymap.json | lint | plan | tour | verify\n"
    ));
    Ok(out)
}

/// A recipe's raw JSON text, for `--new` / `--print`.
pub fn text_of(name: &str) -> Option<&'static str> {
    FILES.iter().chain(PATTERNS.iter()).find(|(n, _)| *n == name).map(|(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_has_metadata() {
        for r in all() {
            assert!(!r.json.is_null(), "recipe {} is not valid JSON", r.name);
            assert!(!r.summary().is_empty() && !r.list("teaches").is_empty(), "recipe {} needs recipe.summary and recipe.teaches", r.name);
        }
    }

    #[test]
    fn every_pattern_file_is_listed_and_every_listed_pattern_verifies_with_its_own_scenarios() {
        // Nothing may sit in examples/patterns without being findable.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/patterns");
        let mut on_disk: Vec<String> =
            std::fs::read_dir(&dir).unwrap().flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".game2d.json").map(str::to_string)).collect();
        on_disk.sort();
        let mut listed: Vec<String> = PATTERNS.iter().map(|(n, _)| n.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed, "examples/patterns/*.game2d.json and recipes::PATTERNS must name the same files");
        for p in patterns() {
            assert!(
                !p.json.is_null() && !p.description().is_empty() && !p.rules().is_empty() && !p.scenarios().is_empty(),
                "pattern {} needs a description, rules and scenarios",
                p.name
            );
            let tmp = std::env::temp_dir().join(format!("re2_pattern_{}_{}", std::process::id(), p.name));
            std::fs::create_dir_all(&tmp).unwrap();
            let path = tmp.join(format!("{}.game2d.json", p.name));
            std::fs::write(&path, p.text).unwrap();
            let r = crate::tools::game2d::verify(&path, None);
            let _ = std::fs::remove_dir_all(&tmp);
            assert!(r.ok, "pattern {} fails its own scenarios:\n{}", p.name, r.text);
        }
    }

    #[test]
    fn the_listing_and_an_explanation_name_the_mechanics_and_how_to_copy_one() {
        let list = render_list();
        assert!(list.contains("2D mechanics") && list.contains("key-door") && list.contains("survive-then-escape"), "{list}");
        let one = render_one("key-door").unwrap();
        assert!(one.contains("Rules:") && one.contains("take the key") && one.contains("--new mygame.game2d.json") && one.contains("Scenarios"), "{one}");
        assert!(text_of("timer-lose").unwrap().contains("\"game2d\": 1"));
        assert!(render_one("key-dor").unwrap_err().contains("key-door"), "a typo suggests the pattern");
    }

    #[test]
    fn every_recipe_is_lint_clean_and_passes_its_checks() {
        for r in all() {
            let dir = std::env::temp_dir().join("re2_recipe_tests");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{}.json", r.name));
            std::fs::write(&path, r.text).unwrap();
            // Views need a GPU + goldens: the CI-safe subset (everything but views) runs here.
            let report = crate::tools::verify::run(&path, &crate::tools::verify::Options { skip_views: true, ..Default::default() }).unwrap();
            assert!(report.failed() == 0, "recipe {} fails its own checks:\n{}", r.name, report.render());
            assert!(report.results.len() >= 3, "recipe {} should declare lint + walk + objects checks", r.name);
        }
    }
}
