// The sky dome, shared by background.wgsl (what you see above the world) and ocean.wgsl (what the water reflects and fades into).
// `gpu.rs` concatenates common.wgsl, then this file, then the shader that uses it.
//
// Globals used: `inv_view_proj` (to turn a pixel into a view direction), `bg_top` = zenith, `bg_bottom` = horizon, `sky.y` = the gradient
// exponent, `sky.z` = 1 when the sky has a sun, `sun_dir` = (direction toward the sun, angular radius in radians), `sun_color` = (rgb, glow).
// With a scene `clock` (`night.w` = 1) it also draws the glow low on the sun's side (`glow`), the stars and the Milky Way (`night.x` = how visible,
// `night.y` = time, `celestial` = the rotation of the star sphere) and the moon (`moon_dir`, `night.z` = its phase).

// The unit direction, in world space, that pixel `ndc` looks along.
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    let far_p = globals.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let near_p = globals.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    return normalize(far_p.xyz / far_p.w - near_p.xyz / near_p.w);
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

// ---- Stars -------------------------------------------------------------------------------------------------------------------------

// One layer of stars on the celestial sphere. The sky is cut into cells (`cells` per radian); a cell holds a star with probability `density`, placed inside
// the cell so it never reaches a neighbour (one lookup per layer). `size` is the star's radius in cells; a star is never drawn smaller than a pixel (`px`
// radians), with its energy kept, so faint stars shimmer instead of crawling as the view moves.
fn star_layer(d: vec3<f32>, cells: f32, seed: f32, density: f32, size: f32, gain: f32, halo: f32, px: f32, tw_time: f32, horizon_tw: f32) -> vec3<f32> {
    let p = d * cells;
    let c = floor(p);
    let rnd = hash33(c + seed);
    if (rnd.x > density) {
        return vec3<f32>(0.0);
    }
    let jitter = hash33(c * 1.37 + seed + 11.0);
    // A star with a halo is kept near the middle of its cell, so its glow (which only exists inside the cell) falls off before the edge.
    let margin = select(size, 0.34, halo > 0.0);
    let center = c + vec3<f32>(0.5) + (jitter - vec3<f32>(0.5)) * (1.0 - 2.0 * margin);
    let dist = length(p - center);
    let foot = px * cells * 0.75;
    let rr = max(size, foot);
    let k = exp(-(dist * dist) / (rr * rr * 0.45)) * ((size * size) / (rr * rr));
    // The brightest stars bloom: a soft halo around the point.
    let halo_t = halo * exp(-(dist * dist) / (2.0 * 0.075 * 0.075));
    // Brightness: many faint, a few bright. Colour: from blue-white through white to amber.
    let b = pow(rnd.y, 3.2) * gain;
    let temp = rnd.z;
    var col = mix(vec3<f32>(0.62, 0.76, 1.0), vec3<f32>(1.0, 0.97, 0.9), smoothstep(0.0, 0.55, temp));
    col = mix(col, vec3<f32>(1.0, 0.74, 0.52), smoothstep(0.78, 1.0, temp));
    let tw = 1.0 + (0.10 + 0.30 * horizon_tw) * sin(tw_time * (2.5 + jitter.x * 6.0) + jitter.y * 40.0);
    return col * b * (k + halo_t) * tw;
}

fn star_field(d: vec3<f32>, px: f32, time: f32) -> vec3<f32> {
    let dc = vec3<f32>(dot(globals.celestial[0].xyz, d), dot(globals.celestial[1].xyz, d), dot(globals.celestial[2].xyz, d));
    let horizon_tw = 1.0 - clamp(d.y * 1.6, 0.0, 1.0);
    // The Milky Way: a band around a tilted great circle, patchy, with dust lanes, brighter and warmer toward the galactic centre.
    let galactic_n = normalize(vec3<f32>(0.34, 0.82, 0.46));
    let centre_dir = normalize(cross(galactic_n, vec3<f32>(0.0, 0.0, 1.0)));
    let lat = dot(dc, galactic_n);
    let band = exp(-(lat * lat) / (2.0 * 0.095 * 0.095));
    let patch_ = fbm(dc * 8.0 + 4.0);
    let dust = fbm(dc * 22.0 + 21.0);
    let core = pow(max(dot(dc, centre_dir), 0.0), 3.0);
    let lanes = 1.0 - 0.55 * smoothstep(0.50, 0.68, dust) * band;
    let grain = 0.7 + 0.6 * vnoise(dc * 90.0);
    let glow = band * (0.25 + 0.75 * smoothstep(0.30, 0.78, patch_)) * lanes * grain;
    var way = mix(vec3<f32>(0.46, 0.58, 1.0), vec3<f32>(1.0, 0.88, 0.72), clamp(core * 1.1, 0.0, 1.0)) * glow * (0.060 + 0.260 * core);
    var s = vec3<f32>(0.0);
    // A few rare, bright stars with halos: the ones the eye goes to first.
    s = s + star_layer(dc, 20.0, 7.0, 0.045, 0.10, 5.0, 0.20, px, time, horizon_tw);
    s = s + star_layer(dc, 38.0, 1.0, 0.070, 0.12, 3.0, 0.07, px, time, horizon_tw);
    s = s + star_layer(dc, 80.0, 2.0, 0.100, 0.13, 1.7, 0.0, px, time, horizon_tw);
    s = s + star_layer(dc, 150.0, 3.0, 0.130, 0.17, 0.85, 0.0, px, time, horizon_tw);
    // Dust: a dense scatter of faint stars concentrated in the band.
    s = s + star_layer(dc, 280.0, 4.0, 0.08 + 0.40 * band, 0.25, 0.55, 0.0, px, time, horizon_tw);
    return s + way;
}

