// RedCraft: every block face in the world, painted as 16 x 16 pixel art.
//
// The engine calls `surface(s)` for every pixel of a painted object (`redcraft.art`). The mesh's vertex colour is data (the object's material.z is 1):
// r = the texture number / 255 (block textures 1..57, flat model colours 128..), g = sky light 0..1 and b = block light 0..1, both darkened in
// occluded corners. s.p is the point in the section's own space (a block is a unit cell), s.n its normal there. Returns colour and glow.
//
// game_param(0) = the aimed-at block's corner (the renderer's space) and the mining stage (0..10) in w; game_param(1).x = 1 when a block is aimed at;
// game_param(2).x = how much daylight there is (0 night .. 1 noon), so a torch shows where the sky does not.
//
// games/redcraft/src/icons.rs paints the same pictures for the inventory: keep the two alike.

fn hsh(a: u32) -> u32 {
    var x = a;
    x = x ^ (x >> 16u);
    x = x * 0x7feb352du;
    x = x ^ (x >> 15u);
    x = x * 0x846ca68bu;
    x = x ^ (x >> 16u);
    return x;
}

fn hash3u(x: u32, y: u32, z: u32) -> u32 {
    return hsh((x * 73856093u) ^ hsh((y * 19349663u) ^ hsh(z * 83492791u)));
}

// A die per texel, 0..1.
fn rnd(x: i32, y: i32, k: u32) -> f32 {
    return f32(hash3u(u32(x & 15), u32(y & 15), k) & 0xffffu) / 65535.0;
}

fn hex(v: u32) -> vec3<f32> {
    let c = vec3<f32>(f32((v >> 16u) & 255u), f32((v >> 8u) & 255u), f32(v & 255u)) / 255.0;
    return pow(c, vec3<f32>(2.2));
}

// Soft blotches across the texture (value noise on a grid of `cell` texels, tiling every 16).
fn blotch(x: i32, y: i32, k: u32, cell: i32) -> f32 {
    let n = 16 / cell;
    let gx = x / cell;
    let gy = y / cell;
    let fx = f32(x - gx * cell) / f32(cell);
    let fy = f32(y - gy * cell) / f32(cell);
    let a = rnd(gx % n, gy % n, k);
    let b = rnd((gx + 1) % n, gy % n, k);
    let c = rnd(gx % n, (gy + 1) % n, k);
    let d = rnd((gx + 1) % n, (gy + 1) % n, k);
    return mix(mix(a, b, fx), mix(c, d, fx), fy);
}

// One of four shades by a value 0..1.
fn shade4(a: u32, b: u32, c: u32, d: u32, v: f32) -> vec3<f32> {
    if (v < 0.25) {
        return hex(a);
    }
    if (v < 0.5) {
        return hex(b);
    }
    if (v < 0.75) {
        return hex(c);
    }
    return hex(d);
}

// Stones packed in mortar: the distance to the nearest feature point and the gap to the second (tiling cells of 4 texels).
fn cells(x: i32, y: i32, k: u32) -> vec3<f32> {
    var d1 = 99.0;
    var d2 = 99.0;
    var id = 0.0;
    let gx = x / 4;
    let gy = y / 4;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let cx = gx + i;
            let cy = gy + j;
            let wx = (cx + 4) % 4;
            let wy = (cy + 4) % 4;
            let fx = f32(cx * 4) + 0.5 + rnd(wx, wy, k) * 3.0;
            let fy = f32(cy * 4) + 0.5 + rnd(wx, wy, k + 7u) * 3.0;
            let d = length(vec2<f32>(f32(x) + 0.5 - fx, f32(y) + 0.5 - fy));
            if (d < d1) {
                d2 = d1;
                d1 = d;
                id = rnd(wx, wy, k + 13u);
            } else if (d < d2) {
                d2 = d;
            }
        }
    }
    return vec3<f32>(d1, d2 - d1, id);
}

fn t_stone(x: i32, y: i32) -> vec3<f32> {
    let v = rnd(x, y, 1u) * 0.55 + blotch(x, y, 2u, 4) * 0.45;
    return shade4(0x6b6b6bu, 0x777777u, 0x828282u, 0x8e8e8eu, v);
}

fn t_dirt(x: i32, y: i32) -> vec3<f32> {
    let v = rnd(x, y, 3u) * 0.7 + blotch(x, y, 4u, 8) * 0.3;
    var c = shade4(0x5e4230u, 0x77553bu, 0x866144u, 0x93704fu, v);
    if (rnd(x, y, 5u) > 0.93) {
        c = hex(0xa1876au);
    }
    return c;
}

