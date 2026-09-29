// The ocean: an analytic plane drawn from the eye to the horizon (a fullscreen triangle intersects each pixel's view ray with the water
// level), lit by the sky, translucent over the shore. `gpu.rs` concatenates common.wgsl, sky.wgsl, then this file.
//
// Group 1: the water's own numbers and the heights of the terrain under it (so the water knows how deep it is at every pixel).

struct OceanU {
    // water level y, wave amplitude, wave frequency (rad/m), speed
    wave: vec4<f32>,
    // deep colour, roughness
    deep: vec4<f32>,
    // shallow colour, depth scale (m)
    shallow: vec4<f32>,
    // foam colour, haze distance (m)
    foam: vec4<f32>,
    // terrain origin x, origin z, cell x, cell z
    terr: vec4<f32>,
    // terrain samples x, samples z, periodic axis (0 none, 1 x, 2 z), has terrain (1 / 0)
    terr2: vec4<f32>,
    // seconds
    time: vec4<f32>,
};
@group(1) @binding(0) var<uniform> ocean: OceanU;
@group(1) @binding(1) var<storage, read> heights: array<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_ocean(@builtin(vertex_index) vi: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: VsOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.ndc = p[vi];
    return out;
}

// World height of the terrain at xz (bilinear over the sample texture), or a very low number where there is none (open deep water).
fn terrain_h(xz: vec2<f32>) -> f32 {
    if (ocean.terr2.w < 0.5) {
        return -1000.0;
    }
    let dims = vec2<i32>(i32(ocean.terr2.x), i32(ocean.terr2.y));
    var u = (xz.x - ocean.terr.x) / ocean.terr.z;
    var v = (xz.y - ocean.terr.y) / ocean.terr.w;
    let per = i32(ocean.terr2.z);
    var cells = vec2<f32>(f32(dims.x - 1), f32(dims.y - 1));
    if (per == 1) {
        cells.x = f32(dims.x);
        u = u - floor(u / cells.x) * cells.x;
    }
    if (per == 2) {
        cells.y = f32(dims.y);
        v = v - floor(v / cells.y) * cells.y;
    }
    if (u < 0.0 || v < 0.0 || u > cells.x || v > cells.y) {
        return -1000.0;
    }
    let iu = min(i32(floor(u)), i32(cells.x) - 1);
    let iv = min(i32(floor(v)), i32(cells.y) - 1);
    let fu = u - f32(iu);
    let fv = v - f32(iv);
    var iu1 = iu + 1;
    var iv1 = iv + 1;
    if (iu1 >= dims.x) {
        iu1 = 0;
    }
    if (iv1 >= dims.y) {
        iv1 = 0;
    }
    let h00 = heights[iv * dims.x + iu];
    let h10 = heights[iv * dims.x + iu1];
    let h01 = heights[iv1 * dims.x + iu];
    let h11 = heights[iv1 * dims.x + iu1];
    return mix(mix(h00, h10, fu), mix(h01, h11, fu), fv);
}

