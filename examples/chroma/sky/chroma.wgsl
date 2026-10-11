// Chroma: music you can see. The engine calls `sky(dir, px)` for every pixel; the music arrives through sky_band/bass/mid/treble/level/beat/beats
// (a scene's `audio.playlist`) and `sky_time()` stops while the music is paused.
//
// The picture is drawn in screen space (the view ray projected onto the camera's own axes), so it fills the window whichever way the camera points.
// A listen is a run of SCENES: each lasts 14 to 30 seconds and crossfades into the next. A scene is one of eight MODES (halo, kaleidoscope, tunnel,
// waves, blobs, grid, starburst, polygons) with its own palette and shape parameters.
//
// Each song has its own character: its seed (`sky_song()`) picks the palette family and three favourite modes it returns to. Each play of it is new:
// the play seed (`sky_play()`) orders the scenes, shifts the colours and draws every shape parameter, so the same song never looks the same twice.

const TAU: f32 = 6.28318530718;
const PI: f32 = 3.14159265359;
const MODES: f32 = 8.0;

// ---- Small helpers ------------------------------------------------------------------------------------------------------------------------

fn h2(a: f32, b: f32) -> f32 {
    return hash13(vec3<f32>(a * 0.1371 + 1.3, b * 0.731 + 7.1, a * 0.0173 + b * 0.0311 + 3.7));
}

fn h3v(a: f32, b: f32) -> vec3<f32> {
    return hash33(vec3<f32>(a * 0.917 + 5.3, b * 3.71 + 0.3, a * 0.013 + 1.9));
}

fn wrap1(x: f32) -> f32 {
    return x - floor(x);
}

fn palette(t: f32, c: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.5) + vec3<f32>(0.5) * cos(TAU * (c * t + d));
}

// The spectrum at x (0 lowest band .. 1 highest), between bands.
fn band_at(x: f32) -> f32 {
    let f = clamp(x, 0.0, 1.0) * 31.0;
    let i = i32(floor(f));
    return mix(sky_band(i), sky_band(i + 1), fract(f));
}

// The music, with a gentle breathing in its place when nothing plays (the screen never goes dead).
struct Ear {
    bass: f32,
    mid: f32,
    treble: f32,
    level: f32,
    beat: f32,
    live: f32,
};

fn ear(t: f32) -> Ear {
    var e: Ear;
    let live = clamp(sky_playing(), 0.0, 1.0);
    let idle = 0.25 + 0.12 * sin(t * 0.7);
    e.bass = mix(idle, sky_bass(), live);
    e.mid = mix(idle * 0.8, sky_mid(), live);
    e.treble = mix(idle * 0.6, sky_treble(), live);
    e.level = mix(idle, sky_level(), live);
    e.beat = sky_beat() * live;
    e.live = live;
    return e;
}

// The spectrum, or a slow wave across it in silence.
fn spec(x: f32, t: f32, live: f32) -> f32 {
    let idle = 0.3 + 0.2 * sin(x * 9.0 - t * 1.3) * sin(t * 0.4 + x * 3.0);
    return mix(idle, band_at(x), live);
}

// ---- What one scene is ---------------------------------------------------------------------------------------------------------------------

struct Look {
    mode: f32,
    pc: vec3<f32>,
    pd: vec3<f32>,
    p1: f32,
    p2: f32,
    p3: f32,
    p4: f32,
    spin: f32,
};

// The song's own seed and this play's, folded small so the hashes stay exact.
fn song_key() -> f32 {
    return sky_song() - 9973.0 * floor(sky_song() / 9973.0);
}

fn play_key() -> f32 {
    // This play, of this song (two songs never share a run of scenes, even if their play seeds met).
    let p = sky_play() + sky_seed() * 0.37 + song_key() * 1.618;
    return p - 7919.0 * floor(p / 7919.0);
}

// The mode of scene `k`: one of the song's three favourites more often than not, otherwise any; never the one before.
fn mode_raw(k: f32) -> f32 {
    let s = song_key();
    let p = play_key();
    let r = h2(p + k * 13.1, 2.0);
    if (r < 0.45) {
        let which = floor(h2(p + k * 7.7, 3.0) * 3.0);
        return floor(h2(s, 10.0 + which) * MODES);
    }
    return floor(h2(p + k * 5.3, 4.0) * MODES);
}

