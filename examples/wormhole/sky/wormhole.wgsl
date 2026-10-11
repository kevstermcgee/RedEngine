// Wormhole: an endless fall through a tunnel of light.
//
// The engine calls `sky(dir, px)` for every pixel: `dir` is where that pixel looks, `px` the angle it spans. Nothing in the world is geometry;
// the tunnel is found by intersecting the view ray with a wall around the travel axis (-Z is ahead), so looking anywhere (ahead, aside,
// behind) sees the same endless place.
//
// Variety, never a loop:
//   * colour and pattern belong to STRETCHES of the tunnel: every stretch (`STRETCH` units long) draws its own palette, pattern mix, twist,
//     arms and ripples from a hash of (seed, stretch number), so new worlds appear at the far end and roar past you;
//   * the tunnel's SHAPE (round, polygon, petals, the two planes of a slit-scan corridor) morphs over time on its own, slower clock;
//   * pulses of light are spawned at random moments and race toward you faster than the walls;
//   * speed breathes. Every choice comes from the seed (a new one each launch) and an ever-growing index, so no stretch repeats.

const TAU: f32 = 6.28318530718;
const PI: f32 = 3.14159265359;
const RADIUS: f32 = 3.0;
const STRETCH: f32 = 150.0;
const SHAPE_SECS: f32 = 41.0;

// ---- Small helpers ------------------------------------------------------------------------------------------------------------------------

fn h1(k: f32, salt: f32) -> f32 {
    return hash13(vec3<f32>(k * 1.37 + 0.11, salt * 7.31 + 1.7, sky_seed() * 0.0137 + 3.3));
}

fn h3(k: f32, salt: f32) -> vec3<f32> {
    return hash33(vec3<f32>(k * 0.917 + 5.3, salt * 3.71 + 0.3, sky_seed() * 0.0091 + 1.9));
}

// Three octaves of value noise: the engine's `fbm` has five, and this runs three times a pixel.
fn fbm3(p: vec3<f32>) -> f32 {
    return 0.5 * vnoise(p) + 0.25 * vnoise(p * 2.03 + vec3<f32>(17.1, 3.7, 9.2)) + 0.125 * vnoise(p * 4.1 + vec3<f32>(5.3, 11.9, 2.1));
}

// Inigo Quilez's cosine palette.
fn palette(t: f32, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return a + b * cos(TAU * (c * t + d));
}

// How far we have fallen: speed breathes between about 4 and 9 units a second and never stops.
fn travel(t: f32) -> f32 {
    return 6.2 * t + 22.0 * sin(t * 0.071) + 9.0 * sin(t * 0.0437 + 1.3);
}

fn speed(t: f32) -> f32 {
    return 6.2 + 22.0 * 0.071 * cos(t * 0.071) + 9.0 * 0.0437 * cos(t * 0.0437 + 1.3);
}

// ---- The tunnel's cross-section ----------------------------------------------------------------------------------------------------------

// The wall's "closeness" in the direction `a` (radians round the axis) for shape `kind`: 1 for a round tunnel; less where the wall is
// further away. The ray meets the wall at distance RADIUS / (closeness * sideways part of the ray).
fn closeness(kind: f32, a: f32, sides: f32, spin: f32, petal: f32) -> f32 {
    let k = floor(kind);
    if (k < 1.0) {
        return 1.0;
    }
    if (k < 2.0) {
        // A regular polygon of `sides`.
        let seg = TAU / sides;
        let x = (a + spin) - seg * floor((a + spin) / seg) - seg * 0.5;
        return cos(x) / cos(PI / sides);
    }
    if (k < 3.0) {
        // Petals: the wall breathes in and out round the axis.
        return 1.0 / (1.0 + petal * (0.5 + 0.5 * cos(sides * (a + spin))));
    }
    // A slit-scan corridor: a wide lens, nearly two planes of light (turned by `spin`). Not quite planes: the wall is addressed by its angle round
    // the axis, which would smear where true planes run off sideways to their horizon.
    return 0.3 + 0.7 * abs(sin(a + spin));
}

struct Shape {
    kind_a: f32,
    kind_b: f32,
    mix_ab: f32,
    sides_a: f32,
    sides_b: f32,
    petal_a: f32,
    petal_b: f32,
    spin: f32,
};

