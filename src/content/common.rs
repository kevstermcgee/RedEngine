//! Building blocks shared by the typed content formats: the `strict_struct!` macro (every field optional, unknown keys refused), animated tracks,
//! materials and the loose leaf types.

use serde::{Deserialize, Serialize};

/// A leaf whose JSON shape varies by design (a number or an expression string, a colour or a track of colours, an id or a list of ids...). The
/// checker owns its meaning and its errors; the typed definition only fixes the keys around it.
pub type Any = serde_json::Value;

/// Declares a typed content struct: `Serialize`, `Deserialize`, `JsonSchema` (with the `schema` feature), `#[serde(deny_unknown_fields)]`, and
/// every field `Option<T>` (default `None`, omitted when `None`). Which fields are required, and their ranges, are the checkers' business (they
/// already say so with a fix); the typed definition pins the keys and the shapes.
#[macro_export]
macro_rules! strict_struct {
    ($(#[$m:meta])* pub struct $name:ident { $($(#[$fm:meta])* $f:ident : $t:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $($(#[$fm])* #[serde(default, skip_serializing_if = "Option::is_none")] pub $f: Option<$t>,)*
        }
    };
}

/// A value that is either constant (`[0, 1, 0]`) or animated (`{"keyframes": [...]}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Track<T> {
    Keys(Keyframes<T>),
    Const(T),
}

/// `{"keyframes": [...]}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Keyframes<T> {
    pub keyframes: Vec<Keyframe<T>>,
}

/// One keyframe: time in seconds, the value, an easing name (`linear` by default).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Keyframe<T> {
    pub t: f64,
    pub value: T,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<String>,
}

/// A scalar leaf: a number, or a string (an expression `"=0.56+$gap"`, a param `"$h"`, a colour), or true/false. Never an object, so an
/// unknown key can never hide behind it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Scalar {
    Num(f64),
    Text(String),
    Flag(bool),
}

/// `[x, y, z]` (each a number, or an expression / `$param` inside a prefab).
pub type V3 = [Scalar; 3];
/// `[x, z]` or `[w, d]`.
pub type V2 = [Scalar; 2];
/// A position or a rotation: constant or keyframed.
pub type V3Track = Track<V3>;

strict_struct! {
    /// A surface: colour (`#rrggbb`, `@role`, or a track of colours), metal, roughness, glow, opacity, or a `style.materials` preset.
    pub struct Material {
        color: Track<Scalar>,
        metallic: Track<Scalar>,
        roughness: Track<Scalar>,
        emissive: Track<Scalar>,
        opacity: Track<Scalar>,
        preset: String,
    }
}

/// A scale: one number, `[x, y, z]`, or keyframes of either.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Scale {
    Keys(Keyframes<Any>),
    Uniform(Scalar),
    Axes(V3),
}

/// A colour string, or an object with more to say (a wall's `baseboard`, a text's `backing`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ColorOr<T> {
    Color(String),
    Detailed(T),
}