fn mode_of(k: f32) -> f32 {
    var m = mode_raw(k);
    if (k > 0.5 && m == mode_raw(k - 1.0)) {
        m = wrap1((m + 1.0 + floor(h2(play_key() + k, 5.0) * (MODES - 1.0))) / MODES) * MODES;
        m = floor(m + 0.5);
        if (m >= MODES) {
            m = 0.0;
        }
    }
    return m;
}

fn look_of(k: f32) -> Look {
    var l: Look;
    let s = song_key();
    let p = play_key();
    l.mode = mode_of(k);
    // The palette: its family is the song's, its hues this play's and this scene's.
    let family = floor(h2(s, 1.0) * 5.0);
    let spread = 0.55 + 0.9 * h2(s, 6.0);
    if (family < 1.0) {
        l.pc = vec3<f32>(1.0, 1.0, 1.0) * spread;
    } else if (family < 2.0) {
        l.pc = vec3<f32>(1.0, 1.0, 0.5) * spread;
    } else if (family < 3.0) {
        l.pc = vec3<f32>(0.8, 1.0, 1.2) * spread;
    } else if (family < 4.0) {
        l.pc = vec3<f32>(2.0, 1.0, 0.0) * spread * 0.5;
    } else {
        l.pc = vec3<f32>(1.0, 0.7, 0.4) * spread;
    }
    l.pd = wrap1(h2(s, 7.0) + h2(p, 8.0) * 0.6) * vec3<f32>(1.0) + h3v(p + k * 3.1, 9.0) * 0.35 + vec3<f32>(0.0, 0.33, 0.67);
    l.p1 = h2(p + k * 1.7, 11.0);
    l.p2 = h2(p + k * 2.3, 12.0);
    l.p3 = h2(p + k * 3.9, 13.0);
    l.p4 = h2(p + k * 4.1, 14.0);
    l.spin = select(-1.0, 1.0, h2(p + k, 15.0) > 0.5) * (0.05 + 0.25 * h2(p + k, 16.0));
    return l;
}

fn col(l: Look, x: f32) -> vec3<f32> {
    return palette(x + sky_beats() * 0.021, l.pc, l.pd);
}

// ---- The eight modes: uv is screen space (y -1 bottom .. 1 top, x scaled by the aspect), t the listening clock -----------------------------

// 0. A halo: the spectrum as a ring of bars (or a smooth outline) around a pulsing heart, mirrored into folds, turning.
fn m_halo(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    let r = length(uv);
    let a = atan2(uv.y, uv.x) / TAU + t * l.spin * 0.2;
    let folds = 1.0 + floor(l.p1 * 3.0);
    let x = abs(wrap1(a * folds) * 2.0 - 1.0);
    let bars = 24.0 + floor(l.p2 * 40.0);
    let smooth_outline = l.p3 > 0.6;
    let cell = floor(x * bars) / bars;
    let v = spec(select(cell, x, smooth_outline), t, e.live);
    let r0 = 0.28 + 0.07 * e.bass + 0.06 * e.beat;
    let len = 0.08 + 0.55 * v;
    var c = vec3<f32>(0.0);
    if (smooth_outline) {
        let d = abs(r - (r0 + len));
        c = c + col(l, x * 0.6 + r) * (exp(-d * d / (0.0006 + aa)) * 1.6 + exp(-d * 9.0) * 0.35);
    } else {
        let gap = abs(fract(x * bars) - 0.5) < 0.38;
        let inside = r > r0 && r < r0 + len && gap;
        c = c + select(vec3<f32>(0.0), col(l, x * 0.6 + (r - r0) * 1.5) * (0.6 + 0.8 * (r - r0) / len), inside);
        let tip = abs(r - (r0 + len));
        c = c + col(l, x) * exp(-tip * 30.0) * 0.4 * select(0.0, 1.0, gap);
    }
    // The heart: a soft disc that breathes with the bass and flashes on the beat.
    c = c + col(l, 0.5 + t * 0.03) * (exp(-r * r / (0.008 + 0.02 * e.bass)) * (0.25 + 0.9 * e.beat));
    c = c + col(l, 0.2) * exp(-abs(r - r0) * 60.0) * 0.6;
    return c;
}

