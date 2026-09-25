//! `red_engine2 net-test` as a regression gate: the game must stay playable on a bad connection. A real server and real clients run behind
//! the seeded bursty-lossy proxy (`net::netsim`) and are judged on what a player notices (`tools::nettest`). This is where the
//! remote-player freeze-then-snap after a burst of lost snapshots was found (see `net::interp::MAX_EXTRAPOLATE`).

use red_engine2::net::netsim;
use red_engine2::tools::nettest::{self, Options};
use std::path::PathBuf;

fn lab() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/test_lab.json")
}

fn run(profiles: &[&str], players: usize, secs: f64) -> Vec<nettest::ProfileReport> {
    let profiles = profiles.iter().map(|p| netsim::profile(p).expect("a built-in profile")).collect();
    nettest::run(&lab(), &Options { profiles, players, secs, seed: 3 }).expect("net-test runs")
}

fn assert_all_pass(reports: &[nettest::ProfileReport]) {
    let failed: Vec<String> = reports.iter().filter(|r| !r.ok()).map(|r| nettest::render(std::slice::from_ref(r))).collect();
    assert!(failed.is_empty(), "the game must survive these links:\n{}", failed.join("\n"));
}

#[test]
fn a_clean_link_and_a_bad_link_both_stay_playable_with_two_players() {
    assert_all_pass(&run(&["lan", "bad"], 2, 4.0));
}

#[test]
fn a_cruel_link_still_keeps_everyone_connected_and_nobody_teleports() {
    assert_all_pass(&run(&["awful"], 2, 5.0));
}

#[test]
fn a_full_match_of_eight_holds_up_on_a_mobile_link() {
    let reports = run(&["4g"], 8, 4.0);
    assert_all_pass(&reports);
    assert_eq!(reports[0].clients.len(), 8);
}

#[test]
fn the_report_is_machine_readable_and_says_what_each_check_measured() {
    let reports = run(&["wifi"], 2, 3.0);
    let j = nettest::to_json(&reports);
    assert_eq!(j["ok"], true);
    let p = &j["profiles"][0];
    assert_eq!(p["profile"], "wifi");
    assert!(p["checks"].as_array().is_some_and(|c| c.len() >= 5));
    assert!(p["clients"][0]["rtt_ms"].as_f64().is_some_and(|r| r > 0.0));
}
