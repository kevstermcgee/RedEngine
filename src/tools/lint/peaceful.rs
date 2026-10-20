//! The `peaceful-swing` check, kept apart so `lint.rs` stays under its length ceiling.

use super::{finding, Finding, Severity};
use crate::tools::world::MapWorld;

/// A `peaceful` player has no bat, so a rule waiting for `swing` never fires and nothing says so (Idea Forge 2026-10-18, crooked-ferry).
pub(super) fn check_swing(world: &MapWorld, out: &mut Vec<Finding>) {
    if !world.scene.player.mode.is_peaceful() {
        return;
    }
    for r in &world.scene.rules.rules {
        if matches!(&r.when, crate::sim::rules::When::Event(e) if e == "swing") {
            out.push(finding(
                Severity::Warn,
                "peaceful-swing",
                format!("rule `{}` waits for the `swing` event, but `player.mode` is `peaceful`: there is no bat to swing, so it never fires (drop `mode: peaceful`, or use the `interact` event)", r.id),
                None,
                &[],
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{format_report, lint, Finding};
    use crate::tools::reach::{compute, ReachParams};
    use crate::tools::world::MapWorld;
    use std::path::Path;

    fn run(json: &str) -> Vec<Finding> {
        let w = MapWorld::from_text(json, Path::new("t.json")).unwrap();
        let r = compute(&w, &ReachParams { cell: 0.1, ..Default::default() });
        lint(&w, &r)
    }

    #[test]
    fn a_swing_rule_in_a_peaceful_scene_is_a_warning() {
        let scene = |mode: &str| {
            format!(
                r##"{{"camera":{{"position":[0,1.7,0]}},"player":{{{mode}}},"objects":[{{"id":"b","type":"box","size":[1,1,1],"position":[0,0.5,3]}}],
            "vars":{{"n":0}},"rules":[{{"id":"hit_it","when":{{"event":"swing"}},"do":[{{"add":["n",1]}}]}}]}}"##
            )
        };
        let f = run(&scene(r#""mode":"peaceful""#));
        assert!(f.iter().any(|x| x.code == "peaceful-swing" && x.message.contains("`hit_it`")), "{}", format_report(&f));
        let f = run(&scene(r#""still":false"#));
        assert!(!f.iter().any(|x| x.code == "peaceful-swing"), "{}", format_report(&f));
    }
}
