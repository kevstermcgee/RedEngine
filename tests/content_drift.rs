//! The drift guard between the two definitions of every content format (ADR 2026-10-11-content-formats-are-typed-and-checked-twice): the hand-written
//! checker (the source of the error messages) and the typed `content::` definition (`deny_unknown_fields`, the source of the JSON Schema).
//!
//! 1. Every content file in the repository passes BOTH.
//! 2. An unknown key injected at every distinct object path of those files is refused by BOTH or accepted by BOTH (accepted = the place is a map of
//!    the author's own names, like `vars`). Any disagreement fails the build: one definition learned a key the other does not know.

use red_engine2::content;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PROBE: &str = "zzq_drift_probe";

/// Places where a new key is refused for a reason that is not a key set, so the typed definition (a map there) cannot know it. Each has its reason.
const NAME_CHECKED: &[(&str, &str)] = &[
    // A prefab instance's params must be the params that prefab declares: a cross-reference to another definition, checked by `prefabs::expand_instance`.
    ("[type=prefab].params", "a prefab instance's params are checked against that prefab's declared params"),
    // A 2D `apply`'s arguments must be the params of the effect it applies: checked against that effect by `red2d::effects`.
    (".do[].with", "an effect's arguments are checked against that effect's params"),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Which files of a folder are samples of a format.
type Keep<'a> = &'a dyn Fn(&Path) -> bool;
/// A format's checker: `Err` lists every problem, as the loader reports them.
type Checker<'a> = &'a dyn Fn(&Value, &Path) -> Result<(), Vec<String>>;

fn files(globs: &[(&str, Keep)]) -> Vec<PathBuf> {
    fn walk(dir: &Path, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, keep, out);
            } else if p.extension().is_some_and(|x| x == "json") && keep(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for (dir, keep) in globs {
        walk(&root().join(dir), *keep, &mut out);
    }
    out
}

fn read(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn rel(p: &Path) -> String {
    p.strip_prefix(root()).unwrap_or(p).display().to_string().replace('\\', "/")
}

/// Every object path in `v` (skipping the extension namespace), with a signature that merges list positions (`objects[].material`) and, for a
/// list of `type`-tagged objects, one entry per type.
fn object_paths(v: &Value) -> Vec<(Vec<Step>, String)> {
    fn go(v: &Value, path: &mut Vec<Step>, sig: String, out: &mut Vec<(Vec<Step>, String)>) {
        match v {
            Value::Object(o) => {
                out.push((path.clone(), sig.clone()));
                for (k, x) in o {
                    if red_engine2::fields::is_extension_key(k) {
                        continue;
                    }
                    path.push(Step::Key(k.clone()));
                    go(x, path, format!("{sig}.{k}"), out);
                    path.pop();
                }
            }
            Value::Array(a) => {
                let mut seen = BTreeSet::new();
                for (i, x) in a.iter().enumerate() {
                    let ty = x.get("type").and_then(Value::as_str).map(str::to_string).unwrap_or_default();
                    if !seen.insert(ty.clone()) {
                        continue;
                    }
                    path.push(Step::Index(i));
                    go(x, path, if ty.is_empty() { format!("{sig}[]") } else { format!("{sig}[type={ty}]") }, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    go(v, &mut Vec::new(), String::new(), &mut out);
    out
}

#[derive(Clone)]
enum Step {
    Key(String),
    Index(usize),
}

fn with_probe(v: &Value, path: &[Step]) -> Value {
    let mut out = v.clone();
    let mut node = &mut out;
    for s in path {
        node = match s {
            Step::Key(k) => &mut node[k.as_str()],
            Step::Index(i) => &mut node[*i],
        };
    }
    if let Some(o) = node.as_object_mut() {
        o.insert(PROBE.to_string(), Value::from(1));
    }
    out
}

/// One sample of a format: where it came from (a repository file, or what a tool writes), its JSON, the folder its relative files resolve in.
struct Item {
    label: String,
    value: Value,
    base: PathBuf,
}

fn items(paths: Vec<PathBuf>) -> Vec<Item> {
    paths.into_iter().map(|p| Item { label: rel(&p), value: read(&p), base: p }).collect()
}

/// One content format: its samples, its checker and its typed definition.
struct Format<'a> {
    name: &'a str,
    files: Vec<Item>,
    checker: Checker<'a>,
    typed: &'a dyn Fn(&Value) -> Result<(), String>,
    /// False only for a format whose checker is lenient on purpose (written by the engine; see the ADR): its files must still pass both.
    probe_keys: bool,
}

fn check(f: &Format) -> Vec<String> {
    let mut problems = Vec::new();
    assert!(!f.files.is_empty(), "{}: no files found", f.name);
    let mut probed = BTreeSet::new();
    for item in &f.files {
        let (v, p) = (&item.value, item.base.as_path());
        if let Err(e) = (f.checker)(v, p) {
            problems.push(format!("{} {}: the checker refuses it: {}", f.name, item.label, e.join("; ")));
        }
        if let Err(e) = (f.typed)(v) {
            problems.push(format!("{} {}: the typed definition refuses it: {e}", f.name, item.label));
        }
        if !f.probe_keys {
            continue;
        }
        for (path, sig) in object_paths(v) {
            if !probed.insert(sig.clone()) || NAME_CHECKED.iter().any(|(suffix, _)| sig.ends_with(suffix)) {
                continue;
            }
            let m = with_probe(v, &path);
            let (c, t) = ((f.checker)(&m, p).is_ok(), (f.typed)(&m).is_ok());
            if c != t {
                problems.push(format!(
                    "{} {}: an unknown key at `{}` is {} by the checker but {} by the typed definition",
                    f.name,
                    item.label,
                    if sig.is_empty() { "<root>" } else { &sig },
                    if c { "accepted" } else { "refused" },
                    if t { "accepted" } else { "refused" }
                ));
            }
        }
    }
    problems
}

fn is_scene(p: &Path) -> bool {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".game2d.json") || name.ends_with(".trace.json") || name == "audio_golden.json" || name == "vocabulary.json" {
        return false;
    }
    let v = read(p);
    v.get("objects").is_some() && v.get("game2d").is_none()
}

fn scene_checker(v: &Value, p: &Path) -> Result<(), Vec<String>> {
    red_engine2::validate_scene_text(&v.to_string(), p.parent(), &rel(p))
}

#[test]
fn scenes_agree() {
    let f = Format {
        name: "scene",
        files: items(files(&[("examples", &is_scene), ("recipes", &is_scene), ("tests/fixtures", &is_scene)]))
            .into_iter()
            .chain(scaffold_items("map"))
            .collect(),
        checker: &scene_checker,
        typed: &|v| content::typed::<content::scene::Scene>(v).map(|_| ()),
        probe_keys: true,
    };
    let problems = check(&f);
    assert!(problems.is_empty(), "{} disagreement(s):\n  {}", problems.len(), problems.join("\n  "));
}

/// A library's checker: its definitions load (`add_json`) and each one expands into a valid scene (where its objects are checked).
fn library_checker(v: &Value, _p: &Path) -> Result<(), Vec<String>> {
    let mut lib = red_engine2::prefabs::builtin().0.clone();
    let errs = lib.add_json(v, "probe");
    if !errs.is_empty() {
        return Err(errs);
    }
    let mut out = Vec::new();
    for name in v.as_array().into_iter().flatten().filter_map(|d| d.get("name").and_then(Value::as_str)) {
        let scene = red_engine2::prefabs::preview_scene_in(&lib, name, None);
        if let Err(e) = red_engine2::scene_blocks::parse_scene(&scene.to_string()) {
            out.extend(e.into_iter().map(|e| format!("{name}: {e}")));
        }
    }
    if out.is_empty() {
        Ok(())
    } else {
        Err(out)
    }
}

#[test]
fn prefab_libraries_agree() {
    let lib = |p: &Path| p.parent().is_some_and(|d| d.ends_with("assets")) && read(p).is_array();
    let f = Format {
        name: "prefab library",
        files: items(files(&[("assets", &lib)])),
        checker: &library_checker,
        typed: &|v| content::typed::<content::prefab::PrefabLibrary>(v).map(|_| ()),
        probe_keys: true,
    };
    let problems = check(&f);
    assert!(problems.is_empty(), "{} disagreement(s):\n  {}", problems.len(), problems.join("\n  "));
}

/// What `new-game` writes, for each kind: real `game.json`, blueprints, maps and 2D games that exist in no repository folder.
fn scaffold_items(want: &str) -> Vec<Item> {
    use red_engine2::tools::{game::EngineRef, newgame};
    let mut out = Vec::new();
    for (k, kind) in [("walk", newgame::Kind::Walk), ("race", newgame::Kind::Race), ("2d", newgame::Kind::TwoD)] {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("content_drift_{}_{n}_{want}_{k}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let written = newgame::scaffold_kind(&dir, "drift", &EngineRef { path: Some("../red-engine-2".into()), ..Default::default() }, kind)
            .unwrap_or_else(|e| panic!("new-game --kind {k}: {e}"));
        for p in written.into_iter().filter(|p| p.extension().is_some_and(|x| x == "json")) {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            let kind_of = if name == "game.json" {
                "project"
            } else if name.ends_with(".blueprint.json") {
                "blueprint"
            } else if name.ends_with(".game2d.json") {
                "game2d"
            } else if read(&p).get("objects").is_some() {
                "map"
            } else {
                "other"
            };
            if kind_of == want {
                out.push(Item { label: format!("new-game --kind {k}: {name}"), value: read(&p), base: p.clone() });
            }
        }
    }
    out
}

fn assert_agree(f: Format) {
    let problems = check(&f);
    assert!(problems.is_empty(), "{} disagreement(s):\n  {}", problems.len(), problems.join("\n  "));
}

#[test]
fn games_2d_agree() {
    let g2d = |p: &Path| p.to_string_lossy().ends_with(".game2d.json");
    assert_agree(Format {
        name: "2D game",
        files: items(files(&[("examples", &g2d)])).into_iter().chain(scaffold_items("game2d")).collect(),
        checker: &|v, _| red2d::game::parse(&v.to_string()).map(|_| ()),
        typed: &|v| content::typed::<content::game2d::Game2d>(v).map(|_| ()),
        probe_keys: true,
    });
}

fn score_files() -> Vec<PathBuf> {
    let score = |p: &Path| {
        let v = read(p);
        v.get("tracks").is_some() && v.get("instruments").is_some()
    };
    files(&[("assets", &score), ("examples", &score)])
}

#[test]
fn scores_agree() {
    assert_agree(Format {
        name: "score",
        files: items(score_files()),
        checker: &|v, _| red_engine2::score::parse_score(v).map(|_| ()),
        typed: &|v| content::typed::<content::score::Score>(v).map(|_| ()),
        probe_keys: true,
    });
}

/// Every sound of every 2D game, held to the voice checker (a score's instruments are voices with the score's own limits, checked with the score).
#[test]
fn voices_agree() {
    let g2d = |p: &Path| p.to_string_lossy().ends_with(".game2d.json");
    let mut voices = Vec::new();
    for p in files(&[("examples", &g2d)]) {
        for (name, v) in read(&p)["sounds"].as_object().into_iter().flatten() {
            voices.push(Item { label: format!("{} sounds.{name}", rel(&p)), value: v.clone(), base: p.clone() });
        }
    }
    assert!(voices.len() > 20, "found only {} voices", voices.len());
    assert_agree(Format {
        name: "voice",
        files: voices,
        checker: &|v, _| red_engine2::voice_spec::parse_voice(v).map(|_| ()),
        typed: &|v| content::typed::<content::voice::Voice>(v).map(|_| ()),
        probe_keys: true,
    });
}

#[test]
fn blueprints_agree() {
    let example: Value = serde_json::from_str(&red_engine2::tools::blueprint::example()).unwrap();
    let mut samples = vec![Item { label: "build --example".into(), value: example, base: root() }];
    samples.extend(scaffold_items("blueprint"));
    samples.extend(items(files(&[("tests/fixtures", &|p: &Path| p.to_string_lossy().ends_with(".blueprint.json"))])));
    assert_agree(Format {
        name: "blueprint",
        files: samples,
        checker: &|v, p| red_engine2::tools::blueprint::compile_in(v, p.parent()).map(|_| ()),
        typed: &|v| content::typed::<content::blueprint::Blueprint>(v).map(|_| ()),
        probe_keys: true,
    });
}

#[test]
fn playscripts_agree() {
    let play = |p: &Path| p.to_string_lossy().ends_with(".play.json");
    let doc: Value = serde_json::from_str(
        r#"{"steps":[{"wait_for":{"at":"/online/in_round","eq":true}},{"expect":{"at":"/remote/drawn","eq":7,"msg":"8 fighters means 7 drawn"}},{"turn":360,"over":6},{"shot":"spin"},{"expect":{"at":"/remote/undrawn","eq":0}}]}"#,
    )
    .unwrap();
    let mut samples = vec![Item { label: "describe playtest EXAMPLE".into(), value: doc, base: root() }];
    samples.extend(items(files(&[("tests/fixtures", &play)])));
    assert_agree(Format {
        name: "playscript",
        files: samples,
        checker: &|v, _| red_engine2::playscript::Script::parse(&v.to_string()).map(|_| ()),
        typed: &|v| content::typed::<content::playscript::Script>(v).map(|_| ()),
        probe_keys: true,
    });
}

#[test]
fn projects_agree() {
    assert_agree(Format {
        name: "game.json",
        files: scaffold_items("project"),
        checker: &|v, p| red_engine2::tools::game::parse(p.parent().unwrap_or(p), &v.to_string()).map(|_| ()),
        typed: &|v| content::typed::<content::project::Project>(v).map(|_| ()),
        probe_keys: true,
    });
}

#[test]
fn the_driver_roster_agrees() {
    assert_agree(Format {
        name: "drivers",
        files: items(vec![root().join("assets/data/drivers.json")]),
        checker: &|v, _| red_engine2::sim::kart::Roster::parse(&v.to_string()).map(|_| ()).map_err(|e| vec![e]),
        typed: &|v| content::typed::<content::drivers::DriverRoster>(v).map(|_| ()),
        probe_keys: true,
    });
}

#[test]
fn the_pack_registry_and_the_traces_load_typed() {
    // The pack registry's checker IS its typed definition (it had none before), so there is nothing to drift; it must load.
    content::typed::<content::packs::PackRegistry>(&read(&root().join("assets/packs.json"))).unwrap_or_else(|e| panic!("assets/packs.json: {e}"));
    // Traces are written by the engine and read leniently: both definitions accept every recorded trace in the repository (keys are not probed).
    assert_agree(Format {
        name: "trace",
        files: items(files(&[("tests/fixtures", &|p: &Path| p.to_string_lossy().ends_with(".trace.json"))])),
        checker: &|v, _| red_engine2::sim::trace::Trace::from_json(v).map(|_| ()).map_err(|e| vec![e]),
        typed: &|v| content::typed::<content::trace::Trace>(v).map(|_| ()),
        probe_keys: false,
    });
}

#[test]
fn the_guard_catches_a_key_one_definition_lacks() {
    // A typed definition that forgot a key the checker knows (here: refuses any scene with a `zones` block) must fail the guard, with the path.
    let f = Format {
        name: "scene",
        files: items(vec![root().join("recipes/coin_run.json")]),
        checker: &scene_checker,
        typed: &|v| match v.get("zones").and_then(Value::as_array).and_then(|z| z.first()).and_then(|z| z.get(PROBE)) {
            Some(_) => Ok(()), // "forgot" that a zone refuses unknown keys
            None => content::typed::<content::scene::Scene>(v).map(|_| ()),
        },
        probe_keys: true,
    };
    let problems = check(&f);
    assert!(problems.iter().any(|p| p.contains("`.zones[]`") && p.contains("refused by the checker but accepted by the typed definition")), "{problems:#?}");
}
