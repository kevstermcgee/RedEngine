// Declarations shared by scene.wgsl, shadow.wgsl and background.wgsl. `gpu.rs` concatenates this file in front of
// each of them at compile time (`concat!(include_str!(..))`), so `Globals` and `ObjectUniform` exist exactly once: a
// layout change made here reaches every shader, and `gpu::tests` checks the Rust mirrors (`GlobalUniform`,
// `ObjectUniform`) against these definitions without needing a GPU.
//
// MAX_LIGHTS must equal `schema::MAX_LIGHTS` (tested).
const MAX_LIGHTS: u32 = 16u;

struct Globals {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    ambient: vec4<f32>,
    light_pos_or_dir: array<vec4<f32>, 16>,
    light_color_intensity: array<vec4<f32>, 16>,
    counts: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct ObjectUniform {
    model: mat4x4<f32>,
    normal_mat: mat4x4<f32>,
    base_color: vec4<f32>,
    material: vec4<f32>,
    emissive: vec4<f32>,
};
