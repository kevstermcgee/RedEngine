struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc_y: f32,
};

@vertex
fn vs_bg(@builtin(vertex_index) vi: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = positions[vi];
    var out: VsOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.ndc_y = p.y;
    return out;
}

@fragment
fn fs_bg(in: VsOut) -> @location(0) vec4<f32> {
    if (globals.bg_top.w < 0.5) {
        return vec4<f32>(globals.bg_top.rgb, 1.0);
    }
    let t = clamp(in.ndc_y * 0.5 + 0.5, 0.0, 1.0);
    let color = mix(globals.bg_bottom.rgb, globals.bg_top.rgb, t);
    return vec4<f32>(color, 1.0);
}
