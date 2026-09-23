// A tiny screen-space crosshair: two thin bars (a "+"), sized in pixels and converted to NDC
// in the vertex shader so it stays a constant size regardless of window/resolution. Drawn as
// the last pass over the already-shaded frame (no depth test — always on top, like a HUD).

struct Crosshair {
    color: vec4<f32>,
    // xy = 2/viewport_width, 2/viewport_height (pixel-offset -> NDC-offset factors); zw unused.
    to_ndc: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Crosshair;

const ARM: f32 = 9.0;
const HALF_THICK: f32 = 1.25;

@vertex
fn vs_crosshair(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var px = array<vec2<f32>, 12>(
        // horizontal bar
        vec2<f32>(-ARM, -HALF_THICK), vec2<f32>(ARM, -HALF_THICK), vec2<f32>(ARM, HALF_THICK),
        vec2<f32>(-ARM, -HALF_THICK), vec2<f32>(ARM, HALF_THICK), vec2<f32>(-ARM, HALF_THICK),
        // vertical bar
        vec2<f32>(-HALF_THICK, -ARM), vec2<f32>(HALF_THICK, -ARM), vec2<f32>(HALF_THICK, ARM),
        vec2<f32>(-HALF_THICK, -ARM), vec2<f32>(HALF_THICK, ARM), vec2<f32>(-HALF_THICK, ARM),
    );
    let p = px[vi] * u.to_ndc.xy;
    return vec4<f32>(p, 0.0, 1.0);
}

@fragment
fn fs_crosshair() -> @location(0) vec4<f32> {
    return u.color;
}