// Grass, tinted by where in the world it grows (a gentle drift of greens).
fn grass_tint(world: vec3<f32>) -> vec3<f32> {
    let w = 0.5 + 0.5 * sin(world.x * 0.011 + 1.3) * sin(world.z * 0.009 - 0.4);
    return mix(vec3<f32>(0.92, 1.04, 0.86), vec3<f32>(1.08, 0.98, 0.84), w);
}

fn t_grass_top(x: i32, y: i32) -> vec3<f32> {
    let v = rnd(x, y, 6u) * 0.75 + blotch(x, y, 7u, 4) * 0.25;
    return shade4(0x4f8a32u, 0x5d9c3cu, 0x69a845u, 0x77b551u, v);
}

fn t_grass_side(x: i32, y: i32, snow: bool) -> vec3<f32> {
    // The overhang: two or three texels, and a fourth here and there.
    let hang = 2 + i32(rnd(x, 0, 8u) * 2.0) + select(0, 1, rnd(x, 1, 8u) > 0.8);
    if (y < hang) {
        if (snow) {
            return shade4(0xdcebefu, 0xe8f3f5u, 0xf2fafbu, 0xffffffu, rnd(x, y, 9u));
        }
        return t_grass_top(x, y);
    }
    return t_dirt(x, y);
}

fn t_cobble(x: i32, y: i32, moss: bool) -> vec3<f32> {
    let c = cells(x, y, 10u);
    if (c.y < 0.7) {
        return hex(0x4a4a4au);
    }
    let edge = clamp((c.y - 0.7) / 1.6, 0.0, 1.0);
    var col = mix(hex(0x6a6a6au), hex(0x8d8d8du), c.z * 0.6 + rnd(x, y, 11u) * 0.4);
    col = col * mix(0.82, 1.06, edge);
    if (moss && blotch(x, y, 12u, 4) > 0.52 && rnd(x, y, 13u) > 0.25) {
        col = shade4(0x3f6b2bu, 0x4c7a31u, 0x58883au, 0x63963fu, rnd(x, y, 14u));
    }
    return col;
}

// Boards four texels high, a darker seam under each and a joint somewhere along it.
fn t_planks(x: i32, y: i32, light: u32, mid: u32, dark: u32, seam: u32) -> vec3<f32> {
    let board = y / 4;
    if (y % 4 == 3) {
        return hex(seam);
    }
    let joint = i32(rnd(board, 0, 15u) * 16.0);
    if (x == joint) {
        return hex(seam);
    }
    let grain = rnd(x / 3, y, 16u + u32(board)) * 0.6 + rnd(x, y, 17u) * 0.4;
    if (grain < 0.2) {
        return hex(dark);
    }
    if (grain > 0.82) {
        return hex(light);
    }
    return hex(mid);
}

fn t_bedrock(x: i32, y: i32) -> vec3<f32> {
    return shade4(0x1f1f1fu, 0x3a3a3au, 0x555555u, 0x7a7a7au, rnd(x, y, 18u) * 0.6 + blotch(x, y, 19u, 4) * 0.4);
}

fn t_sand(x: i32, y: i32) -> vec3<f32> {
    return shade4(0xcfc08eu, 0xd8ca9au, 0xdfd3a6u, 0xe7ddb5u, rnd(x, y, 20u) * 0.8 + blotch(x, y, 21u, 8) * 0.2);
}

fn t_gravel(x: i32, y: i32) -> vec3<f32> {
    let c = cells(x, y, 22u);
    if (c.y < 0.45) {
        return hex(0x4e4846u);
    }
    let k = c.z;
    var col = hex(0x7f7a78u);
    if (k < 0.25) {
        col = hex(0x9a9592u);
    } else if (k < 0.5) {
        col = hex(0x6a6463u);
    } else if (k < 0.65) {
        col = hex(0x8a7b6eu);
    }
    return col * (0.88 + rnd(x, y, 23u) * 0.22);
}

// Ore spots over stone: a fixed set of clusters.
fn ore_spot(x: i32, y: i32) -> i32 {
    // Five clusters, each a few texels; 0 none, 1 the spot's body, 2 its highlight.
    var centres = array<vec2<i32>, 5>(vec2<i32>(3, 3), vec2<i32>(11, 2), vec2<i32>(7, 8), vec2<i32>(2, 12), vec2<i32>(12, 12));
    for (var i = 0; i < 5; i = i + 1) {
        let d = vec2<i32>(x, y) - centres[i];
        let m = abs(d.x) + abs(d.y);
        if (m == 0) {
            return 2;
        }
        if (m == 1 && rnd(x, y, 24u + u32(i)) > 0.2) {
            return 1;
        }
        if (m == 2 && abs(d.x) == 1 && rnd(x, y, 30u) > 0.55) {
            return 1;
        }
    }
    return 0;
}

