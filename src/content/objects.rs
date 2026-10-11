//! Scene objects (`objects`, a group's `children`, a prefab's `objects`): one struct per `type`, each with the fields every object shares
//! (`strict::object_keys`).

use super::common::{Any, ColorOr, Material, Scalar, Scale, Track, V3Track, V3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Declares one object type: the common fields (`id position rotation scale collide movable lint_ignore on_terrain`) plus its own.
macro_rules! object_struct {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $f:ident : $t:ty),* $(,)? }) => {
        $crate::strict_struct! {
            $(#[$m])*
            pub struct $name {
                id: String,
                position: V3Track,
                rotation: V3Track,
                scale: Scale,
                collide: bool,
                movable: bool,
                lint_ignore: Vec<String>,
                on_terrain: bool,
                $($(#[$fm])* $f: $t,)*
            }
        }
    };
}

object_struct!(BoxObj { size: Any, material: Material });
object_struct!(SphereObj { radius: Any, material: Material });
object_struct!(
    /// A cylinder, cone or capsule.
    RoundObj { radius: Any, height: Any, material: Material }
);
object_struct!(PlaneObj { size: Any, material: Material });
object_struct!(
    /// A group; `prefab_name` and `mount` are written by prefab expansion.
    GroupObj { children: Vec<Object>, prefab_name: String, mount: String }
);
object_struct!(HumanoidObj { height: Any, build: Any, material: Material, pose: HumanoidPose, skin: Any, hair: Any, pants: Any, shoes: Any, style: Any });
object_struct!(RatObj { material: Material, pose: RatPose });
object_struct!(PropObj { prop: String, material: Material });
object_struct!(StairsObj { width: Any, run: Any, rise: Any, steps: Any, material: Material });
object_struct!(TerrainObj {
    size: Any,
    cell: Any,
    resolution: Any,
    heights: Any,
    heightmap: String,
    elevation_scale: Any,
    generator: TerrainGenerator,
    palette: Any,
    grain: Any,
    material: Material
});
object_struct!(WallObj {
    from: Any,
    to: Any,
    y: Any,
    height: Any,
    thickness: Any,
    material: Material,
    openings: Vec<Opening>,
    trim: Any,
    baseboard: ColorOr<Baseboard>,
    extend: Any
});
object_struct!(FenceObj {
    points: Any,
    closed: bool,
    y: Any,
    height: Any,
    post_spacing: Any,
    style: String,
    material: Material,
    post_color: Any,
    gaps: Vec<FenceGap>
});
object_struct!(ArrayObj { template: Box<Object>, count: Any, step: Any, positions: Any, rotation_step: Any });
object_struct!(TextObj {
    text: String,
    height: Any,
    depth: Any,
    align: String,
    spacing: Any,
    line_gap: Any,
    backing: ColorOr<TextBacking>,
    material: Material
});
object_struct!(VoxelsObj { rows: Vec<String>, palette: BTreeMap<String, String>, cell: Any, depth: Any, material: Material });
object_struct!(
    /// A prefab instance (`params` are the prefab's own: a map).
    PrefabObj { prefab: String, params: BTreeMap<String, Any>, material: Material }
);

/// Any scene object, by its `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "lowercase")]
#[allow(clippy::large_enum_variant)] // a data definition read by the drift guard and the schema, not a hot path
pub enum Object {
    Box(BoxObj),
    Sphere(SphereObj),
    Cylinder(RoundObj),
    Cone(RoundObj),
    Capsule(RoundObj),
    Plane(PlaneObj),
    Group(GroupObj),
    Humanoid(HumanoidObj),
    Rat(RatObj),
    Prop(PropObj),
    Stairs(StairsObj),
    Terrain(TerrainObj),
    Wall(WallObj),
    Fence(FenceObj),
    Array(ArrayObj),
    Text(TextObj),
    Voxels(VoxelsObj),
    Prefab(PrefabObj),
}

/// A joint's rotation: `[rx, ry, rz]` degrees, or one bend angle (elbows, knees).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum JointAngle {
    Axes(V3),
    Bend(Scalar),
}

/// A joint: constant or keyframed.
pub type Joint = Track<JointAngle>;

crate::strict_struct! {
    /// A humanoid's joint rotations.
    pub struct HumanoidPose {
        spine: Joint,
        head: Joint,
        l_shoulder: Joint,
        r_shoulder: Joint,
        l_elbow: Joint,
        r_elbow: Joint,
        l_hip: Joint,
        r_hip: Joint,
        l_knee: Joint,
        r_knee: Joint,
    }
}

crate::strict_struct! {
    /// A rat's gait.
    pub struct RatPose { gait: Any, stride: Any, sway: Any }
}

crate::strict_struct! {
    /// One door or window cut into a wall.
    pub struct Opening { kind: String, at: Any, width: Any, height: Any, sill: Any, glass: Any, trim: Any }
}

crate::strict_struct! {
    /// A gap in a fence.
    pub struct FenceGap { at: Any, width: Any }
}

crate::strict_struct! {
    /// A generated terrain: a seed, a height profile and layered noise.
    pub struct TerrainGenerator { seed: Any, profile: Any, noise: TerrainNoise }
}

crate::strict_struct! {
    /// Noise layered on a terrain profile.
    pub struct TerrainNoise { amplitude: Any, wavelength: Any, octaves: Any, seed: Any, fade_x: Any }
}

crate::strict_struct! {
    /// A wall's baseboard.
    pub struct Baseboard { color: String, height: Any }
}

crate::strict_struct! {
    /// The board behind a text.
    pub struct TextBacking { color: String, material: Material, margin: Any, thickness: Any }
}
