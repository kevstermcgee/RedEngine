// Frameless: every painting, tapestry and sculpture surface in the museum, and (through the sky) the world inside a painting.
//
// The engine calls `surface(s)` for every pixel of a painted object (`museum.art`). s.params: x 1 = a canvas (s.p.xy is the unit square, -0.5..0.5),
// 2 = a solid (s.p, s.n are the sculpture's own space); y = style (canvas) or material (solid), 1-based in the collection's order; z = seed (0..1000);
// w = the canvas's width over its height. s.palette[0..3].rgb = four colours, darkest first; .w lanes: [0] panel offset and [1] panel width along x
// (a triptych's panels share one picture), [2] how alive (glow), [3] 1 = on the way to a key (a golden shimmer). Returns colour and glow.
// The sky (shaders/sky.wgsl) calls `art(...)` to draw a painting's world around the player.

struct Pal {
    a: vec3<f32>,
    b: vec3<f32>,
    c: vec3<f32>,
    d: vec3<f32>,
};

fn pal_of(s: SurfaceIn) -> Pal {
    return Pal(s.palette[0].rgb, s.palette[1].rgb, s.palette[2].rgb, s.palette[3].rgb);
}

// Through the four colours, dark to light.
fn ramp(p: Pal, x: f32) -> vec3<f32> {
    let t = clamp(x, 0.0, 1.0) * 3.0;
    if (t < 1.0) {
        return mix(p.a, p.b, smoothstep(0.0, 1.0, t));
    }
    if (t < 2.0) {
        return mix(p.b, p.c, smoothstep(0.0, 1.0, t - 1.0));
    }
    return mix(p.c, p.d, smoothstep(0.0, 1.0, t - 2.0));
}

fn h1(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

fn h2(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

fn n2(p: vec2<f32>, s: f32) -> f32 {
    return vnoise(vec3<f32>(p, s * 0.137));
}

fn f2(p: vec2<f32>, s: f32) -> f32 {
    return fbm(vec3<f32>(p, s * 0.137));
}

// Three-octave noise: cheaper, for things drawn many times.
fn f3(p: vec2<f32>, s: f32) -> f32 {
    var a = 0.5;
    var q = p;
    var v = 0.0;
    for (var i = 0; i < 3; i = i + 1) {
        v = v + a * n2(q, s);
        q = q * 2.1 + vec2<f32>(5.2, 1.3);
        a = a * 0.5;
    }
    return v / 0.875;
}

// A field of small round stars: `cells` across, about one cell in `1/odds` lit, twinkling.
fn stars(q: vec2<f32>, cells: f32, odds: f32, s: f32, t: f32) -> f32 {
    let z = q * cells;
    let cell = floor(z);
    let o = h2(cell + s);
    let lit = step(1.0 - odds, o.x);
    let at = vec2<f32>(0.2) + 0.6 * h2(cell + s + 7.0);
    let d = length(fract(z) - at);
    let size = 0.06 + 0.08 * o.y;
    return lit * smoothstep(size, size * 0.3, d) * (0.65 + 0.35 * sin(t * (1.0 + o.y * 3.0) + o.x * 40.0));
}

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, s, -s, c);
}

fn edge(d: f32, w: f32) -> f32 {
    return 1.0 - smoothstep(-w, w, d);
}

// Canvas grain: the weave of linen under the paint.
fn grain(uv: vec2<f32>) -> f32 {
    let w = sin(uv.x * 900.0) * sin(uv.y * 900.0);
    return 0.97 + 0.03 * w;
}

// ---- The styles ----------------------------------------------------------------------------------------------------------------------
// Each takes q (centred, aspect-correct: x in -aspect/2..aspect/2, y in -0.5..0.5), the seed, the palette, the time and the pixel size.

// 1. Ink: currents of colour folding into one another, never settling.
fn st_ink(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let z = q * 2.2;
    let w1 = vec2<f32>(f3(z + vec2<f32>(t * 0.02, 0.0), s), f3(z + vec2<f32>(5.2, 1.3 - t * 0.017), s));
    let w2 = vec2<f32>(f3(z + 3.0 * w1 + vec2<f32>(1.7, 9.2), s + 1.0), f3(z + 3.0 * w1 + vec2<f32>(8.3, 2.8 + t * 0.01), s + 1.0));
    let v = f3(z + 3.5 * w2, s + 2.0);
    var c = ramp(p, v * 1.1 - 0.05);
    // Dark veins where the currents meet.
    let vein = smoothstep(0.02, 0.0, abs(w2.x - 0.5) - 0.01);
    c = mix(c, p.a, vein * 0.6);
    c = c + p.d * pow(clamp(w2.y, 0.0, 1.0), 6.0) * 0.4;
    return c;
}

// 2. Colour field: soft rectangles of colour that breathe.
fn st_field(q: vec2<f32>, aspect: f32, s: f32, p: Pal, t: f32) -> vec3<f32> {
    var c = mix(p.a, p.b, 0.35 + 0.2 * h1(s));
    let n = 2 + i32(h1(s + 1.0) * 2.0);
    for (var i = 0; i < 3; i = i + 1) {
        if (i >= n) {
            break;
        }
        let fi = f32(i);
        let span = 0.84 / f32(n);
        let cy = 0.42 - span * (fi + 0.5);
        let breathe = 0.012 * sin(t * 0.25 + fi * 2.1 + s);
        let hh = span * 0.42 + breathe;
        let hw = aspect * 0.5 - 0.07 + breathe;
        let wob = (n2(q * vec2<f32>(3.0, 9.0) + vec2<f32>(fi * 7.0, 0.0), s) - 0.5) * 0.025;
        let d = max(abs(q.x) - hw, abs(q.y - cy) - hh) + wob;
        let col = select(select(p.c, p.d, h1(s + fi * 3.1) > 0.5), p.b, h1(s + fi * 5.7) > 0.7);
        let k = 1.0 - smoothstep(-0.02, 0.025, d);
        c = mix(c, col * (0.85 + 0.25 * f3(q * 6.0 + fi, s)), k);
    }
    return c;
}

