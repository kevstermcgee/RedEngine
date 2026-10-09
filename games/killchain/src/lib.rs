//! Killchain: the loadout team-deathmatch game (ADR 2026-09-30-killchain-loadout-shooter), a program on Red Engine 2's public API (`red_engine2::...`), not part of the engine crate.
//!
//! The game owns its whole front end: a minimal home menu (solo, host, join, stats), match setup, the team lobby, the match itself (first person, 90 degrees, iron sights and
//! scopes, crouching, keyboard/mouse or a gamepad laid out like CS:GO's), the killcam, the end-of-match menu and the lifetime statistics. All of the rules run on the authoritative
//! server (the engine's `sim::kit`, `sim::ordnance`, `sim::objective`); this client only sends buttons and draws what the server says.
//!
//! - [`app`]: the window, the menus, hosting and joining, the stats file, and `KC_SCRIPT` runs (no window, pictures taken offscreen).
//! - [`game`]: one match from the client's side: input, prediction, camera, viewmodel, feedback, killcam, HUD.
//! - [`input`]: what the keyboard, the mouse and the gamepad mean.
//! - [`ui`] and [`objective_hud`]: the 2-D screens and what the HUD says about the objective modes, as pure functions of a window size and a small view struct.
//! - [`stats`]: the player's lifetime statistics, kept on their own machine.

pub mod app;
pub mod game;
pub mod input;
pub mod objective_hud;
pub mod stats;
pub mod ui;
