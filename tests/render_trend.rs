//! The render trend keeps what is exact (triangles, draw calls) apart from what belongs to one adapter (milliseconds), and makes the same fixed scenes runnable on a real GPU.
//!
//! * `red_engine2 render-trend` writes a record that names its adapter and says whether it is a `software` rasteriser or a `gpu`.
//! * `benches/render_trend.py compare` only ever sets milliseconds against a run on the SAME adapter, and says so when the newest run has no earlier twin.
//!
//! The comparison logic is tested with synthetic records (this machine has no usable GPU to make a real one); the native command is tested on whatever adapter exists
//! here and skips itself when there is none.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("re2_render_trend_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/render_trend.py")
}

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

/// A record as `render-trend` writes it, for an adapter and kind.
fn record(adapter: &str, kind: &str, commit: &str, label: &str, tris: u64, ms: f64) -> Value {
    json!({
        "schema": "red-render/1", "label": label, "date": "2026-10-05T00:00:00+00:00", "engine_commit": commit, "dirty": false, "profile": "debug", "repeat": 3,
        "adapter": adapter, "adapter_kind": kind, "machine": {"os": "test", "arch": "x", "cores": 1},
        "scenes": [{"id": "marcel-morning", "players": 1, "view": [960, 540], "ms_best": ms, "ms_median": ms, "draws": 91, "tris": tris, "shadow_tris": 1, "resident": 9, "adapter": adapter}]
    })
}

fn py(history: &Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(python().expect("python")).arg(script()).args(args).env("RENDER_TREND_HISTORY", history).output().expect("run render_trend.py");
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
}

const SOFT: &str = "llvmpipe (LLVM 20.1.2, 256 bits) (Vulkan, Cpu)";
const GPU: &str = "NVIDIA GeForce RTX 3060 (Vulkan, DiscreteGpu)";

#[test]
fn compare_sets_milliseconds_only_against_the_same_adapter() {
    if python().is_none() {
        return;
    }
    let dir = scratch("compare");
    let history = dir.join("render.json");
    let runs = json!({"schema": "red-render/1", "runs": [
        record(SOFT, "software", "aaa1111", "before", 897067, 370.0),
        record(GPU, "gpu", "aaa1111", "before on a GPU", 897067, 4.0),
        record(SOFT, "software", "bbb2222", "after", 800000, 333.0),
        record(GPU, "gpu", "bbb2222", "after on a GPU", 800000, 3.5)]});
    std::fs::write(&history, runs.to_string()).unwrap();
    // The newest run is on the GPU: it is set against the earlier GPU run, never against the software ones.
    let (ok, text) = py(&history, &["compare"]);
    assert!(ok, "{text}");
    assert!(text.contains("aaa1111 (before on a GPU)  ->  bbb2222 (after on a GPU)") && text.contains("[gpu]"), "{text}");
    assert!(text.contains("ms 4 -> 4 ") && !text.contains("370") && !text.contains("333"), "no software milliseconds beside GPU ones: {text}");
    assert!(text.contains("tris 897,067 -> 800,000"), "{text}");
    // Asking for the software adapter pairs the two software runs.
    let (ok, text) = py(&history, &["compare", "--adapter", "llvmpipe"]);
    assert!(ok && text.contains("before  ->") || text.contains("(before)  ->"), "{text}");
    assert!(text.contains("ms 370 -> 333") && text.contains("software"), "{text}");
    // An adapter nobody ran on is an error that lists the adapters there are.
    let (ok, text) = py(&history, &["compare", "--adapter", "radeon"]);
    assert!(!ok && text.contains("llvmpipe") && text.contains("RTX 3060"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_run_on_a_new_adapter_has_nothing_to_compare_with_and_says_why() {
    if python().is_none() {
        return;
    }
    let dir = scratch("alone");
    let history = dir.join("render.json");
    std::fs::write(&history, json!({"schema": "red-render/1", "runs": [record(SOFT, "software", "aaa1111", "dev box", 1, 370.0)]}).to_string()).unwrap();
    // A record made on a player's GPU is filed with `add`; the dev box's software run stays separate.
    let file = dir.join("gpu.json");
    std::fs::write(&file, record(GPU, "gpu", "unknown", "RTX 3060, driver 560", 1, 4.0).to_string()).unwrap();
    let (ok, text) = py(&history, &["add", file.to_str().unwrap(), "--commit", "bbb2222"]);
    assert!(ok && text.contains("adding a run on") && text.contains("[gpu]"), "{text}");
    let (ok, text) = py(&history, &["compare"]);
    assert!(ok && text.contains("only run on") && text.contains("RTX 3060") && text.contains("not comparable"), "{text}");
    let (_, listed) = py(&history, &["list"]);
    assert!(listed.contains("software") && listed.contains("gpu") && listed.contains("bbb2222"), "{listed}");
    // A file that is not a record is refused, not filed.
    std::fs::write(&file, "{\"hello\": 1}").unwrap();
    let (ok, text) = py(&history, &["add", file.to_str().unwrap()]);
    assert!(!ok && text.contains("not a `red_engine2 render-trend` record"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(feature = "gfx")]
#[test]
fn render_trend_writes_a_record_that_names_its_adapter_and_kind() {
    let dir = scratch("native");
    std::fs::create_dir_all(dir.join("benches")).unwrap();
    let scene = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/endless_meadow.json");
    let list = dir.join("benches/render_scenes.json");
    std::fs::write(&list, json!({"scenes": [{"id": "meadow-small", "scene": scene, "players": 1, "size": "160x90", "hour": 9}]}).to_string()).unwrap();
    let out = dir.join("record.json");
    let o = Command::new(env!("CARGO_BIN_EXE_red_engine2"))
        .args(["render-trend", "--scenes", list.to_str().unwrap(), "--repeat", "2", "--label", "test run", "--out", out.to_str().unwrap()])
        .output()
        .expect("run render-trend");
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    if !o.status.success() && text.contains("GPU adapter") {
        return; // no adapter at all on this machine
    }
    assert!(o.status.success(), "{text}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(v["schema"], "red-render/1");
    assert_eq!(v["label"], "test run");
    assert!(["software", "gpu"].contains(&v["adapter_kind"].as_str().unwrap()), "{v}");
    assert!(v["adapter"].as_str().is_some_and(|a| !a.is_empty()));
    let s = &v["scenes"][0];
    assert!(s["id"] == "meadow-small" && s["tris"].as_u64().unwrap() > 1000 && s["ms_best"].as_f64().unwrap() > 0.0 && s["streamed"] == true, "{s}");
    // A software adapter says, in the output a human reads, that its milliseconds are not a player's.
    if v["adapter_kind"] == "software" {
        assert!(text.contains("software rasteriser"), "{text}");
    }
    // An empty scene list is an error that says so, not an empty record.
    std::fs::write(&list, "{\"scenes\": []}").unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_red_engine2")).args(["render-trend", "--scenes", list.to_str().unwrap()]).output().unwrap();
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("non-empty"), "{}", String::from_utf8_lossy(&o.stderr));
    let _ = std::fs::remove_dir_all(dir);
}