// 1. A kaleidoscope: warped noise folded into mirrored wedges, flowing inward, its rings driven by the bass.
fn m_kaleido(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    let r = length(uv);
    let n = 3.0 + floor(l.p1 * 7.0);
    let seg = TAU / n;
    var a = atan2(uv.y, uv.x) + t * l.spin;
    a = a - seg * floor(a / seg);
    a = abs(a - seg * 0.5);
    let flow = log(r + 0.02) * (1.2 + l.p2) - t * (0.25 + 0.4 * e.level);
    let q = vec2<f32>(cos(a), sin(a)) * (1.0 + 0.5 * l.p3);
    let w = fbm(vec3<f32>(q * 2.5, flow * 1.5));
    let v = fbm(vec3<f32>(q * 4.0 + w * 2.0, flow * 2.0 + 3.1));
    let ridge = pow(1.0 - abs(v * 2.0 - 1.0), 4.0 + 6.0 * l.p4);
    let rings = pow(0.5 + 0.5 * sin(flow * 10.0 - e.bass * 4.0), 6.0) * (0.3 + e.bass);
    var c = col(l, v + r * 0.5) * (ridge * (0.8 + 1.4 * e.mid) + rings * 0.6);
    c = c + col(l, 0.0) * exp(-r * 4.0) * (0.3 + e.beat);
    return c * smoothstep(0.0, 0.08, r + 0.02);
}

// 2. A tunnel: rings rushing out of the centre at the speed of the music, the spectrum painted round its wall, flashes on the beat.
fn m_tunnel(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    let r = max(length(uv), 0.001);
    let a = atan2(uv.y, uv.x) / TAU;
    let depth = 0.35 / r + t * (0.6 + 1.6 * e.level) * (0.7 + l.p1);
    let sides = 1.0 + floor(l.p2 * 6.0);
    let twist = a * sides + depth * (l.p3 - 0.5) * 0.6;
    let band = spec(abs(wrap1(a + t * l.spin * 0.1) * 2.0 - 1.0), t, e.live);
    let rings = pow(0.5 + 0.5 * cos(depth * TAU * (1.0 + l.p4)), 14.0);
    let ribs = pow(0.5 + 0.5 * cos(twist * TAU), 12.0);
    let fade = smoothstep(0.0, 0.25, r) * (1.0 - smoothstep(1.2, 2.2, r) * 0.5);
    let sym = abs(wrap1(a) * 2.0 - 1.0);
    var c = col(l, depth * 0.08 + sym * 0.5) * (rings * (0.3 + 1.8 * band) + ribs * (0.35 + 0.9 * e.treble) * (0.4 + band));
    c = c + col(l, depth * 0.08) * rings * e.beat * 1.5;
    c = c * fade + col(l, 0.5) * exp(-r * 9.0) * (0.25 + 0.8 * e.bass);
    return c;
}

// 3. Waves: ribbons of light across the screen, each one a slice of the spectrum, rippling.
fn m_waves(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    let lines = 5.0 + floor(l.p1 * 6.0);
    let freq = 1.5 + 4.0 * l.p2;
    for (var i = 0; i < 11; i = i + 1) {
        let fi = f32(i);
        if (fi >= lines) {
            break;
        }
        let x = fi / (lines - 1.0);
        let v = spec(x, t, e.live);
        let base = (x - 0.5) * 1.5 * (0.5 + 0.5 * l.p3);
        let y = base + (0.08 + 0.35 * v) * sin(uv.x * freq + t * (0.6 + fi * 0.13) * (1.0 + e.level) + fi * 1.7) * cos(uv.x * freq * 0.37 - t * 0.4 + fi);
        let d = abs(uv.y - y);
        let width = 0.004 + 0.01 * v + aa;
        c = c + col(l, x + uv.x * 0.1) * (exp(-d * d / (width * width)) * (0.6 + v) + exp(-d * 14.0) * 0.12 * (1.0 + e.beat));
    }
    return c;
}

