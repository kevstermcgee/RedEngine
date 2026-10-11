//! Prefab definitions: a library file (`assets/*.json`, `catalog --library`) is an array of them; a scene's `prefabs` block is an array or an
//! object keyed by name. Checker: `prefabs::Library::add_json` (definition, param spec and `meta` keys) and `prefabs::expand_instance` (the
//! objects, at expansion).

use super::common::Any;
use super::objects::Object;
use crate::strict_struct;
use std::collections::BTreeMap;

strict_struct! {
    /// One prefab definition.
    pub struct PrefabDef {
        name: String,
        extends: String,
        meta: PrefabMeta,
        tags: Vec<String>,
        /// name -> a default value, or a spec `{default, desc, palette}`.
        params: BTreeMap<String, ParamSpec>,
        objects: Vec<Object>,
        desc: String,
        mount: String,
        collide: bool,
    }
}

strict_struct! {
    /// Discovery and lifecycle metadata.
    pub struct PrefabMeta {
        aliases: Vec<String>,
        roles: Vec<String>,
        styles: Vec<String>,
        status: String,
        revision: u64,
        license: String,
        origin: String,
        provenance: String,
    }
}

strict_struct! {
    /// A param with a description and an optional palette role.
    pub struct ParamSpecFull { default: Any, desc: String, palette: String }
}

/// A param: a spec (an object with `default`) or the default value itself. Deserialized by hand, as the checker reads it: an object with a
/// `default` key must be a strict spec (an untagged enum would fall back to "any value" and accept a misspelt key in a spec).
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ParamSpec {
    Spec(ParamSpecFull),
    Value(Any),
}

impl<'de> serde::Deserialize<'de> for ParamSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Any::deserialize(d)?;
        if v.as_object().is_some_and(|o| o.contains_key("default")) {
            serde_json::from_value(v).map(ParamSpec::Spec).map_err(serde::de::Error::custom)
        } else {
            Ok(ParamSpec::Value(v))
        }
    }
}

/// A scene's own prefabs: a list of definitions or an object `{name: definition}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum PrefabDefs {
    List(Vec<PrefabDef>),
    Named(BTreeMap<String, PrefabDef>),
}

/// A library file (`assets/*.json`).
pub type PrefabLibrary = Vec<PrefabDef>;