fn shape_at(t: f32) -> Shape {
    let e = floor(t / SHAPE_SECS);
    let f = fract(t / SHAPE_SECS);
    var s: Shape;
    // The first epoch is always the round tunnel: the fall begins in the classic wormhole.
    s.kind_a = select(floor(h1(e, 11.0) * 4.0), 0.0, e < 0.5);
    s.kind_b = floor(h1(e + 1.0, 11.0) * 4.0);
    s.sides_a = 3.0 + floor(h1(e, 12.0) * 6.0);
    s.sides_b = 3.0 + floor(h1(e + 1.0, 12.0) * 6.0);
    s.petal_a = 0.15 + 0.6 * h1(e, 13.0);
    s.petal_b = 0.15 + 0.6 * h1(e + 1.0, 13.0);
    s.mix_ab = smoothstep(0.78, 1.0, f);
    s.spin = t * 0.05 + 0.6 * sin(t * 0.013);
    return s;
}

fn shape_closeness(s: Shape, a: f32) -> f32 {
    let ca = closeness(s.kind_a, a, s.sides_a, s.spin, s.petal_a);
    let cb = closeness(s.kind_b, a, s.sides_b, s.spin * 1.3, s.petal_b);
    return mix(ca, cb, s.mix_ab);
}

// ---- A stretch of tunnel: its palette and its patterns -----------------------------------------------------------------------------------

struct Look {
    pa: vec3<f32>,
    pb: vec3<f32>,
    pc: vec3<f32>,
    pd: vec3<f32>,
    w_rings: f32,
    w_ink: f32,
    w_cells: f32,
    w_ribbons: f32,
    w_rays: f32,
    arms: f32,
    twist: f32,
    ring_freq: f32,
    ink_scale: f32,
    cells_u: f32,
    ribbon_n: f32,
    ribbon_amp: f32,
    hue_drift: f32,
    glow: vec3<f32>,
    dark: f32,
};

fn look_of(k: f32) -> Look {
    var l: Look;
    let r0 = h3(k, 1.0);
    let r1 = h3(k, 2.0);
    let r2 = h3(k, 3.0);
    let r3 = h3(k, 4.0);
    // The palette: centred and wide (vivid), with a random spread of hue across the pattern and a random hue offset per channel.
    l.pa = vec3<f32>(0.5);
    l.pb = vec3<f32>(0.5) + 0.1 * (r0 - 0.5);
    let family = floor(r3.x * 4.0);
    if (family < 1.0) {
        l.pc = vec3<f32>(1.0, 1.0, 1.0) * (0.6 + r0.x * 0.9);
    } else if (family < 2.0) {
        l.pc = vec3<f32>(1.0, 1.0, 0.5) * (0.7 + r0.y * 0.6);
    } else if (family < 3.0) {
        l.pc = vec3<f32>(2.0, 1.0, 0.0) * (0.3 + r0.z * 0.3);
    } else {
        l.pc = vec3<f32>(1.0, 0.7, 0.4) * (0.6 + r1.x * 0.7);
    }
    l.pd = r1;
    // Two or three patterns lead, the rest whisper.
    let w = h3(k, 5.0);
    let w2 = h3(k, 6.0);
    l.w_rings = pow(w.x, 1.6);
    l.w_ink = pow(w.y, 1.3);
    l.w_cells = pow(w.z, 3.0);
    l.w_ribbons = pow(w2.x, 1.6);
    l.w_rays = pow(w2.y, 2.5);
    let total = l.w_rings + l.w_ink + l.w_cells + l.w_ribbons + l.w_rays + 1e-3;
    l.w_rings = l.w_rings / total;
    l.w_ink = l.w_ink / total;
    l.w_cells = l.w_cells / total;
    l.w_ribbons = l.w_ribbons / total;
    l.w_rays = l.w_rays / total;
    l.arms = floor(r2.x * 7.0) - 3.0;
    l.twist = (r2.y - 0.5) * 1.6;
    l.ring_freq = 0.18 + r2.z * 0.55;
    l.ink_scale = 0.06 + r3.y * 0.14;
    l.cells_u = 4.0 + floor(r3.z * 12.0);
    l.ribbon_n = 1.0 + floor(w2.z * 5.0);
    l.ribbon_amp = 0.4 + h1(k, 7.0) * 1.6;
    l.hue_drift = (h1(k, 8.0) - 0.5) * 0.02;
    l.glow = palette(h1(k, 9.0), l.pa, l.pb, l.pc, l.pd);
    l.dark = 0.25 + 0.5 * h1(k, 10.0);
    return l;
}

