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

// ---- Lighting model constants (one place, so every map reads the same) ------------------------
// Maps author point lights hot (intensity ~10-40) because inverse-square falloff eats most of it;
// summed with a white ambient and a bright cream wall that blew every interior out to flat white.
// POINT_GAIN brings authored lamps into a range the tone-mapper can separate, AMBIENT_FLOOR is a
// small uniform fill so no corner/ceiling ever crushes to black, and LAMP_SOFT_RADIUS flattens the
// hot spot right under a lamp (light stops climbing inside that distance).
const POINT_GAIN: f32 = 0.36;
const AMBIENT_FLOOR: f32 = 0.10;
const LAMP_SOFT_RADIUS: f32 = 1.6;
const LAMP_WRAP: f32 = 0.35;
const EXPOSURE: f32 = 0.56;

// ACES filmic curve (Narkowicz fit): keeps mid-tones contrasty and rolls highlights off gently,
// instead of Reinhard's grey haze on anything bright.
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + vec3<f32>(0.03))) / (x * (2.43 * x + vec3<f32>(0.59)) + vec3<f32>(0.14)), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let v = normalize(globals.camera_pos.xyz - in.world_pos);
    // Hemisphere ambient: surfaces facing up catch more sky light than ones facing down (a
    // ceiling reads darker than a floor), which separates the planes of a room even where no
    // light reaches them directly.
    let hemi = mix(0.80, 1.10, n.y * 0.5 + 0.5);
    var color = (globals.ambient.rgb + vec3<f32>(AMBIENT_FLOOR)) * obj.base_color.rgb * hemi;

    let metallic = obj.material.x;
    let roughness = max(obj.material.y, 0.04);
    // A gentler curve than a straight mix(8, 160, 1-roughness): keeps mid-roughness surfaces
    // (most props) from jumping straight to a hard plastic-looking highlight the way a linear
    // roughness->shininess mapping tends to.
    let smoothness = 1.0 - roughness;
    let shininess = mix(8.0, 160.0, smoothness * smoothness);
    let diffuse_color = obj.base_color.rgb * (1.0 - metallic);
    // Fresnel-ish rim term: grazing angles reflect more than head-on ones on any real surface,
    // metal or not. Cheap Schlick approximation reusing the existing view/normal vectors, no
    // extra per-light cost since it only depends on view angle.
    let n_dot_v = max(dot(n, v), 0.0);
    let fresnel = pow(1.0 - n_dot_v, 5.0);
    let rim_strength = mix(0.05, 0.35, metallic) * fresnel;

    let n_lights = i32(globals.counts.x);
    let shadow_idx = i32(globals.counts.y);

    for (var i = 0; i < 16; i = i + 1) {
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
            let soft = dist / LAMP_SOFT_RADIUS;
            atten = falloff * falloff * POINT_GAIN / (1.0 + soft * soft * 0.5);
        }
        // Point lights "wrap" a little past the terminator so ceilings and walls seen at a grazing
        // angle from a lamp still pick up light (a flat Lambert term left ceilings pitch dark
        // next to a bright lamp band). Sun-style directional light keeps the sharp terminator.
        var n_dot_l = max(dot(n, l), 0.0);
        if (pod.w >= 0.5) {
            n_dot_l = clamp((dot(n, l) + LAMP_WRAP) / (1.0 + LAMP_WRAP), 0.0, 1.0);
        }
        if (n_dot_l <= 0.0) {
            continue;
        }
        var shadow = 1.0;
        if (i == shadow_idx) {
            shadow = shadow_factor(in.world_pos, n_dot_l);
        }
        let h = normalize(l + v);
        let spec_pow = pow(max(dot(n, h), 0.0), shininess);
        let spec = spec_pow * mix(0.15, 1.0, metallic) + rim_strength * n_dot_l;
        color = color + (diffuse_color * n_dot_l + vec3<f32>(spec, spec, spec)) * ci.rgb * atten * shadow;
    }

    color = color + obj.emissive.rgb;
    // Exposure + filmic tone-mapping: rolls strong/overlapping lights off toward white instead of
    // hard-clipping, so an author's light intensities don't need to be perfectly balanced.
    color = aces(color * EXPOSURE);
    return vec4<f32>(color, 1.0);
}
