//! The headless simulation core: the rules that must behave identically on the client and on a
//! future match server (ADR 0010, ADR 0014). **No window, GPU or audio types in this tree** — a
//! test (`sim::tests::sim_tree_has_no_renderer_imports`) enforces it.
//!
//! - [`clock`]: the fixed 60 Hz tick ([`clock::TickClock`]) and tick/second conversion.
//! - [`combat`]: weapon timing in whole ticks (melee swing, shot cooldown, weapon switch).
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

pub mod change;
pub mod checksum;
pub mod clock;
pub mod combat;
pub mod components;
pub mod entities;
pub mod interact;
pub mod interest;
pub mod match_sim;
pub mod player;
pub mod replay;
pub mod rules;
pub mod rules_expr;
pub mod rules_run;
pub mod scenario;
pub mod scratch;
pub mod snapshot;
pub mod spawns;
pub mod statics;
pub mod trace;

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
