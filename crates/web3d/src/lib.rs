//! The browser build of the 3D player. All of it lives in `red_engine2::web3d`; this crate exists only to be a `cdylib` for `wasm-bindgen` without making the engine
//! crate one on every platform.
#![cfg(target_arch = "wasm32")]

pub use red_engine2::web3d::*;
