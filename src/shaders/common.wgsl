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
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    sky: vec4<f32>,
    moon_dir: vec4<f32>,
    night: vec4<f32>,
    celestial: array<vec4<f32>, 3>,
    glow: vec4<f32>,
    fog: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct ObjectUniform {
    model: mat4x4<f32>,
    normal_mat: mat4x4<f32>,
    base_color: vec4<f32>,
    material: vec4<f32>,
    emissive: vec4<f32>,
};

// ---- Sunset light (shared by the sky and the haze) -----------------------------------------------------------------------------------

// The glow low on the sun's side of the sky, plus a faint pink band opposite (the Belt of Venus), as a colour to add.
fn horizon_glow(dir: vec3<f32>) -> vec3<f32> {
    let g = globals.glow;
    if (g.w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let up = max(dir.y, 0.0);
    let sun_az = normalize(vec3<f32>(globals.sun_dir.x, 0.0, globals.sun_dir.z) + vec3<f32>(1e-5, 0.0, 0.0));
    let az = normalize(vec3<f32>(dir.x, 0.0, dir.z) + vec3<f32>(1e-5, 0.0, 0.0));
    let toward = dot(az, sun_az);
    let near = pow(max(toward, 0.0), 2.2);
    let wide = pow(max(toward * 0.5 + 0.5, 0.0), 3.0);
    // A strong low glow on the sun's side, a broad soft wash all round the horizon, climbing less far than the glow itself.
    let low = exp(-up * 5.5);
    let high = exp(-up * 1.9);
    var c = g.rgb * g.w * (near * (low * 0.95 + high * 0.30) + wide * low * 0.30);
    // Opposite the sun the shadow of the earth rises as a dusky blue band with a pink edge above it.
    let anti = max(-toward, 0.0);
    let belt = exp(-pow((up - 0.13) / 0.10, 2.0));
    c = c + vec3<f32>(1.0, 0.55, 0.62) * g.w * anti * anti * belt * 0.22;
    return c;
}

// The colour of the haze for a view direction (used by the scene shader): the horizon colour, warmed toward the sun's side by the sunset glow.
fn haze_color(dir: vec3<f32>) -> vec3<f32> {
    var c = globals.fog.rgb;
    if (globals.glow.w > 0.0) {
        let g = horizon_glow(vec3<f32>(dir.x, 0.0, dir.z));
        c = c + g * 0.55;
    }
    return c;
}

// ---- Wind -----------------------------------------------------------------------------------------------------------------------------------
// The displacement in metres of a vertex with sway weight `w` (the metres it moves at full gust) at world position `p`: a wave travelling
// across the field, so grass and flowers ripple instead of nodding together, under a slow gust envelope. The clock is the scene time.
fn wind_offset(w: f32, p: vec3<f32>) -> vec3<f32> {
    if (w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let t = globals.night.y;
    let phase = dot(p.xz, vec2<f32>(0.34, 0.2));
    let wave = sin(t * 1.7 - phase * 1.3) * 0.55 + sin(t * 2.9 - phase * 2.1 + 1.3) * 0.25;
    let gust = 0.55 + 0.45 * sin(t * 0.23 - phase * 0.11);
    let d = w * (wave * gust + 0.35 * gust);
    return vec3<f32>(0.86 * d, 0.0, 0.5 * d);
}