// ---- The moon ----------------------------------------------------------------------------------------------------------------------

fn moon_disc(d: vec3<f32>) -> vec4<f32> {
    let md = globals.moon_dir.xyz;
    let r = globals.moon_dir.w;
    let cosang = clamp(dot(d, md), -1.0, 1.0);
    let ang = acos(cosang);
    let phase = globals.night.z;
    let illum = 0.5 - 0.5 * cos(6.2831853 * phase);
    // A soft halo around the moon, brighter with the lit fraction.
    var rgb = vec3<f32>(0.70, 0.78, 1.0) * (exp(-ang / (r * 3.2)) * 0.20 + exp(-ang / (r * 14.0)) * 0.055) * (0.25 + 0.75 * illum);
    var alpha = 0.0;
    if (ang < r * 1.08) {
        let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), md));
        let upv = cross(md, right);
        let p = vec2<f32>(dot(d, right), dot(d, upv)) / sin(r);
        let r2 = dot(p, p);
        let z = sqrt(max(0.0, 1.0 - r2));
        let n = vec3<f32>(p.x, p.y, z);
        // The sun lights the moon from a direction that swings round it over the month: behind the viewer at full moon, behind the moon at new.
        let th = (phase - 0.5) * 6.2831853;
        let l = vec3<f32>(sin(th), 0.0, cos(th));
        let lit = smoothstep(-0.03, 0.10, dot(n, l));
        // Maria and craters from noise on the sphere; limb darkening.
        let maria = fbm(n * 2.4 + 5.0);
        let tone = 0.62 + 0.38 * smoothstep(0.36, 0.64, maria) + 0.16 * (vnoise(n * 15.0) - 0.5) + 0.10 * (vnoise(n * 40.0) - 0.5);
        let limb = 0.55 + 0.45 * z;
        let earthshine = vec3<f32>(0.020, 0.028, 0.050);
        let base = vec3<f32>(1.0, 0.97, 0.90) * 0.98 * tone * limb;
        let disc = base * lit + earthshine * (1.0 - lit);
        let edge = 1.0 - smoothstep(0.96, 1.0, sqrt(r2));
        rgb = mix(rgb, disc, edge);
        alpha = edge;
    }
    return vec4<f32>(rgb, alpha);
}

// ---- The sky -----------------------------------------------------------------------------------------------------------------------

// The sky colour along `dir`: horizon at eye level rising to zenith overhead, the sun's halo and disc, and with a clock the sunset glow, stars and moon.
// `px` is the angular size of a pixel (radians), so stars can be kept at least a pixel wide. Below the horizon it is the horizon colour (what distant
// water and haze fade into), so nothing ever shows a dark seam.
fn sky_color_px(dir: vec3<f32>, px: f32) -> vec3<f32> {
    let up = clamp(dir.y, 0.0, 1.0);
    var c = mix(globals.bg_bottom.rgb, globals.bg_top.rgb, pow(up, globals.sky.y));
    let clocked = globals.night.w > 0.5;
    if (clocked) {
        c = c + horizon_glow(dir);
    }
    if (globals.sky.z > 0.5) {
        let s = clamp(dot(dir, globals.sun_dir.xyz), -1.0, 1.0);
        let ang = acos(s);
        let r = globals.sun_dir.w;
        // The sun sinks behind the horizon: its disc is cut by it and its halo fades just below.
        let hide = smoothstep(-0.012, 0.004, dir.y);
        let veil = smoothstep(-0.25, 0.04, dir.y);
        // Far below the horizon the sun lights nothing, and its halo never reaches far across the sky.
        let sun_alive = smoothstep(-0.42, -0.08, globals.sun_dir.y);
        let reach = 1.0 - smoothstep(0.35, 1.3, ang);
        let disc = (1.0 - smoothstep(r * 0.9, r * 1.06, ang)) * hide;
        let halo = (exp(-(ang * ang) / (r * r * 60.0)) * 0.55 + exp(-ang / (r * 9.0)) * 0.25) * veil * sun_alive * reach;
        c = c + globals.sun_color.rgb * halo * globals.sun_color.w;
        c = mix(c, globals.sun_color.rgb * 1.6, disc);
    }
    if (clocked && dir.y > -0.02) {
        let vis = globals.night.x;
        let fade = smoothstep(-0.02, 0.12, dir.y);
        if (vis > 0.001) {
            let moon_wash = 1.0 - 0.85 * exp(-acos(clamp(dot(dir, globals.moon_dir.xyz), -1.0, 1.0)) / 0.20) * select(0.0, 1.0, globals.moon_dir.w > 0.0);
            c = c + star_field(dir, px, globals.night.y) * vis * fade * moon_wash;
        }
        if (globals.moon_dir.w > 0.0) {
            let m = moon_disc(dir);
            // The moon is only drawn once the sun is low (its colour would vanish into a bright sky anyway), and it too sits behind the horizon.
            let mvis = clamp(vis * 2.2 + 0.0, 0.0, 1.0) * smoothstep(-0.01, 0.02, dir.y);
            // The disc blocks the stars behind it; its halo adds light.
            c = c * (1.0 - m.a * mvis) + m.rgb * mvis;
        }
    }
    return c;
}

fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    return sky_color_px(dir, 0.0012);
}
