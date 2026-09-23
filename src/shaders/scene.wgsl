struct Globals {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    ambient: vec4<f32>,
    light_pos_or_dir: array<vec4<f32>, 4>,
    light_color_intensity: array<vec4<f32>, 4>,
    counts: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

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
struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let world = obj.model * vec4<f32>(in.pos, 1.0);
    var out: VsOut;
    out.clip_pos = globals.view_proj * world;
    out.world_pos = world.xyz;
    out.world_normal = normalize((obj.normal_mat * vec4<f32>(in.normal, 0.0)).xyz);
    return out;
}

fn shadow_factor(world_pos: vec3<f32>, n_dot_l: f32) -> f32 {
    let lp = globals.light_view_proj * vec4<f32>(world_pos, 1.0);
    if (lp.w <= 0.0) {
        return 1.0;
    }
    let ndc = lp.xyz / lp.w;
    if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
        return 1.0;
    }
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let bias = clamp(0.0035 * (1.0 - n_dot_l) + 0.0006, 0.0006, 0.004);
    let texel = 1.0 / 2048.0;
    var shadow = 0.0;
    for (var dx = -1; dx <= 1; dx = dx + 1) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            let offset = vec2<f32>(f32(dx), f32(dy)) * texel;
            shadow = shadow + textureSampleCompare(shadow_map, shadow_sampler, uv + offset, ndc.z - bias);
        }
    }
    return shadow / 9.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let v = normalize(globals.camera_pos.xyz - in.world_pos);
    var color = globals.ambient.rgb * obj.base_color.rgb;

    let metallic = obj.material.x;
    let roughness = max(obj.material.y, 0.04);
    let shininess = mix(8.0, 160.0, 1.0 - roughness);
    let diffuse_color = obj.base_color.rgb * (1.0 - metallic);

    let n_lights = i32(globals.counts.x);
    let shadow_idx = i32(globals.counts.y);

    for (var i = 0; i < 4; i = i + 1) {
        if (i >= n_lights) {
            break;
        }
        let pod = globals.light_pos_or_dir[i];
        let ci = globals.light_color_intensity[i];
        var l: vec3<f32>;
        var atten = 1.0;
        if (pod.w < 0.5) {
            l = normalize(-pod.xyz);
        } else {
            let to_light = pod.xyz - in.world_pos;
            let dist = length(to_light);
            l = to_light / max(dist, 1e-4);
            let range = max(ci.w, 0.01);
            let falloff = clamp(1.0 - dist / range, 0.0, 1.0);
            atten = falloff * falloff;
        }
        let n_dot_l = max(dot(n, l), 0.0);
        if (n_dot_l <= 0.0) {
            continue;
        }
        var shadow = 1.0;
        if (i == shadow_idx) {
            shadow = shadow_factor(in.world_pos, n_dot_l);
        }
        let h = normalize(l + v);
        let spec_pow = pow(max(dot(n, h), 0.0), shininess);
        let spec = spec_pow * mix(0.15, 1.0, metallic);
        color = color + (diffuse_color * n_dot_l + vec3<f32>(spec, spec, spec)) * ci.rgb * atten * shadow;
    }

    color = color + obj.emissive.rgb;
    // Exposure + Reinhard tone-mapping: gracefully rolls off strong/overlapping lights toward
    // white instead of hard-clipping, so an author's light intensities don't need to be
    // perfectly balanced to avoid flat-white blowout. The exposure factor keeps typical
    // mid-tones from reading as too dark once Reinhard compresses the range.
    color = color * 1.3;
    color = color / (color + vec3<f32>(1.0));
    return vec4<f32>(color, 1.0);
}