// The light on the wall at angle `a` (radians), depth coordinate `v` (units along the tunnel), for look `l`. `fu`, `fv`: how much of a cycle
// of a unit-frequency pattern one pixel covers round and along the wall (fine patterns fade out where they would alias).
fn wall(l: Look, a: f32, v: f32, t: f32, fu: f32, fv: f32) -> vec3<f32> {
    let u = a / TAU;
    let cs = vec2<f32>(cos(a), sin(a));
    // Rings and helices: bands along the tunnel, wound into spirals by `arms`.
    let ph = v * l.ring_freq + l.arms * u + l.twist * sin(v * 0.05 + t * 0.2);
    let ring_aa = clamp(1.0 - fv * l.ring_freq * 3.0 - fu * abs(l.arms) * 3.0, 0.0, 1.0);
    let ring = pow(0.5 + 0.5 * sin(TAU * ph), 3.0) * ring_aa + 0.12 * (1.0 - ring_aa);
    // Ink: domain-warped noise, liquid colour flowing along the wall.
    // (Skipped where the stretch barely uses it: it is the costly one.)
    var ink = 0.0;
    var ink_v = 0.5;
    if (l.w_ink > 0.04) {
        let p = vec3<f32>(cs * 2.2, v * l.ink_scale);
        let warp = vec3<f32>(fbm3(p + vec3<f32>(0.0, 0.0, t * 0.05)), fbm3(p + vec3<f32>(5.2, 1.3, -t * 0.04)), 0.0);
        ink_v = fbm3(p + 2.9 * warp + vec3<f32>(t * 0.02, 0.0, 0.0)) / 0.875;
        // Veins rather than blobs: light gathers where the warped noise crosses its middle.
        let vein = 1.0 - abs(ink_v * 2.0 - 1.0);
        ink = pow(clamp(vein, 0.0, 1.0), 6.0) * 1.3 + 0.25 * smoothstep(0.55, 0.9, ink_v);
    }
    // Cells: a lattice of tiles round and along the wall, each lit at its own moment.
    let cu = u * l.cells_u;
    let cv = v * l.ring_freq * 0.7;
    let cell = vec2<f32>(floor(cu), floor(cv));
    let cell_rnd = hash13(vec3<f32>(cell, 3.0 + sky_seed() * 0.001));
    let fcell = vec2<f32>(fract(cu), fract(cv)) - 0.5;
    let edge = 1.0 - smoothstep(0.32, 0.5, max(abs(fcell.x), abs(fcell.y)));
    let cells_aa = clamp(1.0 - fv * l.ring_freq * 2.0 - fu * l.cells_u * 2.0, 0.0, 1.0);
    let cells = (edge * (0.25 + 0.75 * pow(0.5 + 0.5 * sin(t * 1.3 + cell_rnd * 40.0), 4.0))) * cells_aa + 0.2 * (1.0 - cells_aa);
    // Ribbons: bright bands that wave round the wall like an aurora.
    let rib_ph = l.ribbon_n * a + l.ribbon_amp * sin(v * 0.09 + t * 0.6) + 0.7 * sin(v * 0.031 - t * 0.3);
    let ribbons = pow(0.5 + 0.5 * cos(rib_ph), 8.0);
    // Rays: spokes running along the tunnel, flickering with depth.
    let rays = pow(0.5 + 0.5 * cos(a * floor(4.0 + l.cells_u)), 12.0) * (0.6 + 0.4 * sin(v * 0.4 - t * 3.0));

    let level = l.w_rings * ring + l.w_ink * ink + l.w_cells * cells + l.w_ribbons * ribbons + l.w_rays * rays;
    // Colour: across the palette with the pattern, drifting with depth and slowly with time.
    let idx = level * 0.8 + v * l.hue_drift + 0.15 * ink_v + t * 0.01;
    let col = palette(idx, l.pa, l.pb, l.pc, l.pd);
    // Light on dark: most of the wall is deep and coloured, the pattern burns bright.
    let lit = pow(smoothstep(0.12, 1.0, level), 2.4);
    let deep = col * col * 0.06 * l.dark;
    return deep + col * 3.2 * lit;
}

// ---- Streaks: specks of light stretched by the speed -------------------------------------------------------------------------------------

fn streaks(a: f32, v: f32, t: f32, fv: f32) -> vec3<f32> {
    let n = 48.0;
    let u = a / TAU * n;
    let len = 3.0 + speed(t) * 0.6;
    let cell = vec2<f32>(floor(u), floor(v / len));
    let r = hash33(vec3<f32>(cell, 9.0 + sky_seed() * 0.003));
    if (r.x > 0.12) {
        return vec3<f32>(0.0);
    }
    let du = fract(u) - (0.2 + 0.6 * r.y);
    let dv = fract(v / len) - 0.5;
    let width = 0.025 + 0.035 * r.z;
    let body = exp(-du * du / (width * width)) * smoothstep(0.5, 0.0, abs(dv)) * pow(1.0 - abs(dv) * 2.0, 2.0);
    let col = mix(vec3<f32>(0.7, 0.85, 1.0), vec3<f32>(1.0, 0.8, 0.6), r.y);
    return col * body * 0.9 * clamp(1.0 - fv * 0.2, 0.0, 1.0);
}

