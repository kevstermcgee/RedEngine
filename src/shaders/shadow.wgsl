struct Globals {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    ambient: vec4<f32>,
    light_pos_or_dir: array<vec4<f32>, 8>,
    light_color_intensity: array<vec4<f32>, 8>,
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
@group(1) @binding(0) var<uniform> obj: ObjectUniform;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_shadow(in: VsIn) -> @builtin(position) vec4<f32> {
    let world = obj.model * vec4<f32>(in.pos, 1.0);
    return globals.light_view_proj * world;
}