// 3. Horizon: hills under a sky, mist between them, the sun or the moon, clouds going by.
fn st_horizon(q: vec2<f32>, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    let night = h1(s + 4.0) > 0.6;
    let y = q.y + 0.5;
    var c = mix(p.c, p.d, smoothstep(0.35, 1.0, y));
    if (night) {
        c = mix(p.a, p.b, smoothstep(0.2, 1.0, y));
    }
    let sun = vec2<f32>((h1(s) - 0.5) * 0.6, 0.12 + h1(s + 1.0) * 0.25);
    let sd = length(q - sun);
    let disc = smoothstep(0.075 + px, 0.075 - px, sd);
    c = c + select(p.d, vec3<f32>(1.0, 0.97, 0.9), night) * (disc * 1.2 + 0.25 * exp(-sd * 9.0));
    // Clouds drifting.
    let cl = f3(vec2<f32>(q.x * 2.5 + t * 0.01, q.y * 7.0) + vec2<f32>(s, 0.0), s);
    c = mix(c, mix(p.d, p.c, 0.3), smoothstep(0.55, 0.8, cl) * smoothstep(0.0, 0.3, q.y) * 0.6);
    // Hills, far to near.
    for (var i = 0; i < 4; i = i + 1) {
        let fi = f32(i);
        let base = 0.05 - fi * 0.11;
        let hgt = base + (f3(vec2<f32>(q.x * (1.5 + fi * 0.8) + s + fi * 3.0, fi), s) - 0.5) * (0.22 + fi * 0.05);
        let k = smoothstep(px * 2.0, -px * 2.0, q.y - hgt);
        let far = 1.0 - fi / 3.0;
        let col = mix(mix(p.b, p.a, fi / 3.0), c, far * 0.55);
        c = mix(c, col, k);
        // Mist settles in each valley.
        c = mix(c, p.d, (1.0 - k) * exp(-abs(q.y - hgt) * 40.0) * 0.18 * far);
    }
    return c;
}

// 4. Tide: the sea at night or dusk, the moon's road on the water, waves coming in.
fn st_tide(q: vec2<f32>, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    let hz = 0.06 + (h1(s) - 0.5) * 0.15;
    let moon = vec2<f32>((h1(s + 2.0) - 0.5) * 0.5, hz + 0.18 + h1(s + 3.0) * 0.12);
    var c: vec3<f32>;
    if (q.y > hz) {
        c = mix(p.c, p.a, smoothstep(hz, 0.5, q.y));
        let md = length(q - moon);
        c = c + p.d * (smoothstep(0.05 + px, 0.05 - px, md) + 0.35 * exp(-md * 12.0));
        // A few stars.
        c = c + vec3<f32>(1.0) * stars(q, 60.0, 0.06, s, t) * smoothstep(hz, hz + 0.2, q.y);
    } else {
        let depth = hz - q.y;
        // Waves: rows that roll toward the viewer.
        let row = depth * 40.0 / (0.15 + depth * 3.0);
        let wave = sin(row * 6.0 - t * 1.4 + n2(vec2<f32>(q.x * 6.0, row), s) * 4.0);
        c = mix(p.b, p.a, smoothstep(0.0, 0.5, depth));
        c = c + p.c * 0.25 * smoothstep(0.6, 1.0, wave);
        // The moon's road.
        let road = exp(-abs(q.x - moon.x) * (8.0 + depth * 30.0)) * smoothstep(0.3, 1.0, wave * 0.5 + 0.5 + n2(q * 40.0 + vec2<f32>(t, 0.0), s) * 0.5);
        c = c + p.d * road * 0.9;
        // Foam at the shore.
        let shore = -0.38 + 0.03 * sin(q.x * 9.0 + t * 0.7 + s);
        c = mix(c, p.d, smoothstep(0.02, 0.0, abs(q.y - shore)) * 0.6 * (0.6 + 0.4 * sin(t * 0.7)));
        if (q.y < shore) {
            c = mix(p.a, p.b, 0.3) * (0.8 + 0.2 * n2(q * 30.0, s));
        }
    }
    return c;
}

// 5. Starry: a night sky that swirls, stars with haloes, a sleeping village.
fn st_starry(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    var z = q * 2.0;
    // Swirl: the sky turns around a few eddies.
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        let ctr = vec2<f32>((h1(s + fi) - 0.5) * 1.6, 0.2 + h1(s + fi + 5.0) * 0.6);
        let d = z - ctr;
        let r = length(d);
        z = ctr + rot(0.9 * exp(-r * 2.0) * (1.0 + 0.3 * sin(t * 0.2 + fi))) * d;
    }
    let strokes = n2(vec2<f32>(z.x * 3.0, z.y * 18.0) + vec2<f32>(t * 0.05, 0.0), s);
    var c = mix(p.a, p.b, 0.4 + 0.5 * f3(z * 1.5, s));
    c = mix(c, p.c, smoothstep(0.55, 0.85, strokes) * 0.6);
    // Stars.
    for (var i = 0; i < 6; i = i + 1) {
        let fi = f32(i);
        let at = vec2<f32>((h1(s + fi * 7.0) - 0.5) * 1.8, 0.05 + h1(s + fi * 3.0) * 0.45);
        let d = length(q - at);
        let tw = 0.8 + 0.2 * sin(t * (1.0 + fi * 0.3) + fi);
        c = c + p.d * tw * (smoothstep(0.03, 0.0, d) * 1.5 + exp(-d * 18.0) * 0.5);
        c = c + p.c * 0.3 * smoothstep(0.012, 0.0, abs(d - 0.06 - 0.01 * sin(t + fi)));
    }
    // The village: roofs and a church, lit windows.
    let ground = -0.28 + 0.06 * f3(vec2<f32>(q.x * 3.0, 0.0), s);
    if (q.y < ground) {
        c = p.a * 0.6;
        let cell = floor(q.x * 14.0);
        let roof = ground - 0.03 - 0.06 * h1(cell + s);
        if (q.y < roof && fract(q.x * 14.0) > 0.15) {
            c = mix(p.a, p.b, 0.25);
            let win = h2(floor(q * vec2<f32>(56.0, 40.0)) + s);
            if (win.x > 0.8) {
                c = mix(c, p.d, 0.8 * (0.85 + 0.15 * sin(t * 3.0 + win.y * 20.0)));
            }
        }
    }
    return c;
}

// Cells of a jittered grid: distance to the nearest centre and to the second, and the nearest cell's id.
fn voronoi(z: vec2<f32>, s: f32, t: f32, jiggle: f32) -> vec4<f32> {
    let i = floor(z);
    let f = fract(z);
    var d1 = 9.0;
    var d2 = 9.0;
    var id = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let g = vec2<f32>(f32(x), f32(y));
            let o = h2(i + g + s);
            let pnt = g + 0.5 + 0.4 * sin(o * 6.2831 + t * jiggle);
            let d = length(pnt - f);
            if (d < d1) {
                d2 = d1;
                d1 = d;
                id = i + g;
            } else if (d < d2) {
                d2 = d;
            }
        }
    }
    return vec4<f32>(d1, d2, id);
}

// 6. Stained glass: coloured cells in lead, the light behind them moving.
fn st_glass(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let v = voronoi(q * 6.0, s, t, 0.0);
    let k = h1(dot(v.zw, vec2<f32>(1.3, 7.1)) + s);
    var col = ramp(p, 0.25 + k * 0.75);
    let light = 0.65 + 0.5 * f3(q * 2.0 + vec2<f32>(t * 0.04, t * 0.02), s);
    col = col * light * (0.85 + 0.3 * n2(q * 40.0, s));
    let lead = smoothstep(0.035, 0.07, v.y - v.x);
    // A rose in the middle.
    let r = length(q);
    let rose = smoothstep(0.012, 0.0, abs(r - 0.3)) + smoothstep(0.012, 0.0, abs(r - 0.18));
    return mix(vec3<f32>(0.015, 0.012, 0.01), col * 1.25, lead * (1.0 - rose));
}

