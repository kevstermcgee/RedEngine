//! The native half of the browser parity check (`crates/web3d/measure.py`): run a scene through `LocalSession` with scripted keys and print the state the browser's `Web3d.state()` prints.
//!
//! Run: `cargo run --no-default-features --example web3d_parity -- SCENE.json "KeyW:120;KeyW+ShiftLeft:60;:30"`
//! Each step is `KEYS:TICKS`, the keys joined with `+` (`KeyboardEvent.code` names: KeyW KeyA KeyS KeyD ShiftLeft Space KeyC), held for that many simulation ticks; no keys = standing still.
//! The browser runs the same steps with `key(code, down)` and `run(ticks)`; the two states must be the same match.
use red_engine2::app::session::LocalSession;
use red_engine2::sim::player::PlayerInput;
use serde_json::json;

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let scene = args.next().ok_or("usage: web3d_parity SCENE.json \"KeyW:120;:30\"")?;
    let script = args.next().unwrap_or_default();
    let text = std::fs::read_to_string(&scene).map_err(|e| format!("{scene}: {e}"))?;
    let mut session = LocalSession::from_json(&text).map_err(|e| e.join("\n"))?;
    let yaw = session.player().yaw;
    for step in script.split(';').filter(|s| !s.trim().is_empty()) {
        let (keys, ticks) = step.split_once(':').ok_or_else(|| format!("step `{step}`: write KEYS:TICKS"))?;
        let ticks: u32 = ticks.trim().parse().map_err(|_| format!("step `{step}`: TICKS is a whole number"))?;
        let held = |code: &str| keys.split('+').any(|k| k.trim() == code);
        let input = PlayerInput {
            forward: i8::from(held("KeyW") || held("ArrowUp")) - i8::from(held("KeyS") || held("ArrowDown")),
            strafe: i8::from(held("KeyD") || held("ArrowRight")) - i8::from(held("KeyA") || held("ArrowLeft")),
            jump: held("Space"),
            sprint: held("ShiftLeft") || held("ShiftRight"),
            crouch: held("KeyC") || held("ControlLeft"),
            yaw,
            pitch: 0.0,
            ..Default::default()
        };
        for _ in 0..ticks {
            session.step(input);
        }
    }
    let p = session.player_feet();
    println!(
        "{}",
        json!({
            "tick": session.tick(),
            "checksum": format!("{:016x}", session.sim().checksum()),
            "pos": [p.x, p.y, p.z],
            "ended": session.outcome(),
            "collision_disabled": session.rules().collision_disabled().collect::<Vec<_>>(),
            "hidden": session.hidden().collect::<Vec<_>>(),
        })
    );
    Ok(())
}
