//! What the probe and the online tests share: a workshop of any size.

use serde_json::json;

/// A workshop of `n` machines in a temporary folder: its scene text and the folder.
pub fn workshop(n: usize) -> (String, std::path::PathBuf) {
    let dir =
        std::env::temp_dir().join(format!("workshop-scale-{}-{n}-{}", std::process::id(), std::thread::current().name().unwrap_or("main").replace(':', "_")));
    std::fs::create_dir_all(dir.join("defs")).unwrap();
    let n = n.max(2);
    let instances: Vec<_> = (0..n)
        .map(|i| {
            if i % 2 == 0 {
                json!({"id": format!("furnace_{i}"), "machine": "furnace", "from": "storage", "to": "storage"})
            } else {
                json!({"id": format!("press_{i}"), "machine": "press", "from": "storage", "to": "dock"})
            }
        })
        .collect();
    let defs = json!({
        "namespace": "sc",
        "resources": ["ore", "ingot", "gear"],
        "recipes": [{"id": "smelt", "in": {"ore": 2}, "out": {"ingot": 1}, "seconds": 2}, {"id": "press", "in": {"ingot": 1}, "out": {"gear": 2}, "seconds": 1.5}],
        "machines": [{"id": "furnace", "recipes": ["smelt"], "speed": 1, "input": 6, "output": 4}, {"id": "press", "recipes": ["press"], "speed": 1.5, "input": 4, "output": 6}],
        "stores": [{"id": "storage", "capacity": 40 * n, "start": {"ore": 4 * n}}, {"id": "dock", "capacity": 20 * n}],
        "sources": [{"id": "mine", "into": "storage", "seconds": 0.25, "gives": {"ore": [n / 2, n]}}],
        "sinks": [{"id": "ship", "from": "dock", "takes": {"gear": (n / 10).max(1)}, "seconds": 0.25}],
        "instances": instances,
    });
    std::fs::write(dir.join("defs/scale.json"), defs.to_string()).unwrap();
    let scene = json!({
        "camera": {"position": [0, 10, 10], "target": [0, 0, 0]},
        "vars": {"shipped": 0},
        "workshop": {"defs": ["defs/scale.json"], "seed": 1, "show": {"shipped": "sc:ship"}},
        "objects": [{"id": "floor", "type": "box", "size": [20, 0.2, 20], "position": [0, -0.1, 0]}],
    });
    (scene.to_string(), dir)
}
