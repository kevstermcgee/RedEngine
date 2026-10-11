// Declarations shared by scene.wgsl, shadow.wgsl and background.wgsl. `gpu.rs` concatenates this file in front of
// each of them at compile time (`concat!(include_str!(..))`), so `Globals` and `ObjectUniform` exist exactly once: a
// layout change made here reaches every shader, and `gpu::tests` checks the Rust mirrors (`GlobalUniform`,
// `ObjectUniform`) against these definitions without needing a GPU.
//
// MAX_LIGHTS must equal `schema::MAX_LIGHTS` (tested).
const MAX_LIGHTS: u32 = 16u;

struct Globals {
    view_proj: mat4x4<f32>,
    // Sun shadow cascades (see shadow.rs): the matrix of each, its rectangle of the atlas (uv offset xy, scale zw) and
    // x: metres per shadow texel, y: depth units per metre. counts.z says how many are in use.
    cascade_vp: array<mat4x4<f32>, 3>,
    cascade_rect: array<vec4<f32>, 3>,
    cascade_params: array<vec4<f32>, 3>,
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
    audio: array<vec4<f32>, 12>,
    game: array<vec4<f32>, 8>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct ObjectUniform {
    model: mat4x4<f32>,
    normal_mat: mat4x4<f32>,
    base_color: vec4<f32>,
    material: vec4<f32>,
    emissive: vec4<f32>,
    // A game-painted surface (`museum.art`, see `surface_hook` in scene.wgsl): x > 0 turns it on; the game's `surface` reads the rest and `palette`.
    surface: vec4<f32>,
    palette: array<vec4<f32>, 4>,
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

// ---- A game's own shaders (`sky.shader`, `museum.art`): what they may call ------------------------------------------------------------
// These live here, in every module, so a game's code reads the same helpers whether it draws the sky or a surface.

// What a game's `surface` function (`museum.art`) is given for one pixel of a painted object: the point and normal in the object's own space (a canvas is
// the unit square x, y in -0.5..0.5 facing +z), the size of a pixel there, the world point and normal, and the object's `surface` numbers and palette.
struct SurfaceIn {
    p: vec3<f32>,
    n: vec3<f32>,
    px: f32,
    world: vec3<f32>,
    world_n: vec3<f32>,
    params: vec4<f32>,
    palette: array<vec4<f32>, 4>,
};

// Eight numbers a game's client sets every frame (`LiveRenderer::set_game_params`): what its sky and surfaces should show now. All zero otherwise.
fn game_param(i: i32) -> vec4<f32> {
    return globals.game[clamp(i, 0, 7)];
}

// Seconds since the scene began (the scene time every shader sees); with an `audio.playlist`, seconds of listening, which stop while the music is
// paused (so pausing freezes the picture with the music).
fn sky_time() -> f32 {
    return select(globals.night.y, globals.audio[10].z, globals.audio[10].w > 0.5);
}

// The sky's seed: `sky.seed`, or a new whole number each launch with "seed": "launch" (below 2^24, so exact as a float).
fn sky_seed() -> f32 {
    return globals.sky.w;
}

// What the music sounds like (a scene's `audio.playlist`; all 0 without one, so a sky still draws in silence). `sky_band(i)`: band i of 32, low to high,
// 0..1 against the loudest moments so far. `sky_bass/mid/treble/level()`: 0..1. `sky_beat()`: 1 at a beat, gone in about half a second;
// `sky_beats()`: beats so far in this song. `sky_song()`: the same for every play of a song; `sky_play()`: new every time a song starts (both whole
// numbers below 2^24). `sky_song_time()`: seconds into the song; `sky_playing()`: 0 to 1, eased.
fn sky_band(i: i32) -> f32 {
    let k = clamp(i, 0, 31);
    return globals.audio[k / 4][k % 4];
}

fn sky_level() -> f32 {
    return globals.audio[8].x;
}

fn sky_bass() -> f32 {
    return globals.audio[8].y;
}

fn sky_mid() -> f32 {
    return globals.audio[8].z;
}

fn sky_treble() -> f32 {
    return globals.audio[8].w;
}

fn sky_beat() -> f32 {
    return globals.audio[9].x;
}

fn sky_beats() -> f32 {
    return globals.audio[9].y;
}

fn sky_song_time() -> f32 {
    return globals.audio[9].z;
}

fn sky_playing() -> f32 {
    return globals.audio[9].w;
}

fn sky_song() -> f32 {
    return globals.audio[10].x;
}

fn sky_play() -> f32 {
    return globals.audio[10].y;
}

// ---- Noise -------------------------------------------------------------------------------------------------------------------------

fn hash13(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q = q + dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn hash33(p: vec3<f32>) -> vec3<f32> {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q = q + dot(q, q.yxz + 33.33);
    return fract((q.xxy + q.yxx) * q.zyx);
}

fn vnoise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(hash13(i), hash13(i + vec3<f32>(1.0, 0.0, 0.0)), u.x), mix(hash13(i + vec3<f32>(0.0, 1.0, 0.0)), hash13(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(hash13(i + vec3<f32>(0.0, 0.0, 1.0)), hash13(i + vec3<f32>(1.0, 0.0, 1.0)), u.x), mix(hash13(i + vec3<f32>(0.0, 1.0, 1.0)), hash13(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    var s = 0.0;
    var a = 0.5;
    var q = p;
    for (var i = 0; i < 5; i = i + 1) {
        s = s + a * vnoise(q);
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.2);
        a = a * 0.5;
    }
    return s;
}
