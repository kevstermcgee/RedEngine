// Frameless: the sky. In the museum it is the blue hour that never ends, seen through the skylights; inside a painting it is the painting itself, all
// around, as far as one can see. `game_param(0)`: x 0 halls / 1 inside, y style, z seed, w how far the world has opened (0..1 as one arrives);
// `game_param(1..4)`: the painting's palette (depth in [1].w); `game_param(5)`: the room's light; `game_param(6)`: time, the wash, key here, keys found.
// The painting functions (`art`, `ramp`, `Pal`) come from shaders/art.wgsl, which the engine puts in front of this file.

fn hall_sky(dir: vec3<f32>, px: f32) -> vec3<f32> {
    let t = sky_time();
    let up = clamp(dir.y, -1.0, 1.0);
    let warm = game_param(5).rgb;
    var c = mix(mix(vec3<f32>(0.32, 0.22, 0.3), warm * 0.4, 0.3), vec3<f32>(0.02, 0.03, 0.09), smoothstep(-0.05, 0.75, up));
    // Stars, a few bright ones twinkling.
    let cell = floor(dir * 160.0);
    let h = hash13(cell + vec3<f32>(sky_seed()));
    let centre = (cell + vec3<f32>(0.5)) / 160.0;
    let d = length(dir - normalize(centre)) * 160.0;
    let star = step(0.9965, h) * smoothstep(0.35, 0.05, d) * smoothstep(0.1, 0.5, up) * (0.6 + 0.4 * sin(t * (1.0 + h * 3.0) + h * 40.0));
    c = c + vec3<f32>(0.9, 0.92, 1.0) * star;
    // Slow clouds lit from below by the city that is not there.
    let cl = fbm(vec3<f32>(dir.x / max(up, 0.15) * 1.2 + t * 0.004, dir.z / max(up, 0.15) * 1.2, 3.0));
    c = mix(c, warm * 0.25 + vec3<f32>(0.08, 0.06, 0.1), smoothstep(0.55, 0.8, cl) * 0.55 * smoothstep(0.05, 0.4, up));
    // The moon.
    let moon = normalize(vec3<f32>(0.35, 0.8, -0.5));
    let md = acos(clamp(dot(dir, moon), -1.0, 1.0));
    c = c + vec3<f32>(1.0, 0.96, 0.85) * (smoothstep(0.035 + px, 0.035 - px, md) * 1.4 + exp(-md * 9.0) * 0.12);
    return c;
}

fn inside_sky(dir: vec3<f32>, px: f32) -> vec3<f32> {
    let g = game_param(0);
    let pal = Pal(game_param(1).rgb, game_param(2).rgb, game_param(3).rgb, game_param(4).rgb);
    let t = sky_time();
    var c = world_color(dir, i32(g.y + 0.5), g.z, pal, t, px * 0.5);
    // As the world opens, the picture spreads outward from where the player looks.
    let ahead = view_ray(vec2<f32>(0.0, 0.0));
    let off = 1.0 - dot(dir, ahead);
    let reveal = smoothstep(g.w * 2.4 - 0.25, g.w * 2.4, off);
    let wash = mix(pal.c, pal.d, 0.5);
    c = mix(c, wash, reveal * (1.0 - g.w));
    return c;
}

fn sky(dir: vec3<f32>, px: f32) -> vec3<f32> {
    if (game_param(0).x < 0.5) {
        return hall_sky(dir, px);
    }
    return inside_sky(dir, px);
}
