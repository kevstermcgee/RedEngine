//! `assets/packs.json`: the registry of the core asset packs (`catalog --manifest`).

use serde::{Deserialize, Serialize};

/// The pack registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PackRegistry {
    /// Format version (1).
    pub format: u32,
    pub description: String,
    pub policy: PackPolicy,
    pub packs: Vec<Pack>,
}

/// How the library grows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PackPolicy {
    pub foundation: String,
    pub promotion: String,
    pub compatibility: String,
}

/// One pack: an id, the file that holds it, what it is for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub id: String,
    /// The pack's file, repository-relative (`assets/food.json`; `src/props.rs` for the engine-native props).
    pub file: String,
    pub description: String,
    pub tags: Vec<String>,
}