fn t_ore(x: i32, y: i32, body: u32, hi: u32) -> vec3<f32> {
    let s = ore_spot(x, y);
    if (s == 2) {
        return hex(hi);
    }
    if (s == 1) {
        return hex(body);
    }
    return t_stone(x, y);
}

fn t_bark(x: i32, y: i32, a: u32, b: u32, c: u32, d: u32) -> vec3<f32> {
    let streak = rnd(x, y / 3, 31u) * 0.6 + rnd(x, y, 32u) * 0.4;
    return shade4(a, b, c, d, streak);
}

fn t_rings(x: i32, y: i32, bark: u32, inner: u32, ring: u32) -> vec3<f32> {
    let d = length(vec2<f32>(f32(x) - 7.5, f32(y) - 7.5));
    if (d > 6.9) {
        return hex(bark);
    }
    let r = fract(d / 2.2 + rnd(x, y, 33u) * 0.15);
    if (r < 0.3) {
        return hex(ring);
    }
    return hex(inner) * (0.94 + rnd(x, y, 34u) * 0.1);
}

fn t_birch_bark(x: i32, y: i32) -> vec3<f32> {
    // White bark with black dashes across.
    let row = y;
    let dash = rnd(x / 3, row, 35u) > 0.78 && rnd(x, row, 36u) > 0.25;
    if (dash) {
        return hex(0x2e2c28u);
    }
    return shade4(0xc9c8c0u, 0xd6d5cdu, 0xe0dfd8u, 0xeeede6u, rnd(x, y, 37u));
}

fn t_leaves(x: i32, y: i32, a: u32, b: u32, c: u32, hole: u32) -> vec3<f32> {
    let v = rnd(x, y, 38u);
    if (v < 0.16) {
        return hex(hole);
    }
    return shade4(a, a, b, c, v * 0.8 + blotch(x, y, 39u, 4) * 0.2);
}

fn t_glass(x: i32, y: i32) -> vec3<f32> {
    if (x == 0 || y == 0 || x == 15 || y == 15) {
        return hex(0xf2fbffu);
    }
    // Two glints.
    if ((x - y == 2 && x > 2 && x < 7) || (x - y == -3 && x > 8 && x < 12)) {
        return hex(0xffffffu);
    }
    return hex(0xa9cfdcu);
}

fn t_sandstone(x: i32, y: i32, top: bool) -> vec3<f32> {
    if (top) {
        return shade4(0xd5c592u, 0xdcce9eu, 0xe1d4a6u, 0xe6dbb1u, rnd(x, y, 40u) * 0.5 + 0.3);
    }
    if (y < 3) {
        return shade4(0xdccf9fu, 0xe2d6aau, 0xe7dcb3u, 0xede3beu, rnd(x, y, 41u));
    }
    if (y > 12) {
        return shade4(0xc6b682u, 0xcdbd8bu, 0xd3c493u, 0xd9cb9cu, rnd(x, y, 42u));
    }
    let band = select(0.0, 0.25, (y + i32(rnd(x / 4, 0, 43u) * 2.0)) % 4 == 0);
    return shade4(0xcdbd8bu, 0xd4c595u, 0xdacc9eu, 0xe0d3a8u, rnd(x, y, 44u) * 0.7 + band);
}

fn t_bricks(x: i32, y: i32) -> vec3<f32> {
    let row = y / 4;
    let off = select(0, 4, row % 2 == 1);
    if (y % 4 == 3 || (x + off) % 8 == 7) {
        return shade4(0x9b9389u, 0xa79f95u, 0xb0a99fu, 0xbab3aau, rnd(x, y, 45u));
    }
    return shade4(0x8a4f3fu, 0x96594au, 0xa16454u, 0xab6d5cu, rnd(x, y, 46u) * 0.6 + rnd((x + off) / 8, row, 47u) * 0.4);
}

fn t_stone_bricks(x: i32, y: i32) -> vec3<f32> {
    let row = y / 8;
    let off = select(0, 8, row % 2 == 1);
    let lx = (x + off) % 16;
    let ly = y % 8;
    if (ly == 7 || lx == 15 || lx == 7) {
        return hex(0x585858u);
    }
    var c = shade4(0x707070u, 0x7a7a7au, 0x838383u, 0x8c8c8cu, rnd(x, y, 48u) * 0.7 + blotch(x, y, 49u, 4) * 0.3);
    if (ly == 0 || lx == 0 || lx == 8) {
        c = c * 1.12;
    }
    return c;
}

