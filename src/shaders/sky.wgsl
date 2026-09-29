// The sky dome, shared by background.wgsl (what you see above the world) and ocean.wgsl (what the water reflects and fades into).
// `gpu.rs` concatenates common.wgsl, then this file, then the shader that uses it.
//
// Globals used: `inv_view_proj` (to turn a pixel into a view direction), `bg_top` = zenith, `bg_bottom` = horizon, `sky.y` = the gradient
// exponent, `sky.z` = 1 when the sky has a sun, `sun_dir` = (direction toward the sun, angular radius in radians), `sun_color` = (rgb, glow).

// The unit direction, in world space, that pixel `ndc` looks along.
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    let far_p = globals.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let near_p = globals.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0);
    return normalize(far_p.xyz / far_p.w - near_p.xyz / near_p.w);
}

// The sky colour along `dir`: horizon at eye level rising to zenith overhead, plus the sun's halo and disc. Below the horizon it is the
// horizon colour (what distant water fades into), so nothing ever shows a dark seam.
fn sky_color(dir: vec3<f32>) -> vec3<f32> {
    let up = clamp(dir.y, 0.0, 1.0);
    var c = mix(globals.bg_bottom.rgb, globals.bg_top.rgb, pow(up, globals.sky.y));
    if (globals.sky.z > 0.5) {
        let s = clamp(dot(dir, globals.sun_dir.xyz), -1.0, 1.0);
        let ang = acos(s);
        let r = globals.sun_dir.w;
        let disc = 1.0 - smoothstep(r * 0.9, r * 1.06, ang);
        let halo = exp(-(ang * ang) / (r * r * 60.0)) * 0.55 + exp(-ang / (r * 9.0)) * 0.25;
        c = c + globals.sun_color.rgb * halo * globals.sun_color.w;
        c = mix(c, globals.sun_color.rgb * 1.6, disc);
    }
    return c;
}
