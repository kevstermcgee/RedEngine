// Draws a CPU-painted RGBA image (menu text and panels) over the finished frame, one texel per
// screen pixel, alpha-blended. The vertex shader is a single fullscreen triangle.

@group(0) @binding(0) var overlay_tex: texture_2d<f32>;
@group(0) @binding(1) var overlay_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_overlay(@builtin(vertex_index) vi: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: VsOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    out.uv = vec2<f32>(p[vi].x * 0.5 + 0.5, 0.5 - p[vi].y * 0.5);
    return out;
}

@fragment
fn fs_overlay(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(overlay_tex, overlay_samp, in.uv);
}
