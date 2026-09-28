// A screen-space crosshair: four short arms around a gap, each with a thin dark outline so it reads on any background. Sizes are in pixels
// (scaled up on tall windows) and converted to NDC in the vertex shader, so it keeps its size whatever the window. Drawn as the last pass
// over the already-shaded frame (no depth test: always on top, like a HUD).

struct Crosshair {
    color: vec4<f32>,
    // xy = 2/viewport_width, 2/viewport_height (pixel-offset -> NDC-offset factors); z = size scale (1 at 720p); w unused.
    to_ndc: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Crosshair;

const GAP: f32 = 4.0;
const ARM: f32 = 8.0;
const HALF_THICK: f32 = 1.0;
const OUTLINE: f32 = 1.0;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // 0 for the dark outline, 1 for the coloured arm.
    @location(0) shade: f32,
};

@vertex
fn vs_crosshair(@builtin(vertex_index) vi: u32) -> VsOut {
    // Eight rectangles of six vertices: for each of four arms, its outline (drawn first) and its fill.
    let quad = vi / 6u;
    let corner = vi % 6u;
    let arm = quad / 2u; // 0 left, 1 right, 2 up, 3 down
    let filled = quad % 2u;
    let grow = select(OUTLINE, 0.0, filled == 1u);
    // The arm's rectangle along its own axis (from the gap outward) and across it.
    let lo = vec2<f32>(GAP - grow, -HALF_THICK - grow);
    let hi = vec2<f32>(GAP + ARM + grow, HALF_THICK + grow);
    var cx = array<u32, 6>(0u, 1u, 1u, 0u, 1u, 0u);
    var cy = array<u32, 6>(0u, 0u, 1u, 0u, 1u, 1u);
    let along = select(lo.x, hi.x, cx[corner] == 1u);
    let across = select(lo.y, hi.y, cy[corner] == 1u);
    var p = vec2<f32>(0.0, 0.0);
    switch arm {
        case 0u: {
            p = vec2<f32>(-along, across);
        }
        case 1u: {
            p = vec2<f32>(along, across);
        }
        case 2u: {
            p = vec2<f32>(across, -along);
        }
        default: {
            p = vec2<f32>(across, along);
        }
    }
    var out: VsOut;
    out.pos = vec4<f32>(p * u.to_ndc.z * u.to_ndc.xy, 0.0, 1.0);
    out.shade = f32(filled);
    return out;
}

@fragment
fn fs_crosshair(in: VsOut) -> @location(0) vec4<f32> {
    if (in.shade < 0.5) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.6 * u.color.a);
    }
    return u.color;
}