fn t_table_top(x: i32, y: i32) -> vec3<f32> {
    if (x == 0 || y == 0 || x == 15 || y == 15) {
        return hex(0x5c3e1fu);
    }
    if (x == 5 || x == 10 || y == 5 || y == 10) {
        return hex(0x7a5a33u);
    }
    return t_planks(x, y, 0xc49c62u, 0xb38d55u, 0x9c7a47u, 0x8c6c3eu);
}

fn t_table_side(x: i32, y: i32, front: bool) -> vec3<f32> {
    if (y < 3) {
        return select(hex(0x6e4b26u), hex(0x5c3e1fu), y == 2);
    }
    if (x == 0 || x == 15) {
        return hex(0x5c3e1fu);
    }
    // A saw and a hammer hang on the front; a tool rack on the sides.
    if (front) {
        if (x >= 3 && x <= 6 && y >= 5 && y <= 11) {
            return select(hex(0xb8b8b8u), hex(0x8a8a8au), (x + y) % 2 == 0);
        }
        if (x >= 4 && x <= 5 && y >= 12 && y <= 13) {
            return hex(0x5c3e1fu);
        }
        if (x >= 9 && x <= 12 && y >= 5 && y <= 6) {
            return hex(0x9a9a9au);
        }
        if (x >= 10 && x <= 11 && y >= 7 && y <= 13) {
            return hex(0x6e4b26u);
        }
    } else if (y >= 6 && y <= 12 && (x == 4 || x == 8 || x == 12)) {
        return hex(0x6e4b26u);
    }
    return t_planks(x, y, 0xb08a52u, 0xa07c48u, 0x8c6c3eu, 0x7d5f35u);
}

fn t_snow(x: i32, y: i32) -> vec3<f32> {
    return shade4(0xe3eef0u, 0xebf4f6u, 0xf3f9fau, 0xffffffu, rnd(x, y, 50u) * 0.7 + 0.2);
}

fn t_ice(x: i32, y: i32) -> vec3<f32> {
    if ((x + y * 2) % 11 == 0 && rnd(x, y, 51u) > 0.3) {
        return hex(0xd8e6ffu);
    }
    return shade4(0x7ea2f2u, 0x86abf8u, 0x8fb3feu, 0x99bbffu, rnd(x, y, 52u) * 0.5 + blotch(x, y, 53u, 8) * 0.5);
}

fn t_cactus(x: i32, y: i32, top: bool) -> vec3<f32> {
    if (top) {
        let d = length(vec2<f32>(f32(x) - 7.5, f32(y) - 7.5));
        if (d < 2.0) {
            return hex(0x8bc34au);
        }
        return select(hex(0x147d24u), hex(0x0d6a1bu), (x + y) % 3 == 0);
    }
    if (x % 4 == 1 && y % 3 == 1) {
        return hex(0xc8e6a0u);
    }
    if (x % 4 == 0) {
        return hex(0x0b5e18u);
    }
    return shade4(0x107421u, 0x158028u, 0x1a8a2eu, 0x1f9434u, rnd(x, y, 54u));
}

fn t_clay(x: i32, y: i32) -> vec3<f32> {
    return shade4(0x969ca6u, 0x9ea4aeu, 0xa5abb5u, 0xaeb3bcu, rnd(x, y, 55u) * 0.6 + blotch(x, y, 56u, 8) * 0.4);
}

fn t_obsidian(x: i32, y: i32) -> vec3<f32> {
    let v = rnd(x, y, 57u) * 0.5 + blotch(x, y, 58u, 4) * 0.5;
    if (rnd(x, y, 59u) > 0.94) {
        return hex(0x4b3577u);
    }
    return shade4(0x0f0d16u, 0x16121fu, 0x1e1829u, 0x2a2138u, v);
}

fn t_bookshelf(x: i32, y: i32) -> vec3<f32> {
    if (y == 0 || y == 15 || y == 7 || y == 8) {
        return t_planks(x, y, 0xc49c62u, 0xb38d55u, 0x9c7a47u, 0x8c6c3eu);
    }
    if (x == 0 || x == 15) {
        return hex(0x6e4b26u);
    }
    // Books: a colour per spine, a gap now and then.
    let shelf = select(0u, 1u, y > 7);
    let spine = x / 2;
    let k = rnd(spine, i32(shelf), 60u);
    let top = select(1, 9, y > 7) + i32(rnd(spine, i32(shelf), 61u) * 2.0);
    if (y < top || k > 0.92) {
        return hex(0x3a2512u);
    }
    var c = hex(0x7a2a24u);
    if (k < 0.2) {
        c = hex(0x2f4f8au);
    } else if (k < 0.4) {
        c = hex(0x3d6b35u);
    } else if (k < 0.55) {
        c = hex(0x8a6a3au);
    } else if (k < 0.7) {
        c = hex(0x5a3d6bu);
    }
    if (x % 2 == 1) {
        c = c * 0.8;
    }
    return c;
}

