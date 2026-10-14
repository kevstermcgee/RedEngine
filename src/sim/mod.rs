//! The headless simulation core: the rules that must behave identically on the client and on a
//! future match server (ADR 0010, ADR 0014). **No window, GPU or audio types in this tree** — a
//! test (`sim::tests::sim_tree_has_no_renderer_imports`) enforces it.
//!
//! - [`clock`]: the fixed 60 Hz tick ([`clock::TickClock`]) and tick/second conversion.
//! - [`ai`]: bots: AI players inside the match, driven by ordinary [`player::PlayerInput`]s ([`ai::skill`], [`ai::nav`]).
//! - [`combat`]: weapon timing in whole ticks (melee swing, shot cooldown, weapon switch); [`combat_cfg`]: the scene's `combat` pacing block.
//! - [`change`] + [`components`]: generational change tracking (`changed_since(gen)`) on plain components
//!   ([`components::Transform`], [`components::Health`]).
//! - [`entities`] + [`statics`]: dynamic entities, and props as cheap static instances until promoted.
//! - [`snapshot`]: entities to bytes, full or delta (a placeholder layout, not a protocol).
//! - [`player`]: one player's movement for one tick as a pure function (single-player, server and client
//!   prediction all run it); [`spawns`]: multiplayer spawn points from the map.
//! - [`match_sim`]: the authoritative match world (players + props), ticked with queued inputs.
//! - [`rules`] + [`rules_expr`] + [`rules_run`]: game rules as data (`vars`, `rules`): triggers, conditions and actions
//!   parsed and validated with the scene, run deterministically every tick (`describe rules`).
//! - [`trace`] + [`replay`] + [`checksum`]: record a match (joins, inputs, events, checksums), replay it without a renderer or
//!   socket, and find the first tick where two runs disagree (`red_engine2 replay`).
//! - [`scratch`]: reusable per-tick buffers ([`scratch::ScratchVec`]) — reset, never freed.
//!
//! The rule for anything added here: a rendering rate must never be able to change what the
//! simulation does. Render code reads the sim (and may interpolate between ticks); it never
//! feeds it a variable `dt`.

// Nothing reachable from a UDP packet may panic the process. Test code is exempt; a genuine invariant is written
// as a `let ... else` / `?` with a message, not an `unwrap`.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo, clippy::unimplemented, clippy::unreachable))]

pub use red_core::sim::ai;
pub use red_core::sim::approach;
pub use red_core::sim::change;
pub use red_core::sim::checksum;
pub use red_core::sim::clock;
pub use red_core::sim::combat;
pub use red_core::sim::combat_cfg;
pub use red_core::sim::components;
pub use red_core::sim::defs;
pub use red_core::sim::entities;
/// A conventional agent interface to a match (`reset`/`observe`/`step`), over the real simulation; what an agent may know is separate from the privileged state.
pub mod env;
pub use red_core::sim::flow;
pub use red_core::sim::game;
pub use red_core::sim::interact;
pub use red_core::sim::interest;
pub use red_core::sim::items;
pub use red_core::sim::kart;
pub use red_core::sim::kit;
pub use red_core::sim::ledger;
pub use red_core::sim::match_sim;
pub use red_core::sim::objective;
pub use red_core::sim::objective_run;
pub use red_core::sim::ordnance;
pub use red_core::sim::player;
pub use red_core::sim::race;
pub mod replay;
pub use red_core::sim::rules;
pub use red_core::sim::rules_expr;
pub use red_core::sim::rules_run;
pub mod scenario;
pub use red_core::sim::scratch;
pub use red_core::sim::shooter;
pub use red_core::sim::shooter_bots;
pub use red_core::sim::shooter_snap;
pub use red_core::sim::snapshot;
pub use red_core::sim::spawns;
pub use red_core::sim::statics;
pub use red_core::sim::systems;
pub use red_core::sim::trace;

#[cfg(test)]
mod tests {
    /// The sim tree must stay callable on a headless VPS: no wgpu / winit / rodio.
    #[test]
    fn sim_tree_has_no_renderer_imports() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("sim");
        let banned = ["wgpu", "winit", "rodio"];
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|n| n != "mod.rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
                    for b in banned {
                        assert!(!line.contains(b), "{} mentions `{b}`: the sim tree must be renderer-free", path.display());
                    }
                }
            }
        }
    }
}
