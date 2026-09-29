//! The reusable client layer: what any game built on Red needs to *present* a scene, without copying the first-person
//! `re2` application (ADR 0043, `red_engine2 describe custom-client`).
//!
//! | Need | Piece | Build |
//! |---|---|---|
//! | Load validated content, run gameplay | [`session::LocalSession`] (one player in the authoritative `MatchSim`: movement, props, rules) | headless |
//! | Choose a camera, map the pointer to the world | [`camera::ViewCamera`] (`top_down`, `look_at`, `screen_ray`, `pick_ground`); [`camera::FpsCamera`] is the first-person policy | headless |
//! | HUD and outcome | [`hud::HudState`] (rule variables, recent event, outcome) laid out by the audited `ui` kit | headless |
//! | Window, GPU, input | [`gpu::WindowGpu`], [`input::InputState`] | `gfx` |
//! | Draw the world | `viewer::LiveRenderer::world` + `render_view` (any camera; weapons and crosshair are `re2`'s optional layers) | `gfx` |
//! | A whole window loop | [`shell::run`] with a [`shell::ClientGame`] | `gfx` |
//! | Presentation checks without a window | [`offscreen::Offscreen`] (render + HUD to RGBA/PNG) | `gfx` |
//!
//! Gameplay stays in the shared simulation (`sim`): a client turns input into `PlayerInput`s and draws what the
//! session reports; it never re-implements movement or rules. `examples/external/topdown_switch` is a complete game
//! outside the engine crate that uses only this module.

pub mod camera;
pub mod hud;
pub mod session;

#[cfg(feature = "gfx")]
pub mod gpu;
#[cfg(feature = "gfx")]
pub mod input;
#[cfg(feature = "gfx")]
pub mod offscreen;
#[cfg(feature = "gfx")]
pub mod shell;

pub use camera::{FpsCamera, ViewCamera};
pub use hud::{HudState, RecentEvent};
pub use session::{input_toward, place_object, LocalSession};

#[cfg(feature = "gfx")]
pub use gpu::WindowGpu;
#[cfg(feature = "gfx")]
pub use input::{InputState, KeyCode, MouseButton};
#[cfg(feature = "gfx")]
pub use offscreen::Offscreen;
#[cfg(feature = "gfx")]
pub use shell::{run, ClientGame, Frame, HudPainter, WindowOptions};