fn t_metal(x: i32, y: i32, light: u32, mid: u32, dark: u32) -> vec3<f32> {
    if (x == 0 || y == 0) {
        return hex(light);
    }
    if (x == 15 || y == 15) {
        return hex(dark);
    }
    if ((x == 1 && y < 15) || (y == 1 && x < 15)) {
        return mix(hex(light), hex(mid), 0.5);
    }
    return hex(mid) * (0.95 + rnd(x, y, 62u) * 0.08);
}

fn t_tnt(x: i32, y: i32, face: i32) -> vec3<f32> {
    if (face == 0) {
        // The side: a white band with the letters across the middle.
        if (y >= 5 && y <= 10) {
            var letters = array<u32, 4>(0xe97u, 0x4d2u, 0x4b2u, 0x492u);
            if (y >= 6 && y <= 9 && x >= 2 && x <= 13) {
                let bit = (letters[y - 6] >> u32(13 - x)) & 1u;
                if (bit == 1u) {
                    return hex(0x1a1a1au);
                }
            }
            return hex(0xe8e8e8u);
        }
        let stick = x % 4;
        return select(hex(0xc9281au), hex(0xa11c12u), stick == 0);
    }
    if (face == 1 && abs(x - 7) <= 1 && abs(y - 7) <= 1) {
        return select(hex(0x5b5b5bu), hex(0x2a2a2au), x == 7 && y == 7);
    }
    let d = length(vec2<f32>(f32(x % 8) - 3.5, f32(y % 8) - 3.5));
    return select(hex(0xc9281au), hex(0x9e1b10u), d > 3.2);
}

fn t_smooth_stone(x: i32, y: i32) -> vec3<f32> {
    if (x == 0 || y == 0 || x == 15 || y == 15) {
        return hex(0x5f5f5fu);
    }
    return shade4(0x7c7c7cu, 0x828282u, 0x888888u, 0x8e8e8eu, rnd(x, y, 63u));
}

// The furnace's front: its mouth dark, or burning.
fn t_furnace_front(x: i32, y: i32, lit: bool) -> vec4<f32> {
    if (x >= 4 && x <= 11 && y >= 8 && y <= 13) {
        if (lit) {
            let flicker = 0.5 + 0.5 * sin(sky_time() * 9.0 + f32(x) * 1.7);
            let heat = clamp(f32(y - 8) / 5.0 + rnd(x, y, 64u) * 0.35 * flicker, 0.0, 1.0);
            return vec4<f32>(mix(hex(0xb83a0au), hex(0xffd36au), heat), 2.5);
        }
        return vec4<f32>(select(hex(0x1c1c1cu), hex(0x2a2a2au), y == 8), 0.0);
    }
    if (y >= 3 && y <= 5 && x >= 3 && x <= 12) {
        return vec4<f32>(hex(0x4a4a4au), 0.0);
    }
    return vec4<f32>(t_cobble(x, y, false) * 1.05, 0.0);
}

fn t_chest(x: i32, y: i32, which: i32) -> vec3<f32> {
    let frame = x == 0 || x == 15 || y == 0 || y == 15;
    if (frame) {
        return hex(0x3d2814u);
    }
    if (which == 0) {
        if (y == 5) {
            return hex(0x3d2814u);
        }
        if (x >= 7 && x <= 8 && y >= 4 && y <= 7) {
            return select(hex(0xc8c8c8u), hex(0x8f8f8fu), y == 7);
        }
    }
    if (which == 1 && y == 5) {
        return hex(0x3d2814u);
    }
    return t_planks(x, y, 0xb3772eu, 0xa36a26u, 0x8d5a1fu, 0x7d4f1bu);
}

fn t_water(x: i32, y: i32, world: vec3<f32>) -> vec3<f32> {
    let t = sky_time();
    let w = sin(f32(x) * 0.8 + t * 1.6 + world.z * 0.7) + sin(f32(y) * 0.9 - t * 1.1 + world.x * 0.5);
    let v = 0.5 + 0.25 * w + rnd(x, y, 65u + u32(t * 2.0) % 8u) * 0.15;
    return shade4(0x2a4fb8u, 0x3159c4u, 0x3a64ceu, 0x4a75dau, v);
}