// 4. Blobs: liquid light, a handful of bodies orbiting, each swelling with its part of the spectrum, melting where they meet.
fn m_blobs(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    var field = 0.0;
    var tint = 0.0;
    let count = 4.0 + floor(l.p1 * 4.0);
    for (var i = 0; i < 8; i = i + 1) {
        let fi = f32(i);
        if (fi >= count) {
            break;
        }
        let x = fi / (count - 1.0);
        let v = spec(x, t, e.live);
        let sp = (0.15 + 0.3 * h2(fi, l.p2 * 100.0)) * (1.0 + e.level);
        let c = vec2<f32>(sin(t * sp + fi * 2.1 + l.p3 * 6.0) * (0.4 + 0.6 * l.p4), cos(t * sp * 0.83 + fi * 1.3)) * vec2<f32>(1.2, 0.65);
        let rad = 0.12 + 0.3 * v + 0.06 * e.beat;
        let d2 = dot(uv - c, uv - c) + 1e-4;
        let f = rad * rad / d2;
        field = field + f;
        tint = tint + f * x;
    }
    let hue = tint / max(field, 1e-4);
    let edge = smoothstep(0.9, 1.0 + aa * 10.0, field);
    let rim = exp(-abs(field - 1.0) * 5.0);
    let body = 1.0 - exp(-(min(field, 8.0) - 1.0) * 0.6);
    // Lifted: a body never takes the palette's darkest colour (it would read as a hole).
    let c = col(l, hue + min(field, 2.0) * 0.08) * 0.8 + vec3<f32>(0.12);
    return c * (edge * (0.35 + 0.5 * body) + rim * 1.1) + col(l, hue) * min(field, 1.5) * 0.025;
}

// 5. A grid: a neon floor racing toward a striped sun, the spectrum standing as mountains on the horizon.
fn m_grid(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    let horizon = -0.05 + 0.15 * (l.p1 - 0.5);
    var c = vec3<f32>(0.0);
    if (uv.y < horizon) {
        let z = 0.35 / (horizon - uv.y + 0.001);
        let x = uv.x * z;
        let speed = 1.0 + 3.0 * e.level;
        let gx = abs(fract(x * (0.8 + l.p2)) - 0.5);
        let gz = abs(fract(z * 0.5 + t * speed * 0.5) - 0.5);
        let width = 0.015 * z + aa * z * 3.0;
        let lines = exp(-gx * gx / (width * width * 0.3)) + exp(-gz * gz / (width * width * 0.3));
        let fade = exp(-z * 0.06);
        c = col(l, 0.1 + z * 0.01) * lines * fade * (0.6 + 1.0 * e.bass + e.beat);
    } else {
        let h = uv.y - horizon;
        // The sun: banded, breathing with the bass.
        let sun_c = vec2<f32>(0.0, horizon + 0.32);
        let sr = 0.3 + 0.04 * e.bass;
        let sd = length(uv - sun_c);
        let bands = step(0.5, fract((uv.y - horizon) * (14.0 + 10.0 * l.p3) - t * 0.3)) * smoothstep(horizon, horizon + 0.35, uv.y) + step(sun_c.y - 0.05, uv.y);
        c = c + col(l, 0.6 + uv.y * 0.3) * smoothstep(sr + aa, sr - aa, sd) * min(bands, 1.0) * 1.2;
        c = c + col(l, 0.55) * exp(-sd * 3.0) * 0.25;
        // Mountains: the spectrum, mirrored from the middle.
        let m = spec(abs(uv.x) / 1.8, t, e.live) * (0.15 + 0.45 * l.p4);
        let ridge = h - m;
        c = select(c, col(l, 0.3 + abs(uv.x) * 0.2) * 0.12 + col(l, 0.9) * exp(-abs(ridge) * 90.0) * 1.2, ridge < 0.0);
        c = c + col(l, 0.9) * exp(-abs(ridge) * 60.0) * 0.5;
    }
    return c;
}

// 6. A starburst: streaks of light flung out of the centre, more of them with the treble, faster with the level, a shock ring on each beat.
fn m_stars(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    let r = length(uv);
    let a = atan2(uv.y, uv.x) / TAU + 0.5;
    let n = 140.0 + floor(l.p1 * 160.0);
    let cell = floor(a * n);
    let rnd = h3v(cell, l.p2 * 50.0);
    let on = rnd.x < 0.5 + 0.5 * e.treble;
    let speed = (0.3 + rnd.y) * (0.4 + 1.6 * e.level);
    let head = wrap1(rnd.z + t * speed * 0.35) * 2.0;
    let len = 0.1 + 0.35 * e.level;
    let along = (head - r) / len;
    let across = abs(fract(a * n) - 0.5);
    let streak = select(0.0, smoothstep(0.0, 0.2, along) * smoothstep(1.0, 0.0, along), on) * exp(-across * across / 0.02) * smoothstep(0.02, 0.15, r);
    var c = col(l, rnd.y + r * 0.4) * streak * 1.3;
    // Each beat throws a ring outward: its radius grows as the beat fades.
    let ring_r = (1.0 - e.beat) * 1.6;
    c = c + col(l, 0.7) * exp(-abs(r - ring_r) * 40.0) * e.beat * 1.4;
    c = c + col(l, 0.4) * exp(-r * 5.0) * (0.3 + 0.8 * e.bass);
    return c;
}

