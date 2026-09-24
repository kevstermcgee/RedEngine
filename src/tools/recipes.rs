//! Recipes: complete, known-good example maps to imitate. Each file in `recipes/` is a real scene
//! (embedded in the binary) whose top-level `"recipe"` block says what it teaches. They are held
//! to the same bar as user maps: `tests::every_recipe_is_lint_clean_and_passes_its_checks`
//! validates, lints and runs the `checks` of every recipe, so a recipe can never rot.
//!
//! `red_engine2 recipe` lists them; `recipe <name>` explains one; `recipe <name> --new out.json`
//! copies it as a starting point; `recipe <name> --print` dumps the JSON.

use serde_json::Value;

/// Embedded recipe files by name; a new `recipes/*.json` is added here.
pub const FILES: &[(&str, &str)] = &[
    ("rooms_and_door", include_str!("../../recipes/rooms_and_door.json")),
    ("two_floor_house", include_str!("../../recipes/two_floor_house.json")),
    ("convenience_store", include_str!("../../recipes/convenience_store.json")),
    ("classroom_wing", include_str!("../../recipes/classroom_wing.json")),
];

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
    out
}

/// The `recipe <name>` explanation text.
pub fn render_one(name: &str) -> Result<String, String> {
    let all = all();
    let Some(r) = all.iter().find(|r| r.name == name) else {
        let hint = crate::prefabs::suggest(name, all.iter().map(|r| r.name));
        return Err(format!("no recipe '{name}'{} — run `red_engine2 recipe`", if hint.is_empty() { String::new() } else { format!(" (did you mean {}?)", hint.join(", ")) }));
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
    FILES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
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