// Six crossing swells at unrelated angles and wavelengths (deep-water dispersion, w = sqrt(g k)), with the position warped by a slow swell so
// the pattern never lines up into a grid: returns (height, d/dx, d/dz), unit amplitude.
fn waves(p0: vec2<f32>, t: f32) -> vec3<f32> {
    var dirs = array<vec2<f32>, 6>(vec2<f32>(0.96, 0.29), vec2<f32>(0.76, -0.65), vec2<f32>(-0.20, 0.98), vec2<f32>(0.49, 0.87), vec2<f32>(-0.83, 0.56), vec2<f32>(0.12, -0.99));
    var kmul = array<f32, 6>(1.0, 1.63, 2.71, 4.37, 6.91, 9.37);
    var amps = array<f32, 6>(1.0, 0.62, 0.38, 0.22, 0.12, 0.07);
    let p = p0 + vec2<f32>(sin(p0.y * 0.043 + t * 0.21), cos(p0.x * 0.037 - t * 0.17)) * 3.5;
    var sum = vec3<f32>(0.0);
    var norm = 0.0;
    for (var i = 0; i < 6; i = i + 1) {
        let k = ocean.wave.z * kmul[i];
        let w = sqrt(9.81 * k) * ocean.wave.w;
        let phase = dot(dirs[i], p) * k - w * t + f32(i) * 2.3;
        sum.x = sum.x + amps[i] * sin(phase);
        sum.y = sum.y + amps[i] * k * cos(phase) * dirs[i].x;
        sum.z = sum.z + amps[i] * k * cos(phase) * dirs[i].y;
        norm = norm + amps[i];
    }
    return sum / norm;
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_ocean(in: VsOut) -> FsOut {
    let o = globals.camera_pos.xyz;
    let d = view_ray(in.ndc);
    let level = ocean.wave.x;
    // Above the horizon, or the eye under the water: nothing to draw here (the sky is behind).
    if (o.y <= level + 0.02 || d.y >= -0.00005) {
        discard;
    }
    let t = (level - o.y) / d.y;
    let p = o + d * t;
    let time = ocean.time.x;
    let rough = ocean.deep.w;

    let w = waves(p.xz, time);
    // Waves lean the normal (more when rough), and are averaged out with distance so the far sea does not shimmer.
    let lean = ocean.wave.y * 4.5 * (1.0 + 10.0 * rough) * (1.0 - smoothstep(25.0, 160.0, t));
    let n = normalize(vec3<f32>(-w.y * lean, 1.0, -w.z * lean));

    // How deep the water is here (the swell rides up the shore a little).
    let depth = max(level + w.x * ocean.wave.y - terrain_h(p.xz), 0.0);
    let scale = ocean.shallow.w;
    var body = mix(ocean.shallow.rgb, ocean.deep.rgb, smoothstep(0.0, scale, depth));

    // What the surface reflects: the sky, more at a grazing angle (Fresnel), and the sun as a glitter path.
    let r = reflect(d, n);
    let rdir = normalize(vec3<f32>(r.x, max(r.y, 0.02), r.z));
    let cosv = clamp(dot(-d, n), 0.0, 1.0);
    let fres = 0.02 + 0.98 * pow(1.0 - cosv, 5.0);
    var color = mix(body, sky_color(rdir), clamp(fres * 0.9 + 0.04, 0.0, 1.0));
    if (globals.sky.z > 0.5) {
        let along = max(dot(rdir, globals.sun_dir.xyz), 0.0);
        let glitter = (pow(along, mix(900.0, 120.0, rough)) * 2.0 * (1.0 - smoothstep(40.0, 180.0, t)) + pow(along, 18.0) * 0.12);
        color = color + globals.sun_color.rgb * globals.sun_color.w * glitter;
    }

    // Foam: a wash that runs up and back down the beach, broken into speckles, with a thinner line trailing it.
    let wash = 0.30 + 0.22 * sin(time * 0.85 + p.z * 0.11 + sin(p.x * 0.07) * 2.0);
    let speckle = 0.65 + 0.35 * sin(p.x * 2.3 + time * 1.7) * sin(p.z * 3.1 - time * 1.1);
    var foam = (1.0 - smoothstep(0.0, wash, depth)) * speckle;
    let trail = (1.0 - smoothstep(0.0, 0.12, abs(depth - (wash + 0.55)))) * 0.45 * step(0.02, depth);
    foam = clamp(foam + trail * speckle, 0.0, 1.0);
    color = mix(color, ocean.foam.rgb, foam * 0.9);

    // Clear in the shallows, opaque in the deep; foam always shows.
    var alpha = mix(0.22, 1.0, smoothstep(0.0, scale * 0.7, depth));
    alpha = max(alpha, foam * 0.95);

    // Distance: fade into the horizon colour (in this direction, so the sun's halo carries across the horizon too).
    let haze = 1.0 - exp(-pow(t / ocean.foam.w, 1.6) * 3.0);
    color = mix(color, sky_color(normalize(vec3<f32>(d.x, 0.0, d.z))), haze);
    alpha = mix(alpha, 1.0, haze);

    let clip = globals.view_proj * vec4<f32>(p, 1.0);
    var out: FsOut;
    out.color = vec4<f32>(color, alpha);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
