//! Killchain: the loadout team-deathmatch client (ADR 2026-09-30-killchain-loadout-shooter).
//!
//! `re2` starts it instead of the prototype client when the map has a `shooter` block. The game owns its whole front end: a minimal home menu
//! (solo, host, join, stats), match setup, the team lobby, the match itself (first person, 90 degrees, iron sights and scopes, crouching,
//! keyboard/mouse or a gamepad laid out like CS:GO's), the killcam, the end-of-match menu and the lifetime statistics. All of the rules run on
//! the authoritative server (`sim::kit`, `sim::ordnance`); this client only sends buttons and draws what the server says.
//!
//! - [`app`]: the window, the menus, hosting and joining, the stats file.
//! - [`game`]: one match from the client's side: input, prediction, camera, viewmodel, feedback, killcam, HUD.
//! - [`input`]: what the keyboard, the mouse and the gamepad mean.

mod app;
mod game;
mod input;

pub(crate) use app::{run, Options};