// 7. Rings: drops falling into still water, rings spreading and crossing.
fn st_rings(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    var h = 0.0;
    for (var i = 0; i < 5; i = i + 1) {
        let fi = f32(i);
        let period = 6.0 + h1(s + fi) * 5.0;
        let age = fract((t + h1(s + fi * 9.0) * period) / period) * period;
        let at = vec2<f32>((h1(s + fi * 3.0) - 0.5) * 0.9, (h1(s + fi * 5.0) - 0.5) * 0.8);
        let d = length(q - at);
        let front = age * 0.12;
        h = h + sin((d - front) * 70.0) * exp(-abs(d - front) * 18.0) * exp(-age * 0.25);
    }
    let base = mix(p.a, p.b, 0.5 + 0.5 * q.y + 0.2 * f3(q * 3.0, s));
    return base + p.d * max(h, 0.0) * 0.45 + p.c * max(-h, 0.0) * 0.15;
}

// 8. Portrait: a face in profile, half in shadow, light drifting across it; now and then it closes its eyes.
fn st_portrait(q: vec2<f32>, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    var z = q * 1.15 + vec2<f32>(0.04, 0.08);
    if (h1(s) > 0.5) {
        z.x = -z.x;
    }
    // A soft studio backdrop, lighter where the lamp is.
    let lamp = vec2<f32>(0.35 + 0.1 * sin(t * 0.11 + s), 0.3);
    var bg = mix(p.a, p.b, 0.35 + 0.5 * exp(-length(q - lamp) * 2.2));
    bg = bg * (0.9 + 0.2 * f3(q * 5.0, s));
    // The profile, facing +x: skull, brow, nose, lips, chin, jaw, neck, shoulders (a smooth union of simple shapes).
    let skull = length((z - vec2<f32>(-0.04, 0.12)) * vec2<f32>(1.0, 0.92)) - 0.2;
    let face = length((z - vec2<f32>(0.07, 0.03)) * vec2<f32>(1.25, 0.9)) - 0.13;
    let nose = length((z - vec2<f32>(0.19, 0.06)) * vec2<f32>(1.0, 1.6)) - 0.035;
    let lips = length((z - vec2<f32>(0.165, -0.035)) * vec2<f32>(1.6, 1.0)) - 0.03;
    let chin = length(z - vec2<f32>(0.12, -0.09)) - 0.05;
    let neck = max(abs(z.x + 0.02) - 0.075, abs(z.y + 0.22) - 0.16);
    let shoulders = length((z - vec2<f32>(-0.05, -0.62)) * vec2<f32>(0.62, 1.25)) - 0.3;
    let k = 0.035;
    var d = skull;
    for (var i = 0; i < 6; i = i + 1) {
        var other = face;
        if (i == 1) { other = nose; }
        if (i == 2) { other = lips; }
        if (i == 3) { other = chin; }
        if (i == 4) { other = neck; }
        if (i == 5) { other = shoulders; }
        let hh = clamp(0.5 + 0.5 * (other - d) / k, 0.0, 1.0);
        d = mix(other, d, hh) - k * hh * (1.0 - hh);
    }
    let inside = smoothstep(px * 2.0, -px * 2.0, d);
    // The light comes from in front of the face and drifts; the back of the head stays in shadow.
    let lit = smoothstep(-0.12, 0.2, z.x + 0.06 * sin(t * 0.13 + s));
    var skin = mix(p.a * 0.9, mix(p.c, p.d, 0.4), lit);
    skin = skin * (0.9 + 0.12 * f3(z * 9.0, s + 3.0));
    // Hair over the skull.
    let hair = length((z - vec2<f32>(-0.07, 0.15)) * vec2<f32>(1.0, 1.05)) - 0.19;
    let strands = n2(vec2<f32>(z.x * 40.0 + z.y * 10.0, z.y * 8.0), s);
    skin = mix(skin, mix(p.a, p.b, 0.3 + strands * 0.5), smoothstep(px * 2.0, -px * 2.0, max(hair, -(0.05 - z.x))) * step(0.02, z.y));
    // The eye: closed for a moment every few seconds.
    let blink = smoothstep(0.0, 0.08, abs(fract(t / (5.0 + h1(s) * 4.0)) - 0.5) - 0.44);
    let eye = length((z - vec2<f32>(0.12, 0.1)) * vec2<f32>(1.3, 3.2 / max(blink, 0.1))) - 0.016;
    skin = mix(skin, p.a * 0.25, smoothstep(px * 2.0, -px * 2.0, eye));
    // A rim of light along the edge of the profile.
    let rim = smoothstep(0.02, 0.0, abs(d)) * lit;
    return mix(bg, skin, inside) + p.d * rim * 0.35;
}

// 9. Bauhaus: circles, half-moons, bars and triangles, wandering slowly about the whole canvas.
fn st_bauhaus(q: vec2<f32>, aspect: f32, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    var c = mix(p.d, vec3<f32>(0.93, 0.9, 0.82), 0.45) * (0.95 + 0.05 * n2(q * 60.0, s));
    // A big quiet shape behind everything.
    let big = length(q - vec2<f32>((h1(s + 9.0) - 0.5) * aspect * 0.5, (h1(s + 8.0) - 0.5) * 0.4)) - 0.32;
    c = mix(c, mix(p.c, c, 0.35), smoothstep(px * 1.5, -px * 1.5, big));
    for (var i = 0; i < 12; i = i + 1) {
        let fi = f32(i);
        let k = h1(s + fi * 1.7);
        let drift = vec2<f32>(sin(t * 0.07 + fi * 2.0), cos(t * 0.05 + fi)) * 0.03;
        let at = vec2<f32>((h1(s + fi * 2.3) - 0.5) * aspect * 0.9, (h1(s + fi * 3.9) - 0.5) * 0.85) + drift;
        let size = 0.05 + 0.14 * h1(s + fi * 4.1);
        let z = rot(fi + t * 0.02 * (h1(s + fi) - 0.5)) * (q - at);
        var d: f32;
        if (k < 0.3) {
            d = length(z) - size;
        } else if (k < 0.5) {
            d = max(length(z) - size, -z.y);
        } else if (k < 0.75) {
            d = max(abs(z.x) - size * 1.8, abs(z.y) - size * 0.18);
        } else {
            d = max(-z.y - size * 0.6, z.y * 0.55 + abs(z.x) - size * 0.7);
        }
        let col = select(select(p.a, p.b, k > 0.4), p.c, k > 0.72);
        c = mix(c, col, smoothstep(px * 1.5, -px * 1.5, d));
    }
    // A few thin lines across.
    for (var j = 0; j < 3; j = j + 1) {
        let fj = f32(j);
        let a = h1(s + fj * 13.0) * 3.14159;
        let off = (h1(s + fj * 17.0) - 0.5) * 0.6;
        let line = abs(dot(q, vec2<f32>(cos(a), sin(a))) - off);
        c = mix(c, p.a, smoothstep(0.004 + px, 0.004 - px, line));
    }
    return c;
}

