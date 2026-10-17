//! Golden parity: the systems hash and the save of 3000 ticks of the shipped scenes (and a 100-machine workshop) are pinned, so a change to how
//! the systems run (interned ids, slots) is proven to change nothing a player, a save or the match checksum can see. A deliberate change of
//! behavior updates these numbers in the same commit, and says why.
//!
//! `WORKSHOP_GOLDEN_PRINT=1 cargo test -p workshop_server --test golden -- --nocapture` prints the current values.

mod common;

use red_engine2::sim::ledger::Fnv;
use red_engine2::sim::match_sim::MatchSim;
use red_engine2::sim::spawns::parse_spawns;
use std::path::{Path, PathBuf};
use workshop_server::game::WorkshopGame;

fn content() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../content")
}

/// The systems hash every 500 ticks up to 3000, then a hash of the save's text at 3000.
fn trace(text: &str, dir: &Path) -> Vec<u64> {
    workshop_server::register();
    let scene = red_engine2::scene_blocks::parse_scene_in(text, Some(dir)).unwrap();
    let mut sim = MatchSim::try_new(&scene, parse_spawns(text).unwrap()).unwrap();
    let mut out = Vec::new();
    for t in 1..=3000 {
        sim.tick_once();
        if t % 500 == 0 {
            out.push(sim.game::<WorkshopGame>().unwrap().systems().hash());
        }
    }
    let g = sim.game::<WorkshopGame>().unwrap();
    assert!(g.systems().halted().is_none());
    g.systems().ledger().audit().unwrap();
    let mut h = Fnv::new();
    h.str(&g.save().to_string());
    out.push(h.0);
    out
}

fn check(name: &str, got: Vec<u64>, want: &[u64]) {
    if std::env::var_os("WORKSHOP_GOLDEN_PRINT").is_some() {
        println!("{name}: {got:#x?}");
        return;
    }
    assert_eq!(got, want, "{name}: the systems hash or the save changed (WORKSHOP_GOLDEN_PRINT=1 prints the new values)");
}

#[test]
fn the_shipped_workshop_runs_to_the_same_hashes() {
    let text = std::fs::read_to_string(content().join("workshop.json")).unwrap();
    check("workshop.json", trace(&text, &content()), &WORKSHOP);
}

#[test]
fn the_expanded_workshop_runs_to_the_same_hashes() {
    let text = std::fs::read_to_string(content().join("workshop_plus.json")).unwrap();
    check("workshop_plus.json", trace(&text, &content()), &WORKSHOP_PLUS);
}

#[test]
fn a_hundred_machines_run_to_the_same_hashes() {
    let (text, dir) = common::workshop(100);
    let got = trace(&text, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    check("100 machines", got, &HUNDRED);
}

const WORKSHOP: [u64; 7] =
    [0x1593598c227e664d, 0xb05faaa6a00675d3, 0x98b118e0b3aca4a6, 0xf2f9c0043ce2320a, 0x66cea853b16812ea, 0xb59f67d6c85070a5, 0xfda491010586fcd1];
const WORKSHOP_PLUS: [u64; 7] =
    [0x959bc70c8218956b, 0x50e7f818dddbc240, 0x546b623c73aadce1, 0x7f15116ae153a0a5, 0x2a003857059629fd, 0x474c1c2b43868d41, 0xbd4e0aa66b3e3b07];
const HUNDRED: [u64; 7] =
    [0x1f8de4587cdc9580, 0xe95bf96a81cb9579, 0x807d2ba587698ce2, 0x3433a0a59ccb1718, 0xf7e96b78c36e721b, 0x0994665926e1a892, 0xad862a7521ed2f84];