fn t_lava(x: i32, y: i32, world: vec3<f32>) -> vec3<f32> {
    let t = sky_time() * 0.35;
    let flow = blotch(x, (y + i32(t * 6.0)) & 15, 66u, 4) * 0.6 + rnd(x, y, 67u) * 0.25 + 0.15 * sin(world.x * 0.6 + world.z * 0.4 + t * 3.0);
    if (flow < 0.3) {
        return hex(0xb7350au);
    }
    if (flow < 0.55) {
        return hex(0xd65410u);
    }
    if (flow < 0.8) {
        return hex(0xf08a1cu);
    }
    return hex(0xffc848u);
}

// Model colours.
fn model_color(id: u32, x: i32, y: i32) -> vec4<f32> {
    let n = 0.92 + rnd(x, y, 68u + id) * 0.14;
    switch (id) {
        case 128u: { return vec4<f32>(hex(0x3e7d2cu) * n, 0.0); }
        case 129u: { return vec4<f32>(hex(0x4f9a36u) * n, 0.0); }
        case 130u: { return vec4<f32>(hex(0xffd83au) * n, 0.0); }
        case 131u: { return vec4<f32>(hex(0xd8231fu) * n, 0.0); }
        case 132u: { return vec4<f32>(hex(0x4c6ee8u) * n, 0.0); }
        case 133u: { return vec4<f32>(hex(0x58963au) * n, 0.0); }
        case 134u: { return vec4<f32>(hex(0x72b04au) * n, 0.0); }
        case 135u: { return vec4<f32>(hex(0x8a6332u) * n, 0.0); }
        case 136u: { return vec4<f32>(hex(0x7d5a32u) * n, 0.0); }
        case 137u: { return vec4<f32>(hex(0xffc63du), 3.0); }
        case 138u: { return vec4<f32>(hex(0xfff1b0u), 4.0); }
        case 139u: { return vec4<f32>(hex(0x3a1a12u) * n, 0.0); }
        case 140u: { return vec4<f32>(hex(0xe0a91cu) * n, 0.0); }
        case 141u: { return vec4<f32>(hex(0x2c46b0u) * n, 0.0); }
        case 142u: { return vec4<f32>(hex(0xf0a5a2u) * n, 0.0); }
        case 143u: { return vec4<f32>(hex(0xe88b8au) * n, 0.0); }
        case 144u: { return vec4<f32>(hex(0x1c1c24u) * n, 0.0); }
        case 145u: { return vec4<f32>(hex(0x4d8c3au) * n, 0.0); }
        case 146u: { return vec4<f32>(hex(0x2fa3a8u) * n, 0.0); }
        case 147u: { return vec4<f32>(hex(0x45428fu) * n, 0.0); }
        case 148u: { return vec4<f32>(hex(0x4fb83cu) * n, 0.0); }
        case 149u: { return vec4<f32>(hex(0x8fd67au) * n, 0.0); }
        case 150u: { return vec4<f32>(hex(0x1d3a17u) * n, 0.0); }
        case 151u: { return vec4<f32>(hex(0xe8e8e8u) * n, 0.0); }
        case 152u: { return vec4<f32>(hex(0x6e4a3au) * n, 0.0); }
        default: { return vec4<f32>(1.0, 0.0, 1.0, 0.0); }
    }
}