// 10. Shatter: a pane of something broken, its pieces trembling.
fn st_shatter(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let r = length(q - vec2<f32>((h1(s) - 0.5) * 0.4, (h1(s + 1.0) - 0.5) * 0.4));
    // Smaller shards near the point of impact.
    let v = voronoi(q * (4.0 + 8.0 * exp(-r * 3.0)), s, t, 0.0);
    let k = h1(dot(v.zw, vec2<f32>(3.1, 1.7)) + s);
    let tremble = sin(t * 9.0 + k * 40.0) * 0.5 + 0.5;
    var col = ramp(p, 0.15 + 0.85 * k);
    col = col * (0.75 + 0.35 * tremble * smoothstep(0.6, 0.0, r));
    let crack = smoothstep(0.05, 0.0, v.y - v.x);
    col = mix(col, p.d * 1.2, crack * 0.85);
    return col + p.c * exp(-r * 6.0) * 0.3;
}

// 11. Ember: sparks rising out of a fire that never burns down.
fn st_ember(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let y = q.y + 0.5;
    let heat = f3(vec2<f32>(q.x * 3.0, y * 2.0 - t * 0.6), s) * smoothstep(0.7, 0.0, y);
    var c = mix(p.a * 0.5, p.b, heat * 1.3);
    c = c + p.c * pow(heat, 3.0) * 1.5;
    // Sparks: cells that rise, each with its own glow.
    for (var layer = 0; layer < 2; layer = layer + 1) {
        let fl = f32(layer);
        let sc = 14.0 + fl * 10.0;
        let z = vec2<f32>(q.x * sc, (y - t * (0.08 + fl * 0.04)) * sc);
        let cell = floor(z);
        let o = h2(cell + s + fl);
        let spark = length(fract(z) - o) ;
        let alive = step(0.75, h1(cell.x * 3.1 + cell.y + s));
        c = c + mix(p.c, p.d, o.y) * alive * smoothstep(0.12, 0.0, spark) * (1.0 - y) * 1.6;
    }
    return c;
}

// 12. Forest: trunks in fog going back for ever, fireflies; sometimes something looks back.
fn st_forest(q: vec2<f32>, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    var c = mix(p.b, p.c, smoothstep(-0.3, 0.6, q.y)) * 0.8;
    for (var i = 0; i < 4; i = i + 1) {
        let fi = 3.0 - f32(i);
        let sc = 6.0 + fi * 6.0;
        let x = q.x * sc + s + fi * 13.0;
        let cell = floor(x);
        let w = 0.15 + 0.2 * h1(cell + fi);
        let trunk = abs(fract(x) - 0.5) - w * 0.5;
        let present = step(0.35, h1(cell * 1.7 + fi + s));
        let fog = fi / 3.0;
        let col = mix(p.a, c, fog * 0.75);
        c = mix(c, col, present * smoothstep(px * sc, -px * sc, trunk));
    }
    // Fireflies.
    for (var i = 0; i < 8; i = i + 1) {
        let fi = f32(i);
        let at = vec2<f32>(sin(t * 0.2 + fi * 2.7 + s) * 0.45, sin(t * 0.13 + fi * 1.3) * 0.25 - 0.1);
        c = c + p.d * exp(-length(q - at) * 120.0) * (0.6 + 0.4 * sin(t * 3.0 + fi));
    }
    // Two eyes, open only some of the time.
    let open = smoothstep(0.7, 0.9, sin(t * 0.21 + s));
    let eyes = vec2<f32>((h1(s + 6.0) - 0.5) * 0.6, -0.05);
    let e = min(length(q - eyes - vec2<f32>(0.025, 0.0)), length(q - eyes + vec2<f32>(0.025, 0.0)));
    c = c + p.d * open * smoothstep(0.009, 0.0, e) * 1.5;
    return c;
}

// 13. Window: a lit window in the dark, rain running down the glass in front of it.
fn st_window(q: vec2<f32>, aspect: f32, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    var c = p.a * (0.6 + 0.3 * f3(q * 4.0, s));
    let n = 1 + i32(h1(s) * 3.0);
    for (var i = 0; i < 3; i = i + 1) {
        if (i >= n) {
            break;
        }
        let fi = f32(i);
        let at = vec2<f32>((h1(s + fi * 4.0) - 0.5) * aspect * 0.6, (h1(s + fi * 2.0) - 0.5) * 0.4);
        let sz = vec2<f32>(0.07 + 0.06 * h1(s + fi), 0.1 + 0.06 * h1(s + fi + 9.0));
        let d = max(abs(q.x - at.x) - sz.x, abs(q.y - at.y) - sz.y);
        let flick = 0.92 + 0.08 * sin(t * 1.3 + fi * 4.0);
        let inside = smoothstep(px * 2.0, -px * 2.0, d);
        var lit = mix(p.c, p.d, 0.5 + 0.5 * (q.y - at.y) / sz.y) * flick;
        // Mullions.
        let mull = min(abs(q.x - at.x), abs(q.y - at.y));
        lit = mix(lit, p.a, smoothstep(0.006, 0.002, mull));
        // A figure, sometimes, standing at the glass.
        if (h1(s + fi * 11.0) > 0.55) {
            let fig = length((q - at - vec2<f32>(sz.x * 0.3, -sz.y * 0.4)) * vec2<f32>(1.0, 0.45)) - sz.x * 0.35;
            lit = mix(lit, p.a * 0.5, smoothstep(px * 2.0, -px * 2.0, fig));
        }
        c = mix(c, lit, inside);
        c = c + p.c * exp(-max(d, 0.0) * 25.0) * 0.25 * flick;
    }
    // Rain on the glass: streaks running down.
    let col_x = floor(q.x * 70.0);
    let speed = 0.2 + h1(col_x + s) * 0.3;
    let drop = fract(q.y * 2.0 + t * speed + h1(col_x * 3.0));
    let streak = smoothstep(0.02, 0.0, abs(fract(q.x * 70.0) - 0.5) - 0.06) * smoothstep(0.0, 0.3, drop) * smoothstep(1.0, 0.9, drop);
    return c + p.d * streak * 0.18 * step(0.6, h1(col_x * 7.0 + s));
}

