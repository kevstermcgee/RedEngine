@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

@group(1) @binding(0) var<uniform> obj: ObjectUniform;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) sway: f32,
};
struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) vertex_color: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var world = obj.model * vec4<f32>(in.pos, 1.0);
    world = vec4<f32>(world.xyz + wind_offset(in.sway, world.xyz), 1.0);
    var out: VsOut;
    out.clip_pos = globals.view_proj * world;
    out.world_pos = world.xyz;
    out.world_normal = normalize((obj.normal_mat * vec4<f32>(in.normal, 0.0)).xyz);
    out.vertex_color = in.color;
    return out;
}

// ---- Sun shadows: cascades in one atlas (shadow.rs) ----------------------------------------------------------------------------------

const ATLAS_TEXEL: vec2<f32> = vec2<f32>(1.0 / 3072.0, 1.0 / 2048.0);
const SHADOW_TAPS: i32 = 8;

// Interleaved gradient noise: a cheap per-pixel value in 0..1 that rotates the filter, so the softness is smooth grain instead of bands.
fn gradient_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

// Where a world point lands in cascade `c`: xy in -1..1 across its box, z the depth. The point is first pushed along the surface normal by a texel or so, which
// removes self-shadow acne without the detached look of a big depth bias.
fn cascade_ndc(c: u32, world_pos: vec3<f32>, n: vec3<f32>, n_dot_l: f32) -> vec3<f32> {
    let texel = globals.cascade_params[c].x;
    let offset = n * texel * (1.2 + 2.0 * (1.0 - n_dot_l));
    let lp = globals.cascade_vp[c] * vec4<f32>(world_pos + offset, 1.0);
    return lp.xyz / lp.w;
}

fn cascade_lit(c: u32, ndc: vec3<f32>, noise: f32) -> f32 {
    let rect = globals.cascade_rect[c];
    let uv = rect.xy + vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * rect.zw;
    let lo = rect.xy + ATLAS_TEXEL * 3.0;
    let hi = rect.xy + rect.zw - ATLAS_TEXEL * 3.0;
    // A little over a texel of constant bias on top of the normal offset.
    let z = ndc.z - globals.cascade_params[c].x * globals.cascade_params[c].y * 1.5;
    let spin = noise * 6.2831853;
    var lit = 0.0;
    for (var i = 0; i < SHADOW_TAPS; i = i + 1) {
        // A Vogel disc: evenly spread taps, rotated per pixel, about 1.6 texels across at the edge.
        let r = sqrt((f32(i) + 0.5) / f32(SHADOW_TAPS)) * 1.6;
        let a = f32(i) * 2.3999632 + spin;
        let tap = clamp(uv + vec2<f32>(cos(a), sin(a)) * r * ATLAS_TEXEL, lo, hi);
        lit = lit + textureSampleCompare(shadow_map, shadow_sampler, tap, z);
    }
    return lit / f32(SHADOW_TAPS);
}

// How much of the sun reaches `world_pos` (1 lit, 0 shadowed): the sharpest cascade that holds the point, blended into the next one toward its edge; past the last
// one the shadow fades out rather than stopping at a line.
fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>, n_dot_l: f32, pixel: vec2<f32>) -> f32 {
    let count = u32(globals.counts.z);
    let noise = gradient_noise(pixel);
    for (var c = 0u; c < count; c = c + 1u) {
        let ndc = cascade_ndc(c, world_pos, n, n_dot_l);
        let edge = max(abs(ndc.x), abs(ndc.y));
        if (edge >= 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
            continue;
        }
        var lit = cascade_lit(c, ndc, noise);
        let blend = smoothstep(0.80, 0.97, edge);
        if (blend > 0.0) {
            if (c + 1u < count) {
                let next = cascade_ndc(c + 1u, world_pos, n, n_dot_l);
                if (max(abs(next.x), abs(next.y)) < 1.0 && next.z >= 0.0 && next.z <= 1.0) {
                    lit = mix(lit, cascade_lit(c + 1u, next, noise), blend);
                    return lit;
                }
            }
            lit = mix(lit, 1.0, blend);
        }
        return lit;
    }
    return 1.0;
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
    let albedo = obj.base_color.rgb * in.vertex_color;
    var color = (globals.ambient.rgb + vec3<f32>(AMBIENT_FLOOR)) * albedo * hemi;

    let metallic = obj.material.x;
    let roughness = max(obj.material.y, 0.04);
    // A gentler curve than a straight mix(8, 160, 1-roughness): keeps mid-roughness surfaces
    // (most props) from jumping straight to a hard plastic-looking highlight the way a linear
    // roughness->shininess mapping tends to.
    let smoothness = 1.0 - roughness;
    let shininess = mix(8.0, 160.0, smoothness * smoothness);
    let diffuse_color = albedo * (1.0 - metallic);
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
            shadow = shadow_factor(in.world_pos, n, n_dot_l, in.clip_pos.xy);
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
    // Haze: with a scene `clock` the distance fades into the horizon colour, warmed toward the sun's side at sunset (atmospheric perspective).
    if (globals.fog.w > 0.0) {
        let to_frag = in.world_pos - globals.camera_pos.xyz;
        let dist = length(to_frag);
        let f = 1.0 - exp(-dist * globals.fog.w);
        color = mix(color, haze_color(to_frag / max(dist, 1e-4)), f);
    }
    return vec4<f32>(color, obj.base_color.a);
}