// A block texture's colour at texel (x, y) (y 0 is the top row) and how much it glows by itself.
fn block_tex(id: u32, x: i32, y: i32, world: vec3<f32>) -> vec4<f32> {
    switch (id) {
        case 1u: { return vec4<f32>(t_stone(x, y), 0.0); }
        case 2u: { return vec4<f32>(t_dirt(x, y), 0.0); }
        case 3u: { return vec4<f32>(t_grass_top(x, y) * grass_tint(world), 0.0); }
        case 4u: {
            let c = t_grass_side(x, y, false);
            let hang = 2 + i32(rnd(x, 0, 8u) * 2.0) + select(0, 1, rnd(x, 1, 8u) > 0.8);
            return vec4<f32>(select(c, c * grass_tint(world), y < hang), 0.0);
        }
        case 5u: { return vec4<f32>(t_cobble(x, y, false), 0.0); }
        case 6u: { return vec4<f32>(t_planks(x, y, 0xc49c62u, 0xb38d55u, 0x9c7a47u, 0x7d6038u), 0.0); }
        case 7u: { return vec4<f32>(t_bedrock(x, y), 0.0); }
        case 8u: { return vec4<f32>(t_water(x, y, world), 0.0); }
        case 9u: { return vec4<f32>(t_lava(x, y, world), 2.2); }
        case 10u: { return vec4<f32>(t_sand(x, y), 0.0); }
        case 11u: { return vec4<f32>(t_gravel(x, y), 0.0); }
        case 12u: { return vec4<f32>(t_ore(x, y, 0xd9a91fu, 0xfcee4bu), 0.0); }
        case 13u: { return vec4<f32>(t_ore(x, y, 0xb4886bu, 0xe4c0a2u), 0.0); }
        case 14u: { return vec4<f32>(t_ore(x, y, 0x2a2a2au, 0x494949u), 0.0); }
        case 15u: { return vec4<f32>(t_ore(x, y, 0x2bc3cbu, 0x8ff8f8u), 0.0); }
        case 16u: { return vec4<f32>(t_bark(x, y, 0x4a3a24u, 0x5f4a2eu, 0x6e5636u, 0x846a45u), 0.0); }
        case 17u: { return vec4<f32>(t_rings(x, y, 0x5a4529u, 0xb3925cu, 0x9a7a49u), 0.0); }
        case 18u: { return vec4<f32>(t_leaves(x, y, 0x3c7424u, 0x4b862du, 0x5c9a38u, 0x24480fu) * grass_tint(world), 0.0); }
        case 19u: { return vec4<f32>(t_birch_bark(x, y), 0.0); }
        case 20u: { return vec4<f32>(t_rings(x, y, 0xd6d5cdu, 0xd2c08au, 0xb9a771u), 0.0); }
        case 21u: { return vec4<f32>(t_leaves(x, y, 0x5b8a33u, 0x6a9a3cu, 0x7aaa47u, 0x3a5c1du), 0.0); }
        case 22u: { return vec4<f32>(t_bark(x, y, 0x2e2113u, 0x3d2c1au, 0x4b3721u, 0x5c452bu), 0.0); }
        case 23u: { return vec4<f32>(t_rings(x, y, 0x3a2918u, 0x7a5c36u, 0x5f472au), 0.0); }
        case 24u: { return vec4<f32>(t_leaves(x, y, 0x2b4a2bu, 0x335634u, 0x3c623cu, 0x18301au), 0.0); }
        case 25u: { return vec4<f32>(t_glass(x, y), 0.0); }
        case 26u: { return vec4<f32>(t_sandstone(x, y, false), 0.0); }
        case 27u: { return vec4<f32>(t_sandstone(x, y, true), 0.0); }
        case 28u: { return vec4<f32>(t_bricks(x, y), 0.0); }
        case 29u: { return vec4<f32>(t_stone_bricks(x, y), 0.0); }
        case 30u: { return vec4<f32>(t_table_top(x, y), 0.0); }
        case 31u: { return vec4<f32>(t_table_side(x, y, false), 0.0); }
        case 32u: { return vec4<f32>(t_table_side(x, y, true), 0.0); }
        case 33u: { return vec4<f32>(t_snow(x, y), 0.0); }
        case 34u: { return vec4<f32>(t_grass_side(x, y, true), 0.0); }
        case 35u: { return vec4<f32>(t_ice(x, y), 0.0); }
        case 36u: { return vec4<f32>(t_cactus(x, y, false), 0.0); }
        case 37u: { return vec4<f32>(t_cactus(x, y, true), 0.0); }
        case 38u: { return vec4<f32>(t_clay(x, y), 0.0); }
        case 39u: { return vec4<f32>(t_obsidian(x, y), 0.0); }
        case 40u: { return vec4<f32>(t_bookshelf(x, y), 0.0); }
        case 41u: { return vec4<f32>(t_planks(x, y, 0xd7c48eu, 0xc8b47bu, 0xb39f69u, 0x9a8858u), 0.0); }
        case 42u: { return vec4<f32>(t_planks(x, y, 0x82603au, 0x735531u, 0x634829u, 0x4f3a21u), 0.0); }
        case 43u: { return vec4<f32>(t_metal(x, y, 0xf4f4f4u, 0xd8d8d8u, 0xa8a8a8u), 0.0); }
        case 44u: { return vec4<f32>(t_metal(x, y, 0xfff7a0u, 0xf5d93cu, 0xc79c16u), 0.0); }
        case 45u: { return vec4<f32>(t_metal(x, y, 0xb8fff8u, 0x5fe3dau, 0x2aa59eu), 0.0); }
        case 46u: { return vec4<f32>(t_metal(x, y, 0x3a3a3au, 0x1c1c1cu, 0x0c0c0cu), 0.0); }
        case 47u: { return vec4<f32>(t_cobble(x, y, true), 0.0); }
        case 48u: { return vec4<f32>(t_tnt(x, y, 0), 0.0); }
        case 49u: { return vec4<f32>(t_tnt(x, y, 1), 0.0); }
        case 50u: { return vec4<f32>(t_tnt(x, y, 2), 0.0); }
        case 51u: { return t_furnace_front(x, y, false); }
        case 52u: { return t_furnace_front(x, y, true); }
        case 53u: { return vec4<f32>(t_cobble(x, y, false) * 1.05, 0.0); }
        case 54u: { return vec4<f32>(t_smooth_stone(x, y), 0.0); }
        case 55u: { return vec4<f32>(t_chest(x, y, 0), 0.0); }
        case 56u: { return vec4<f32>(t_chest(x, y, 1), 0.0); }
        case 57u: { return vec4<f32>(t_chest(x, y, 2), 0.0); }
        default: { return vec4<f32>(1.0, 0.0, 1.0, 0.0); }
    }
}