// 14. Mosaic: tesserae round a great motif, gold ones catching the light.
fn st_mosaic(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let sc = 40.0;
    let z = q * sc;
    let cell = floor(z);
    let f = fract(z) - 0.5;
    let o = h2(cell + s);
    // The motif: rays from a sun, or waves, or rings.
    let c0 = (cell + 0.5) / sc;
    let r = length(c0);
    let a = atan2(c0.y, c0.x);
    var m: f32;
    let kind = h1(s);
    if (kind < 0.33) {
        m = 0.5 + 0.5 * sin(a * (6.0 + floor(h1(s + 1.0) * 6.0)) + r * 8.0);
    } else if (kind < 0.66) {
        m = 0.5 + 0.5 * sin(c0.y * 20.0 + sin(c0.x * 8.0 + s) * 2.0);
    } else {
        m = 0.5 + 0.5 * sin(r * 30.0 - a);
    }
    var col = ramp(p, m * 0.85 + o.x * 0.15);
    let gold = step(0.93, o.y);
    col = mix(col, vec3<f32>(0.95, 0.72, 0.3) * (0.8 + 0.6 * pow(0.5 + 0.5 * sin(t * 0.8 + o.x * 20.0), 8.0)), gold);
    let grout = smoothstep(0.38, 0.47, max(abs(f.x), abs(f.y)));
    return mix(col, p.a * 0.4, grout);
}

// 15. Aurora: curtains of light folding over a snowfield, their glow on the snow.
fn st_aurora(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let hz = -0.22;
    var c = mix(p.a * 0.6, p.a + p.b * 0.25, smoothstep(0.5, hz, q.y));
    c = c + vec3<f32>(1.0) * stars(q, 70.0, 0.05, s, t) * smoothstep(hz, hz + 0.3, q.y);
    var glow = vec3<f32>(0.0);
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        // The curtain's foot: a slow wave across the sky.
        let foot = 0.02 + fi * 0.08 + 0.07 * sin(q.x * (1.8 + fi * 0.7) + t * (0.08 + fi * 0.03) + s + fi * 2.0) + 0.03 * sin(q.x * 5.0 + t * 0.2 + fi);
        let above = q.y - foot;
        // Light rises from the foot and fades upward, in soft vertical folds.
        let rise = exp(-max(above, 0.0) * (5.0 + fi * 2.0)) * smoothstep(-0.015, 0.01, above);
        let folds = 0.55 + 0.45 * sin(q.x * (22.0 + fi * 9.0) + 3.0 * n2(vec2<f32>(q.x * 3.0, t * 0.15 + fi), s));
        let col = mix(p.c, p.d, clamp(above * 3.0, 0.0, 1.0) + fi * 0.2);
        glow = glow + col * rise * folds * (0.75 - fi * 0.15);
    }
    c = c + glow * smoothstep(hz, hz + 0.05, q.y);
    if (q.y < hz) {
        // Snow, blue in the shadows, lit by the aurora.
        let snow = mix(p.b * 0.5 + vec3<f32>(0.25), vec3<f32>(0.85, 0.88, 0.95), 0.5 + 0.3 * n2(q * vec2<f32>(4.0, 20.0), s));
        c = snow * 0.55 + glow * 0.35;
    }
    return c;
}

// 16. Dots: a sunny picture made only of dots, the dots swelling and shrinking.
fn st_dots(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let sc = 30.0;
    let z = q * sc;
    let cell = floor(z);
    let o = h2(cell + s);
    let at = (cell + 0.5) / sc;
    // The picture under the dots: a sky, a sun, hills, a field.
    let sun = exp(-length(at - vec2<f32>(0.18, 0.18)) * 7.0);
    let hill = smoothstep(0.02, -0.02, at.y + 0.05 - 0.08 * sin(at.x * 5.0 + s));
    var pic = mix(0.75 - at.y * 0.4, 0.35 + 0.2 * f3(at * 4.0, s), hill);
    pic = pic + sun * 0.5;
    let col = ramp(p, pic + (o.x - 0.5) * 0.25);
    let r = 0.38 + 0.08 * sin(t * 0.6 + o.y * 6.28);
    let d = length(fract(z) - 0.5 - (o - 0.5) * 0.25);
    let paper = mix(p.d, vec3<f32>(0.96, 0.94, 0.88), 0.7);
    return mix(paper, col, smoothstep(r, r - 0.06, d));
}

// 17. Galaxy: an island of stars turning.
fn st_galaxy(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let z = rot(t * 0.015 + s) * (q * vec2<f32>(1.0, 1.3));
    let r = length(z);
    let a = atan2(z.y, z.x);
    let arms = 2.0 + floor(h1(s) * 3.0);
    let spiral = 0.5 + 0.5 * cos(arms * (a - log(r + 0.001) * 2.2));
    let dust = f3(z * 6.0, s);
    var c = p.a * 0.5;
    let density = spiral * exp(-r * 3.5) * (0.6 + 0.8 * dust);
    c = c + ramp(p, 0.4 + density) * density * 1.6;
    c = c + p.d * exp(-r * 18.0) * 1.2;
    c = c + vec3<f32>(1.0) * stars(q, 80.0, 0.07, s, t);
    return c;
}

// 18. Rain: a city's lights blurred behind wet glass.
fn st_rain(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    var c = mix(p.a, p.b, smoothstep(-0.5, 0.6, q.y) * 0.5);
    for (var i = 0; i < 16; i = i + 1) {
        let fi = f32(i);
        let at = vec2<f32>((h1(s + fi * 1.3) - 0.5) * 1.6, (h1(s + fi * 2.9) - 0.5) * 0.9);
        let r = 0.04 + 0.07 * h1(s + fi * 5.1);
        let d = length(q - at);
        let col = select(p.c, p.d, h1(fi + s) > 0.5);
        c = c + col * smoothstep(r, r * 0.8, d) * 0.35 * (0.8 + 0.2 * sin(t * 0.5 + fi));
    }
    // Drops sliding down, leaving trails.
    for (var k = 0; k < 2; k = k + 1) {
        let fk = f32(k);
        let sc = 18.0 + fk * 14.0;
        let x = q.x * sc;
        let lane = floor(x);
        let sp = 0.05 + 0.1 * h1(lane + s + fk);
        let y = fract(q.y * 1.3 + t * sp + h1(lane * 2.0));
        let dx = fract(x) - 0.5;
        let head = length(vec2<f32>(dx, (y - 0.5) * 3.0)) - 0.12;
        let trail = smoothstep(0.08, 0.0, abs(dx)) * smoothstep(0.5, 1.0, y) * 0.3;
        c = c + p.d * (smoothstep(0.02, -0.02, head) * 0.5 + trail * 0.25) * step(0.4, h1(lane * 3.7 + fk));
    }
    return c;
}

