// Screen effects drawn over the finished frame: a coloured vignette (damage, low health, spawn protection), an arc on a ring around the
// crosshair pointing at whoever hurt us, whole-screen flashes, and the hit / kill marker. Everything is analytic (no textures), so an
// effect costs one fullscreen triangle and animates at the display rate. The output is premultiplied alpha.

struct Fx {
    // xy = viewport size in pixels, z = pixel scale (height / 1080), w unused.
    res: vec4<f32>,
    // rgb, strength 0..1 (the darkening/tint toward the screen edges).
    vignette: vec4<f32>,
    // rgb, alpha 0..1 (a tint over the whole screen).
    flash: vec4<f32>,
    // x = direction of the attacker relative to the view (radians, 0 ahead, positive to the right), y = strength 0..1 (0 = off).
    hurt: vec4<f32>,
    // x = marker age 0..1 (1 or more = not drawn), y = kind (0 hit, 1 kill).
    marker: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Fx;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_fx(@builtin(vertex_index) vi: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: VsOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    return out;
}

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;
const INV_SQRT2: f32 = 0.70710678;

// `src` (straight colour, alpha) composited over the premultiplied accumulation `dst`.
fn over(dst: vec4<f32>, rgb: vec3<f32>, a: f32) -> vec4<f32> {
    let s = clamp(a, 0.0, 1.0);
    return vec4<f32>(rgb * s + dst.rgb * (1.0 - s), s + dst.a * (1.0 - s));
}

@fragment
fn fs_fx(in: VsOut) -> @location(0) vec4<f32> {
    // Pixels from the centre (x right, y down) and the same in units of half the window height (aspect-correct).
    let d = in.pos.xy - u.res.xy * 0.5;
    let n = d / (u.res.y * 0.5);
    let r = length(n);

    var acc = vec4<f32>(0.0);

    // Vignette: nothing in the middle, building toward (and past) the corners.
    let vig = smoothstep(0.5, 1.5, r);
    acc = over(acc, u.vignette.rgb, vig * u.vignette.a);

    // Whole-screen flash.
    acc = over(acc, u.flash.rgb, u.flash.a);

    // Damage arc: a soft wedge on a ring around the crosshair, on the side the attacker is on.
    if (u.hurt.y > 0.0) {
        let ang = atan2(n.x, -n.y); // 0 = up the screen (ahead), positive clockwise (right)
        var da = ang - u.hurt.x;
        da = da - TAU * floor((da + PI) / TAU);
        let wedge = 1.0 - smoothstep(0.18, 0.42, abs(da));
        // A crisp band with a soft halo around it, like the arc of a dial.
        let band = smoothstep(0.42, 0.45, r) * (1.0 - smoothstep(0.50, 0.53, r));
        let halo = exp(-pow((r - 0.475) / 0.13, 2.0));
        acc = over(acc, vec3<f32>(1.0, 0.12, 0.07), wedge * (0.85 * band + 0.32 * halo) * u.hurt.y);
    }

    // Hit marker: four short diagonal ticks around the crosshair, spreading and fading (red and bigger for a kill).
    if (u.marker.x < 1.0) {
        let s = u.res.z;
        let age = u.marker.x;
        let kill = u.marker.y;
        let fade = 1.0 - smoothstep(0.3, 1.0, age);
        let gap = (5.0 + 6.0 * age + 3.0 * kill) * s;
        let len = (8.0 + 6.0 * kill) * s;
        let half = (1.2 + 0.7 * kill) * s;
        let q = abs(d);
        let along = (q.x + q.y) * INV_SQRT2;
        let across = abs(q.x - q.y) * INV_SQRT2;
        let seg = smoothstep(gap - 0.75, gap + 0.75, along) * (1.0 - smoothstep(gap + len - 0.75, gap + len + 0.75, along));
        let thick = 1.0 - smoothstep(half - 0.75, half + 0.75, across);
        let col = mix(vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(1.0, 0.22, 0.16), kill);
        acc = over(acc, col, seg * thick * fade);
    }

    return acc;
}