// The texel a point of a face falls on, and the face's kind (0 side, 1 top, 2 bottom); `cell` is the block's corner.
fn texel_of(p: vec3<f32>, n: vec3<f32>) -> vec3<i32> {
    let cell = floor(p - n * 0.001);
    let l = clamp(p - cell, vec3<f32>(0.0), vec3<f32>(0.9999));
    var u = 0.0;
    var v = 0.0;
    var face = 0;
    if (abs(n.y) > 0.5) {
        u = l.x;
        v = l.z;
        face = select(2, 1, n.y > 0.0);
    } else if (abs(n.x) > 0.5) {
        u = select(l.z, 1.0 - l.z, n.x > 0.0);
        v = 1.0 - l.y;
    } else {
        u = select(1.0 - l.x, l.x, n.z > 0.0);
        v = 1.0 - l.y;
    }
    return vec3<i32>(i32(u * 16.0), i32(v * 16.0), face);
}

fn surface(s: SurfaceIn) -> vec4<f32> {
    let id = u32(round(s.vertex.x * 255.0));
    let n = s.n;
    let t = texel_of(s.p, n);
    var c: vec4<f32>;
    if (id >= 128u) {
        c = model_color(id, t.x, t.y);
    } else {
        c = block_tex(id, t.x, t.y, s.world);
    }
    var rgb = c.rgb;
    // The aimed-at block: a dark edge round each face, and cracks as it is mined.
    let aim = game_param(0);
    if (game_param(1).x > 0.5) {
        let cell = floor(s.world - s.world_n * 0.001);
        if (all(abs(cell - aim.xyz) < vec3<f32>(0.5))) {
            if (t.x == 0 || t.y == 0 || t.x == 15 || t.y == 15) {
                rgb = rgb * 0.35;
            }
            let stage = aim.w;
            if (stage > 0.0) {
                let crack = rnd(t.x, t.y, 90u) * 0.5 + blotch(t.x, t.y, 91u, 4) * 0.5;
                if ((crack < stage / 10.0 * 0.62 && (t.x + t.y * 3) % 2 == 0) || crack < stage / 10.0 * 0.4) {
                    rgb = rgb * 0.22;
                }
            }
        }
    }
    // Light: the sky's (lit further by the sun and the sky in the engine) and the blocks' own (a glow that shows where the sky does not).
    let day = clamp(game_param(2).x, 0.0, 1.0);
    let sky = clamp(s.vertex.y, 0.0, 1.0);
    let blk = clamp(s.vertex.z, 0.0, 1.0);
    let skyb = 0.03 + 0.97 * pow(sky, 1.5);
    let face_k = select(select(0.78, 0.55, n.y < -0.5), 1.0, n.y > 0.5) * select(1.0, 0.9, abs(n.x) > 0.5);
    let torch = max(pow(blk, 1.35) - skyb * (0.25 + 0.75 * day), 0.0) * face_k;
    let warm = mix(vec3<f32>(1.0), vec3<f32>(1.0, 0.83, 0.62), clamp(torch / (torch + skyb * day + 0.05), 0.0, 1.0));
    rgb = rgb * warm;
    let albedo = rgb * skyb;
    // glow = albedo * a: the torch's share, and what shines by itself (lava, flame).
    let a = (torch * 0.95 + c.a * 0.5) / skyb;
    return vec4<f32>(albedo, a);
}