// 19. Garden: flowers swaying in a summer breeze.
fn st_garden(q: vec2<f32>, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    var c = mix(p.b * 0.7, p.c * 0.6, smoothstep(-0.5, 0.5, q.y));
    c = c * (0.85 + 0.2 * f3(q * 4.0, s));
    for (var i = 0; i < 9; i = i + 1) {
        let fi = f32(i);
        let base = vec2<f32>((h1(s + fi * 2.1) - 0.5) * 1.3, -0.5);
        let hgt = 0.35 + 0.45 * h1(s + fi * 3.3);
        let sway = sin(t * 0.8 + fi * 1.7) * 0.025;
        let head = base + vec2<f32>(sway, hgt);
        // The stem.
        let along = clamp((q.y - base.y) / hgt, 0.0, 1.0);
        let stem_x = base.x + sway * along * along;
        c = mix(c, p.a, smoothstep(0.006 + px, 0.006 - px, abs(q.x - stem_x)) * step(q.y, head.y) * step(base.y, q.y));
        // The bloom: a rose curve.
        let z = q - head;
        let r = length(z);
        let a = atan2(z.y, z.x) + t * 0.1 * (h1(fi) - 0.5);
        let k = 4.0 + floor(h1(s + fi) * 4.0);
        let size = 0.05 + 0.04 * h1(s + fi * 7.0);
        let petal = size * (0.55 + 0.45 * abs(cos(k * 0.5 * a)));
        let col = select(p.d, p.c, h1(s + fi * 9.0) > 0.5);
        c = mix(c, col, smoothstep(px * 2.0, -px * 2.0, r - petal));
        c = mix(c, p.a * 0.8, smoothstep(px * 2.0, -px * 2.0, r - size * 0.22));
    }
    return c;
}

// 20. Maze: a labyrinth with a light that walks it.
fn st_maze(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let sc = 12.0;
    let z = q * sc;
    let cell = floor(z);
    let f = fract(z) - 0.5;
    let flip = step(0.5, h1(dot(cell, vec2<f32>(1.0, 57.0)) + s));
    let g = vec2<f32>(select(f.x, -f.x, flip > 0.5), f.y);
    // Truchet arcs: two quarter circles per cell.
    let d = min(abs(length(g - vec2<f32>(0.5, 0.5)) - 0.5), abs(length(g + vec2<f32>(0.5, 0.5)) - 0.5));
    var c = mix(p.a, p.b, 0.4 + 0.2 * f3(q * 3.0, s));
    let wall = smoothstep(0.1, 0.06, d);
    c = mix(c, p.c, wall);
    // The wanderer.
    let w = vec2<f32>(sin(t * 0.11 + s), sin(t * 0.07 + s * 2.0)) * vec2<f32>(0.4, 0.35);
    c = c + p.d * exp(-length(q - w) * 25.0) * 1.2;
    return c;
}

// 21. Eye: an eye that follows whoever stands before it.
fn st_eye(q: vec2<f32>, s: f32, p: Pal, t: f32, look: vec2<f32>, px: f32) -> vec3<f32> {
    var c = mix(p.a, p.b, 0.3 + 0.3 * f3(q * 3.0, s));
    let blink_phase = fract(t / (6.0 + h1(s) * 5.0));
    let open = smoothstep(0.0, 0.03, abs(blink_phase - 0.5) - 0.02);
    // The almond of the eye.
    let lid = 0.18 * open + 0.002;
    let almond = abs(q.y) - lid * (1.0 - pow(abs(q.x) / 0.42, 2.0));
    let inside = smoothstep(px * 2.0, -px * 2.0, max(almond, abs(q.x) - 0.42));
    var e = mix(p.d, vec3<f32>(0.95, 0.93, 0.88), 0.6);
    let iris_at = clamp(look, vec2<f32>(-0.18, -0.07), vec2<f32>(0.18, 0.07));
    let z = q - iris_at;
    let r = length(z);
    let a = atan2(z.y, z.x);
    let fibres = 0.7 + 0.3 * n2(vec2<f32>(a * 8.0, r * 30.0), s);
    let iris = mix(p.b, p.c, smoothstep(0.13, 0.03, r)) * fibres;
    e = mix(e, iris, smoothstep(0.135, 0.125, r));
    e = mix(e, vec3<f32>(0.01), smoothstep(0.05, 0.04, r));
    e = e + vec3<f32>(1.0) * smoothstep(0.018, 0.01, length(z - vec2<f32>(0.035, 0.04)));
    c = mix(c, e, inside);
    // Lashes.
    c = mix(c, p.a * 0.4, smoothstep(0.012, 0.0, abs(almond)) * step(0.0, q.y) * step(abs(q.x), 0.42));
    return c;
}

// 22. Orbs: two lights drifting toward each other and never touching.
fn st_orbs(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    var c = mix(p.a, p.b * 0.6, smoothstep(-0.6, 0.6, q.y));
    c = c * (0.8 + 0.3 * f3(q * 2.0 + vec2<f32>(t * 0.01, 0.0), s));
    let near = 0.1 + 0.08 * (0.5 + 0.5 * sin(t * 0.17 + s));
    let tilt = rot(sin(t * 0.05 + s) * 0.4);
    let a = tilt * vec2<f32>(-near, 0.0) + vec2<f32>(0.0, 0.03 * sin(t * 0.3));
    let b = tilt * vec2<f32>(near, 0.0) + vec2<f32>(0.0, -0.03 * sin(t * 0.3 + 1.0));
    let da = length(q - a);
    let db = length(q - b);
    c = c + p.c * (exp(-da * 30.0) * 1.4 + exp(-da * 6.0) * 0.4);
    c = c + p.d * (exp(-db * 30.0) * 1.4 + exp(-db * 6.0) * 0.4);
    // Where their light meets, a faint bridge.
    let mid = (a + b) * 0.5;
    c = c + mix(p.c, p.d, 0.5) * exp(-length((q - mid) * vec2<f32>(1.0 / max(near, 0.05), 6.0))) * 0.25;
    return c;
}

// 23. Weave: threads going over and under, a pattern in the cloth.
fn st_weave(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let sc = 90.0;
    let z = q * sc;
    let cell = floor(z);
    let f = fract(z);
    let over = (i32(cell.x) + i32(cell.y)) % 2 == 0;
    let along = select(f.y, f.x, over);
    let thread = 0.6 + 0.4 * sin(along * 3.14159);
    // The pattern: diamonds and bands.
    let d = abs(fract(q.x * 3.0 + s) - 0.5) + abs(fract(q.y * 4.0) - 0.5);
    let band = step(0.5, fract(q.y * 6.0 + 0.25 * sin(q.x * 4.0 + s)));
    let k = select(0.3 + 0.2 * band, 0.85, d < 0.25) + 0.1 * h1(cell.y + s);
    var col = ramp(p, k);
    col = col * thread * (0.92 + 0.08 * sin(t * 0.5 + q.x * 3.0));
    return col;
}

