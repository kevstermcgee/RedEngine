// "Clarity" post pass: contact ambient occlusion + silhouette outlines, computed from the depth
// buffer alone and applied by *multiplying* the already-lit color (blend: dst * src).
//
// Why: a flat-lit prop in front of a similarly-lit wall reads as one blob — the only thing
// separating them is a thin lit edge. Two cheap cues fix that without touching any material:
//   * ambient occlusion darkens the crevice where an object meets the wall/floor behind or under
//     it (the soft "contact shadow" the eye uses to see that a thing is *on* a surface);
//   * an outline darkens the near side of any depth discontinuity, so every object gets a crisp
//     border against whatever is behind it.
//
// DEPTH_TEXTURE_TYPE is replaced when the shader is built: `texture_depth_multisampled_2d` for
// the live viewer's 4x MSAA depth buffer, `texture_depth_2d` for the offline renderer.

struct Post {
    // near, far, projection x-scale (1 / (aspect * tan(fov/2))), projection y-scale (1 / tan(fov/2))
    cam: vec4<f32>,
    // ao strength, outline strength, ao world radius (m), outline width (px)
    params: vec4<f32>,
    // target width px, target height px, outline threshold, unused
    params2: vec4<f32>,
};
@group(0) @binding(0) var<uniform> post: Post;
@group(0) @binding(1) var depth_tex: DEPTH_TEXTURE_TYPE;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) vi: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: VsOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    return out;
}

fn load_depth(c: vec2<i32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(depth_tex));
    let cc = clamp(c, vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    return textureLoad(depth_tex, cc, 0);
}

// Perspective depth (0..1, DirectX convention) -> distance along the view axis.
fn lin(d: f32) -> f32 {
    let n = post.cam.x;
    let f = post.cam.y;
    return n * f / (f - d * (f - n));
}

fn view_pos(c: vec2<i32>, ell: f32) -> vec3<f32> {
    let px = (vec2<f32>(c) + vec2<f32>(0.5, 0.5)) / post.params2.xy;
    let ndc = vec2<f32>(px.x * 2.0 - 1.0, 1.0 - px.y * 2.0);
    return vec3<f32>(ndc.x * ell / post.cam.z, ndc.y * ell / post.cam.w, -ell);
}

fn pos_at(c: vec2<i32>) -> vec3<f32> {
    return view_pos(c, lin(load_depth(c)));
}

@fragment
fn fs_post(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let c = vec2<i32>(frag.xy);
    let d0 = load_depth(c);
    if (d0 >= 0.99999) {
        return vec4<f32>(1.0, 1.0, 1.0, 1.0);
    }
    let l0 = lin(d0);
    let p0 = view_pos(c, l0);

    // View-space normal from depth differences, taking the smaller step on each axis so the
    // estimate doesn't smear across a silhouette.
    let pl = pos_at(c + vec2<i32>(-1, 0));
    let pr = pos_at(c + vec2<i32>(1, 0));
    let pu = pos_at(c + vec2<i32>(0, -1));
    let pd = pos_at(c + vec2<i32>(0, 1));
    let dx = select(pr - p0, p0 - pl, abs(pl.z - p0.z) < abs(pr.z - p0.z));
    let dy = select(pd - p0, p0 - pu, abs(pu.z - p0.z) < abs(pd.z - p0.z));
    var n = normalize(cross(dy, dx));
    if (dot(n, p0) > 0.0) {
        n = -n;
    }

    // ---- ambient occlusion: how much of the hemisphere around p0 is blocked nearby ----------
    let ao_strength = post.params.x;
    let radius = post.params.z;
    var ao = 0.0;
    if (ao_strength > 0.001) {
        // World radius -> pixels at this depth, kept within sane bounds.
        let r_px = clamp(radius * (0.5 * post.params2.y * post.cam.w) / l0, 2.0, 60.0);
        // Each pixel of a 4x4 tile rotates the sample spiral by a different step of an ordered
        // (Bayer) pattern: any 4x4 neighborhood then covers all 16 rotations, so the result is
        // a fine regular dither the eye integrates away — unlike white noise, which reads as
        // visible grain on flat walls.
        let bx = u32(frag.x) & 3u;
        let by = u32(frag.y) & 3u;
        var bayer = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
        let rot = (bayer[by * 4u + bx] / 16.0) * 6.2831853;
        var occ = 0.0;
        for (var i = 0; i < 16; i = i + 1) {
            let fi = f32(i);
            let a = fi * 2.39996323 + rot;
            let r = sqrt((fi + 0.5) / 16.0) * r_px;
            let o = vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * r));
            let v = pos_at(c + o) - p0;
            let dist = length(v);
            let elevation = (dot(n, v) - 0.02 - 0.003 * l0) / max(dist, 0.001);
            // Occluders farther than ~2 radii belong to some other object, not a crevice here.
            let reach = 1.0 - smoothstep(radius, radius * 2.0, dist);
            occ = occ + clamp(elevation, 0.0, 1.0) * reach;
        }
        ao = clamp(occ / 16.0 * 2.2 * ao_strength, 0.0, 0.85);
        // Fade with distance: far away it's mostly noise.
        ao = ao * (1.0 - smoothstep(18.0, 40.0, l0));
    }

    // ---- outline: darken the *nearer* side of a depth discontinuity -------------------------
    var edge = 0.0;
    let outline_strength = post.params.y;
    if (outline_strength > 0.001) {
        let w = max(1, i32(round(post.params.w)));
        let tau = post.params2.z;
        let inv0 = 1.0 / l0;
        var dirs = array<vec2<i32>, 4>(vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(1, 1), vec2<i32>(1, -1));
        for (var k = 0; k < 4; k = k + 1) {
            let o = dirs[k] * w;
            let ia = 1.0 / lin(load_depth(c + o));
            let ib = 1.0 / lin(load_depth(c - o));
            // 1/depth is linear across any flat surface, so this second difference is ~0 on
            // planes at any slope and strongly negative only where this pixel is in front of
            // something much farther.
            let second = ia + ib - 2.0 * inv0;
            edge = max(edge, smoothstep(tau, tau * 3.0, -second / inv0));
        }
    }

    let m = (1.0 - ao) * (1.0 - outline_strength * edge);
    return vec4<f32>(m, m, m, 1.0);
}