// ---- Pulses: waves of colour that race toward you ----------------------------------------------------------------------------------------

fn pulses(a: f32, z: f32, t: f32) -> vec3<f32> {
    var acc = vec3<f32>(0.0);
    let every = 5.5;
    let now = floor(t / every);
    for (var i = 0; i < 4; i = i + 1) {
        let j = now - f32(i);
        let born = (j + h1(j, 21.0) * 0.8) * every;
        let age = t - born;
        if (age < 0.0 || h1(j, 22.0) > 0.7) {
            continue;
        }
        // Born far ahead, rushing in at two to four times the fall, splashing out as it passes.
        let rush = 18.0 + 22.0 * h1(j, 23.0);
        let front = 140.0 - age * rush;
        let width = 2.0 + 5.0 * h1(j, 24.0);
        let wobble = 1.5 * sin(a * (2.0 + floor(h1(j, 25.0) * 5.0)) + t * 2.0) * h1(j, 26.0);
        let d = (z - front - wobble) / width;
        let band = exp(-d * d) * smoothstep(-25.0, 5.0, front) * smoothstep(140.0, 110.0, front);
        let c = palette(h1(j, 27.0), vec3<f32>(0.5), vec3<f32>(0.5), vec3<f32>(1.0), h3(j, 28.0));
        acc = acc + c * band * 1.8;
    }
    return acc;
}

// ---- The sky --------------------------------------------------------------------------------------------------------------------------

fn sky(dir: vec3<f32>, px: f32) -> vec3<f32> {
    let t = sky_time();
    let fall = travel(t);
    // Sideways part of the ray and its angle round the travel axis (-Z ahead).
    let side = max(length(dir.xy), 1e-4);
    let a0 = atan2(dir.y, dir.x);
    let shape = shape_at(t);
    let c = max(shape_closeness(shape, a0), 0.05);
    // Where the ray meets the wall: `z` is how far along the tunnel (positive ahead, negative behind).
    let s = RADIUS / (c * side);
    let z = s * -dir.z;
    // The tunnel winds: the wall's angle twists slowly with depth, so the far end wanders.
    let a = a0 + 0.35 * sin(z * 0.021 + t * 0.11) + 0.25 * sin(z * 0.009 - t * 0.07);
    let v = fall + z;
    // How much wall one pixel spans (see `wall`): round the axis, and along it (which grows quickly toward the far end).
    let fu = px / side / TAU;
    let fv = px * RADIUS / (c * side * side);

    // The stretch this point belongs to, blended into the next over the last third, so a new world arrives smoothly.
    let kf = v / STRETCH;
    let k = floor(kf);
    let blend = smoothstep(0.62, 1.0, fract(kf));
    var col = wall(look_of(k), a, v, t, fu, fv);
    if (blend > 0.0) {
        col = mix(col, wall(look_of(k + 1.0), a, v, t, fu, fv), blend);
    }
    col = col + streaks(a, v, t, fv) + pulses(a, z, t);

    // Far away the wall dissolves into the light at the end of the tunnel (ahead) or the dark we came from (behind).
    // (By the length of the ray, not the depth: a corridor of two planes reaches far away sideways too.)
    // Where one pixel covers metres of wall the detail would only shimmer: it dissolves into the haze as well.
    let fog = max(1.0 - exp(-s / 95.0), smoothstep(1.0, 6.0, fv));
    // The light ahead takes the colour of the stretch it comes from, easing into the next one's.
    let ahead = (fall + 90.0) / STRETCH;
    let glow = mix(look_of(floor(ahead)).glow, look_of(floor(ahead) + 1.0).glow, smoothstep(0.55, 1.0, fract(ahead)));
    let core = glow * 1.4 + vec3<f32>(0.25);
    let far_col = select(core * 0.04, core * 0.22, z > 0.0);
    col = mix(col, far_col, fog);
    // The bright heart of the tunnel, and a soft bloom round it.
    let axis = max(-dir.z, 0.0);
    let heart = pow(axis, 400.0) * 2.5 + pow(axis, 40.0) * 0.35;
    col = col + core * heart;

    // Richer colour, then a gentle filmic curve so nothing clips harshly.
    let luma = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
    col = max(mix(vec3<f32>(luma), col, 1.35), vec3<f32>(0.0));
    let exposure = 1.0;
    return vec3<f32>(1.0) - exp(-col * exposure);
}