// 24. Storm: clouds churning, lightning now and then.
fn st_storm(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    let cl = f2(vec2<f32>(q.x * 2.0 + t * 0.03, q.y * 3.0 - t * 0.01), s);
    var c = mix(p.a, p.b, cl);
    c = mix(c, p.c * 0.6, smoothstep(0.5, 0.8, f3(q * 5.0 + vec2<f32>(t * 0.05, 0.0), s + 3.0)) * 0.5);
    // A flash every few seconds, and a bolt with it.
    let period = 4.0 + h1(s) * 3.0;
    let k = floor(t / period);
    let ph = fract(t / period);
    let flash = exp(-ph * 30.0) + 0.5 * exp(-abs(ph - 0.08) * 40.0);
    let bx = (h1(k + s) - 0.5) * 0.8;
    let jag = (n2(vec2<f32>(q.y * 14.0, k), s) - 0.5) * 0.12 + (n2(vec2<f32>(q.y * 40.0, k + 3.0), s) - 0.5) * 0.03;
    let bolt = smoothstep(0.006, 0.0, abs(q.x - bx - jag)) * step(-0.4, q.y);
    c = c + p.d * flash * (0.5 * cl + bolt * 2.0);
    // Rain hatching.
    let rain = step(0.97, fract((q.x * 0.3 + q.y) * 60.0 + t * 3.0 + n2(q * 10.0, s)));
    return c + p.d * rain * 0.08;
}

// 25. Bloom: one flower, opening and closing as slowly as breathing.
fn st_bloom(q: vec2<f32>, s: f32, p: Pal, t: f32, px: f32) -> vec3<f32> {
    var c = mix(p.a, p.b, 0.3 + 0.4 * f3(q * 2.5, s));
    let open = 0.65 + 0.35 * sin(t * 0.18 + s);
    let r = length(q);
    let a = atan2(q.y, q.x);
    let k = 5.0 + floor(h1(s) * 4.0);
    for (var layer = 0; layer < 3; layer = layer + 1) {
        let fl = f32(layer);
        let rr = (0.38 - fl * 0.1) * open;
        let petal = rr * (0.45 + 0.55 * pow(abs(cos(k * 0.5 * (a + fl * 0.4 + t * 0.01))), 0.7));
        let col = ramp(p, 0.5 + fl * 0.22) * (0.75 + 0.25 * smoothstep(0.0, petal, r));
        c = mix(c, col, smoothstep(px * 2.0, -px * 2.0, r - petal));
    }
    c = c + p.d * exp(-r * 22.0) * 1.3;
    return c;
}

// 26. Void: darkness, and one small flame keeping it company.
fn st_void(q: vec2<f32>, s: f32, p: Pal, t: f32) -> vec3<f32> {
    var c = p.a * (0.4 + 0.25 * f3(q * 3.0, s));
    let at = vec2<f32>((h1(s) - 0.5) * 0.3, -0.1 + (h1(s + 1.0) - 0.5) * 0.2);
    let flick = 0.85 + 0.1 * sin(t * 7.0 + s) + 0.05 * sin(t * 13.0);
    let z = (q - at) * vec2<f32>(1.0, 0.55);
    let flame = length(z + vec2<f32>(sin(t * 3.0 + q.y * 20.0) * 0.004, 0.0)) - 0.018 * flick;
    c = c + mix(p.c, vec3<f32>(1.0, 0.85, 0.5), 0.5) * exp(-length(q - at) * 7.0) * 0.5 * flick;
    c = c + vec3<f32>(1.0, 0.8, 0.45) * smoothstep(0.01, -0.005, flame) * 2.0;
    // The candle under it.
    let wax = max(abs(q.x - at.x) - 0.012, abs(q.y - at.y + 0.07) - 0.05);
    c = mix(c, mix(p.d, vec3<f32>(0.9, 0.85, 0.7), 0.5) * 0.6 * flick, smoothstep(0.003, -0.003, wax));
    return c;
}

// The picture of style `style` at `uv` (0..1 over the canvas) for a canvas `aspect` wide; `look` is where the viewer is, in the canvas's own units
// (the eye follows it).
fn art(style: i32, uv: vec2<f32>, aspect: f32, seed: f32, p: Pal, t: f32, px: f32, look: vec2<f32>) -> vec3<f32> {
    let q = vec2<f32>((uv.x - 0.5) * aspect, uv.y - 0.5);
    switch style {
        case 1: { return st_ink(q, seed, p, t); }
        case 2: { return st_field(q, aspect, seed, p, t); }
        case 3: { return st_horizon(q, seed, p, t, px); }
        case 4: { return st_tide(q, seed, p, t, px); }
        case 5: { return st_starry(q, seed, p, t); }
        case 6: { return st_glass(q, seed, p, t); }
        case 7: { return st_rings(q, seed, p, t); }
        case 8: { return st_portrait(q, seed, p, t, px); }
        case 9: { return st_bauhaus(q, aspect, seed, p, t, px); }
        case 10: { return st_shatter(q, seed, p, t); }
        case 11: { return st_ember(q, seed, p, t); }
        case 12: { return st_forest(q, seed, p, t, px); }
        case 13: { return st_window(q, aspect, seed, p, t, px); }
        case 14: { return st_mosaic(q, seed, p, t); }
        case 15: { return st_aurora(q, seed, p, t); }
        case 16: { return st_dots(q, seed, p, t); }
        case 17: { return st_galaxy(q, seed, p, t); }
        case 18: { return st_rain(q, seed, p, t); }
        case 19: { return st_garden(q, seed, p, t, px); }
        case 20: { return st_maze(q, seed, p, t); }
        case 21: { return st_eye(q, seed, p, t, look, px); }
        case 22: { return st_orbs(q, seed, p, t); }
        case 23: { return st_weave(q, seed, p, t); }
        case 24: { return st_storm(q, seed, p, t); }
        case 25: { return st_bloom(q, seed, p, t, px); }
        case 26: { return st_void(q, seed, p, t); }
        default: { return ramp(p, uv.y); }
    }
}

// ---- The world inside a painting ------------------------------------------------------------------------------------------------------
// Where direction `dir` lands on the picture when one stands inside it: the picture runs across the half of the world ahead, mirrored behind (no seam),
// with the horizon a little below the middle. The sky and the mirror-water underfoot both use it, so the water reflects the sky exactly.
fn world_uv(dir: vec3<f32>) -> vec2<f32> {
    let a = atan2(dir.x, -dir.z);
    let el = asin(clamp(dir.y, -1.0, 1.0));
    return vec2<f32>(0.5 + 0.47 * sin(a), clamp(0.42 + el / 3.14159 * 1.5, -0.4, 1.4));
}

// The world's colour along `dir`.
fn world_color(dir: vec3<f32>, style: i32, seed: f32, pal: Pal, t: f32, px: f32) -> vec3<f32> {
    let el = asin(clamp(dir.y, -1.0, 1.0));
    var c = art(style, world_uv(dir), 2.2, seed, pal, t, px, vec2<f32>(0.0));
    c = c + pal.d * exp(-abs(el) * 6.0) * 0.12;
    return mix(c, pal.a, smoothstep(0.9, 1.5, el) * 0.5);
}

// ---- Sculpture materials ---------------------------------------------------------------------------------------------------------------
// Each returns colour and glow for a point `p` with normal `n` in the sculpture's space.

