//! `docs/features.json` must stay true (ADR 0033): every file pattern, test suite, doc and dependency it names exists, and every source
//! file, test and bench belongs to a feature, so `red_engine2 impact` can always say what a change affects.

use red_engine2::tools::features;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_feature_index_is_true() {
    let all = features::load().expect("docs/features.json parses");
    let problems = features::check(&all, &root());
    assert!(problems.is_empty(), "docs/features.json is out of date (fix the index, or run `red_engine2 features --check`):\n  {}", problems.join("\n  "));
}

#[test]
fn a_changed_source_file_names_its_feature_dependents_and_tests() {
    let all = features::load().unwrap();
    let i = features::impact(&all, &["src/sim/flow.rs".to_string()]);
    assert!(i.direct.contains_key("match_flow"));
    for dependent in ["net_server", "ui_kit", "net_protocol"] {
        assert!(i.downstream.contains(dependent), "{dependent} is built on match_flow: {:?}", i.downstream);
    }
    assert!(i.lib_filters.contains("sim::flow") && i.suites.contains("net_flow") && i.suites.contains("net_auth"), "{i:?}");
    let cmds = i.cargo_commands().join("\n");
    assert!(cmds.contains("cargo test --lib -- ") && cmds.contains("--test net_flow"), "{cmds}");
    assert!(i.docs.contains("docs/adr/0029-match-flow-lobby-rounds-rematch.md"));
}

#[test]
fn editing_a_test_file_says_which_features_it_can_break() {
    let all = features::load().unwrap();
    let i = features::impact(&all, &["tests/net_flow.rs".to_string()]);
    assert!(
        i.direct.contains_key("match_flow") && i.direct.contains_key("net_server") && i.direct.contains_key("net_client"),
        "{:?}",
        i.direct.keys().collect::<Vec<_>>()
    );
    assert!(i.unowned.is_empty());
}

#[test]
fn nothing_in_the_repository_is_orphaned_from_the_index() {
    let all = features::load().unwrap();
    for f in
        ["src/net/upnp.rs", "src/tools/perf.rs", "src/tools/package.rs", "src/tools/nettest.rs", "src/ui/online.rs", "src/bin/re2/online.rs", "src/crypto.rs"]
    {
        assert!(!features::owners(&all, f).is_empty(), "{f} has no owner");
    }
}
