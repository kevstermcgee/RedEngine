@group(1) @binding(0) var<uniform> obj: ObjectUniform;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(3) sway: f32,
};

@vertex
// The cascade being drawn is the instance index: each cascade's draws are issued with first_instance = its number.
fn vs_shadow(in: VsIn, @builtin(instance_index) cascade: u32) -> @builtin(position) vec4<f32> {
    var world = obj.model * vec4<f32>(in.pos, 1.0);
    world = vec4<f32>(world.xyz + wind_offset(in.sway, world.xyz), 1.0);
    return globals.cascade_vp[cascade] * world;
}