fn material(m: i32, p: vec3<f32>, n: vec3<f32>, wn: vec3<f32>, seed: f32, pal: Pal, t: f32, eye: vec3<f32>) -> vec4<f32> {
    let v = normalize(eye);
    let rim = pow(1.0 - abs(dot(wn, v)), 3.0);
    switch m {
        case 1: {
            // Marble: veins through pale stone.
            let w = fbm(p * 3.0 + vec3<f32>(seed));
            let vein = smoothstep(0.03, 0.0, abs(fract(w * 4.0 + p.y * 0.8) - 0.5) - 0.01);
            let base = mix(vec3<f32>(0.92, 0.9, 0.86), pal.d, 0.12);
            return vec4<f32>(mix(base, pal.b * 0.6, vein * 0.55), 0.02);
        }
        case 2: {
            // Bronze with green in the hollows.
            let pat = smoothstep(0.45, 0.75, fbm(p * 6.0 + vec3<f32>(seed)));
            return vec4<f32>(mix(vec3<f32>(0.55, 0.36, 0.18), vec3<f32>(0.25, 0.5, 0.42), pat * 0.6) + rim * 0.2, 0.0);
        }
        case 3: {
            // Glass: pale, lit from inside, coloured at its edges.
            let inner = 0.5 + 0.5 * sin(p.y * 6.0 - t * 0.8 + seed);
            return vec4<f32>(mix(pal.c, pal.d, inner) * 0.8 + rim * pal.d, 0.45 + 0.3 * rim);
        }
        case 4: {
            // Clay, thumb-marked.
            let thumbs = vnoise(p * 18.0 + vec3<f32>(seed));
            return vec4<f32>(mix(vec3<f32>(0.62, 0.4, 0.28), vec3<f32>(0.75, 0.52, 0.38), thumbs), 0.0);
        }
        case 5: {
            // Gilt: gold leaf, worn through to red bole at the edges.
            let worn = smoothstep(0.6, 0.75, fbm(p * 9.0 + vec3<f32>(seed)) + rim * 0.3);
            return vec4<f32>(mix(vec3<f32>(0.95, 0.72, 0.3), vec3<f32>(0.5, 0.18, 0.1), worn * 0.6), 0.08);
        }
        case 6: {
            // Obsidian: black glass with a slow sheen.
            let sheen = pow(0.5 + 0.5 * sin(dot(wn, vec3<f32>(3.0, 2.0, 1.0)) * 4.0 + t * 0.3), 8.0);
            return vec4<f32>(vec3<f32>(0.02, 0.02, 0.03) + pal.c * sheen * 0.3 + rim * 0.15, 0.03);
        }
        case 7: {
            // Porcelain painted with blue swirls.
            let a = atan2(p.z, p.x);
            let sw = sin(a * 5.0 + p.y * 9.0 + 2.0 * sin(p.y * 3.0 + seed));
            let blue = smoothstep(0.75, 0.9, sw);
            return vec4<f32>(mix(vec3<f32>(0.95, 0.95, 0.97), vec3<f32>(0.1, 0.2, 0.55), blue), 0.05);
        }
        case 8: {
            // Opal: colours that move as the viewer does.
            let k = dot(wn, v) * 3.0 + p.y * 2.0 + t * 0.2 + fbm(p * 4.0) * 2.0;
            let c = 0.5 + 0.5 * cos(6.2831 * (vec3<f32>(k) + vec3<f32>(0.0, 0.33, 0.67)));
            return vec4<f32>(mix(vec3<f32>(0.9), c, 0.55), 0.25);
        }
        case 9: {
            // Copper gone to verdigris.
            let g = smoothstep(0.35, 0.65, fbm(p * 5.0 + vec3<f32>(seed)) + n.y * 0.2);
            return vec4<f32>(mix(vec3<f32>(0.7, 0.38, 0.22), vec3<f32>(0.3, 0.62, 0.52), g), 0.0);
        }
        case 10: {
            // Ember glaze: dark, with cracks of fire that breathe.
            let v2 = fbm(p * 7.0 + vec3<f32>(seed));
            let crack = smoothstep(0.03, 0.0, abs(v2 - 0.5));
            let breathe = 0.6 + 0.4 * sin(t * 1.3 + p.y * 2.0);
            return vec4<f32>(mix(vec3<f32>(0.06, 0.04, 0.035), vec3<f32>(1.0, 0.45, 0.12), crack * breathe), crack * breathe * 1.5);
        }
        default: {
            return vec4<f32>(pal.c, 0.0);
        }
    }
}

// ---- The engine's entry point ---------------------------------------------------------------------------------------------------------

fn surface(s: SurfaceIn) -> vec4<f32> {
    let pal = pal_of(s);
    let t = sky_time();
    let seed = s.params.z;
    if (s.params.x < 1.5) {
        // A canvas: the panel's share of the picture, then the style.
        let uv0 = s.p.xy + vec2<f32>(0.5);
        let uv = vec2<f32>(s.palette[0].w + uv0.x * max(s.palette[1].w, 0.01), uv0.y);
        let aspect = max(s.params.w, 0.2);
        // Where the viewer stands, seen from this point: right or left across the canvas, above or below (the eye in style 21 follows it).
        let to_eye = globals.camera_pos.xyz - s.world;
        let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), s.world_n) + vec3<f32>(1e-5, 0.0, 0.0));
        let d = max(length(to_eye), 0.5);
        let look = vec2<f32>(dot(to_eye, right) / d * 0.22, to_eye.y / d * 0.12);
        let px = s.px * 2.0;
        var c = art(i32(s.params.y + 0.5), uv, aspect, seed, pal, t, px, look);
        c = c * grain(uv);
        // The way to a key: a slow golden shimmer sweeping over the picture.
        let glint = s.palette[3].w * pow(0.5 + 0.5 * sin(uv.x * 9.0 + uv.y * 5.0 - t * 1.6), 12.0);
        c = c + vec3<f32>(1.0, 0.78, 0.35) * glint * 0.6;
        let alive = s.palette[2].w;
        return vec4<f32>(c, 0.22 * alive + glint * 0.5);
    }
    if (s.params.x > 2.5) {
        // Still water underfoot inside a painting: it mirrors the world, trembling a little.
        let view = normalize(s.world - globals.camera_pos.xyz);
        let ripple = vec3<f32>(vnoise(vec3<f32>(s.world.xz * 1.3, t * 0.4)) - 0.5, 0.0, vnoise(vec3<f32>(s.world.xz * 1.3 + 7.0, t * 0.4)) - 0.5) * 0.06;
        var r = reflect(view, normalize(vec3<f32>(0.0, 1.0, 0.0) + ripple));
        r.y = abs(r.y);
        let c = world_color(r, i32(s.params.y + 0.5), seed, pal, t, s.px);
        let fres = 0.45 + 0.5 * pow(1.0 - abs(view.y), 3.0);
        return vec4<f32>(mix(pal.a * 0.4, c, fres) * 0.9, 0.85);
    }
    let eye_local = globals.camera_pos.xyz - s.world;
    return material(i32(s.params.y + 0.5), s.p, s.n, s.world_n, seed, pal, t, eye_local);
}