// 7. Polygons: nested shapes, every other one turning the other way, each breathing with its band.
fn m_poly(uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    let sides = 3.0 + floor(l.p1 * 6.0);
    let rings = 6.0 + floor(l.p2 * 5.0);
    for (var i = 0; i < 11; i = i + 1) {
        let fi = f32(i);
        if (fi >= rings) {
            break;
        }
        let x = fi / (rings - 1.0);
        let v = spec(x, t, e.live);
        let rot = t * l.spin * (1.0 + fi * 0.15) * select(-1.0, 1.0, i % 2 == 0) + fi * l.p3;
        let a = atan2(uv.y, uv.x) + rot;
        let seg = TAU / sides;
        let k = a - seg * floor(a / seg) - seg * 0.5;
        // Distance to a regular polygon of `sides` with apothem `rad`.
        let rad = (0.08 + fi * 0.11 * (0.8 + 0.4 * l.p4)) * (1.0 + 0.35 * v + 0.1 * e.beat);
        let d = abs(length(uv) * cos(k) - rad);
        let width = 0.003 + 0.012 * v + aa;
        c = c + col(l, x + t * 0.02) * (exp(-d * d / (width * width)) * (0.7 + v) + exp(-d * 18.0) * 0.08);
    }
    return c;
}

fn draw(m: f32, uv: vec2<f32>, t: f32, l: Look, e: Ear, aa: f32) -> vec3<f32> {
    if (m < 0.5) {
        return m_halo(uv, t, l, e, aa);
    } else if (m < 1.5) {
        return m_kaleido(uv, t, l, e, aa);
    } else if (m < 2.5) {
        return m_tunnel(uv, t, l, e, aa);
    } else if (m < 3.5) {
        return m_waves(uv, t, l, e, aa);
    } else if (m < 4.5) {
        return m_blobs(uv, t, l, e, aa);
    } else if (m < 5.5) {
        return m_grid(uv, t, l, e, aa);
    } else if (m < 6.5) {
        return m_stars(uv, t, l, e, aa);
    }
    return m_poly(uv, t, l, e, aa);
}

// ---- The sky --------------------------------------------------------------------------------------------------------------------------

fn sky(dir: vec3<f32>, px: f32) -> vec3<f32> {
    // Screen space: the camera's forward, right and up from the view rays through the centre and the edges of the screen.
    let fwd = view_ray(vec2<f32>(0.0, 0.0));
    let up_edge = view_ray(vec2<f32>(0.0, 1.0));
    let right_edge = view_ray(vec2<f32>(1.0, 0.0));
    let tan_v = length(up_edge / dot(up_edge, fwd) - fwd);
    let up = normalize(up_edge / dot(up_edge, fwd) - fwd);
    let right = normalize(right_edge / dot(right_edge, fwd) - fwd);
    let f = max(dot(dir, fwd), 0.05);
    let uv = vec2<f32>(dot(dir, right), dot(dir, up)) / f / tan_v;
    let aa = px / tan_v;

    let t = sky_time();
    let e = ear(t);
    // Scenes: their length is this play's; the last sixth of each crossfades into the next.
    let span = 14.0 + 16.0 * h2(play_key(), 20.0);
    let k = floor(t / span);
    let blend = smoothstep(0.83, 1.0, fract(t / span));
    let now = look_of(k);
    var c = draw(now.mode, uv, t, now, e, aa);
    if (blend > 0.0) {
        let next = look_of(k + 1.0);
        c = mix(c, draw(next.mode, uv, t, next, e, aa), blend);
    }
    // A dark, faintly coloured ground behind everything, a whole-screen lift on the beat, a vignette.
    let r = length(uv);
    c = c + col(now, 0.8 + r * 0.2) * 0.004 * (1.0 + e.level) + col(now, 0.5) * e.beat * 0.02;
    c = c * (1.0 - 0.35 * smoothstep(0.8, 1.9, r));
    let luma = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = max(mix(vec3<f32>(luma), c, 1.25), vec3<f32>(0.0));
    return vec3<f32>(1.0) - exp(-c * 1.15);
}
