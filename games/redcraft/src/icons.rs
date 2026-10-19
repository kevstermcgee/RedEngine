//! Pictures for the inventory and the hotbar: every block as a little isometric cube in its own textures, the plants and the torch from the
//! front, and the items as pixel sprites. The block textures are the CPU twin of `content/shaders/blocks.wgsl` (the same hashes, the same
//! colours, function for function): change one, change the other.

use crate::block::{tex, Block, Shape};
use crate::item::{id, Item};

// ---- The textures (twin of blocks.wgsl) ----------------------------------------------------------------------------------------------

fn hsh(a: u32) -> u32 {
    let mut x = a;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

fn hash3u(x: u32, y: u32, z: u32) -> u32 {
    hsh(x.wrapping_mul(73_856_093) ^ hsh(y.wrapping_mul(19_349_663) ^ hsh(z.wrapping_mul(83_492_791))))
}

fn rnd(x: i32, y: i32, k: u32) -> f32 {
    (hash3u((x & 15) as u32, (y & 15) as u32, k) & 0xffff) as f32 / 65535.0
}

type C = [f32; 3];

fn hex(v: u32) -> C {
    let f = |s: u32| (((v >> s) & 255) as f32 / 255.0).powf(2.2);
    [f(16), f(8), f(0)]
}

fn mul(c: C, k: f32) -> C {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn mulv(c: C, k: C) -> C {
    [c[0] * k[0], c[1] * k[1], c[2] * k[2]]
}

fn mix(a: C, b: C, t: f32) -> C {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn blotch(x: i32, y: i32, k: u32, cell: i32) -> f32 {
    let n = 16 / cell;
    let (gx, gy) = (x / cell, y / cell);
    let fx = (x - gx * cell) as f32 / cell as f32;
    let fy = (y - gy * cell) as f32 / cell as f32;
    let a = rnd(gx % n, gy % n, k);
    let b = rnd((gx + 1) % n, gy % n, k);
    let c = rnd(gx % n, (gy + 1) % n, k);
    let d = rnd((gx + 1) % n, (gy + 1) % n, k);
    lerp(lerp(a, b, fx), lerp(c, d, fx), fy)
}

fn shade4(a: u32, b: u32, c: u32, d: u32, v: f32) -> C {
    hex(if v < 0.25 {
        a
    } else if v < 0.5 {
        b
    } else if v < 0.75 {
        c
    } else {
        d
    })
}

fn cells(x: i32, y: i32, k: u32) -> [f32; 3] {
    let (mut d1, mut d2, mut id) = (99.0f32, 99.0f32, 0.0f32);
    let (gx, gy) = (x / 4, y / 4);
    for j in -1..=1 {
        for i in -1..=1 {
            let (cx, cy) = (gx + i, gy + j);
            let (wx, wy) = ((cx + 4) % 4, (cy + 4) % 4);
            let fx = (cx * 4) as f32 + 0.5 + rnd(wx, wy, k) * 3.0;
            let fy = (cy * 4) as f32 + 0.5 + rnd(wx, wy, k + 7) * 3.0;
            let d = ((x as f32 + 0.5 - fx).powi(2) + (y as f32 + 0.5 - fy).powi(2)).sqrt();
            if d < d1 {
                d2 = d1;
                d1 = d;
                id = rnd(wx, wy, k + 13);
            } else if d < d2 {
                d2 = d;
            }
        }
    }
    [d1, d2 - d1, id]
}

fn t_stone(x: i32, y: i32) -> C {
    shade4(0x6b6b6b, 0x777777, 0x828282, 0x8e8e8e, rnd(x, y, 1) * 0.55 + blotch(x, y, 2, 4) * 0.45)
}

fn t_dirt(x: i32, y: i32) -> C {
    let c = shade4(0x5e4230, 0x77553b, 0x866144, 0x93704f, rnd(x, y, 3) * 0.7 + blotch(x, y, 4, 8) * 0.3);
    if rnd(x, y, 5) > 0.93 {
        hex(0xa1876a)
    } else {
        c
    }
}

fn grass_tint() -> C {
    let w = 0.5 + 0.5 * (1.3f32).sin() * (-0.4f32).sin();
    mix([0.92, 1.04, 0.86], [1.08, 0.98, 0.84], w)
}

fn t_grass_top(x: i32, y: i32) -> C {
    shade4(0x4f8a32, 0x5d9c3c, 0x69a845, 0x77b551, rnd(x, y, 6) * 0.75 + blotch(x, y, 7, 4) * 0.25)
}

fn hang(x: i32) -> i32 {
    2 + (rnd(x, 0, 8) * 2.0) as i32 + if rnd(x, 1, 8) > 0.8 { 1 } else { 0 }
}

fn t_grass_side(x: i32, y: i32, snow: bool) -> C {
    if y < hang(x) {
        if snow {
            return shade4(0xdcebef, 0xe8f3f5, 0xf2fafb, 0xffffff, rnd(x, y, 9));
        }
        return t_grass_top(x, y);
    }
    t_dirt(x, y)
}

fn t_cobble(x: i32, y: i32, moss: bool) -> C {
    let c = cells(x, y, 10);
    if c[1] < 0.7 {
        return hex(0x4a4a4a);
    }
    let edge = ((c[1] - 0.7) / 1.6).clamp(0.0, 1.0);
    let mut col = mix(hex(0x6a6a6a), hex(0x8d8d8d), c[2] * 0.6 + rnd(x, y, 11) * 0.4);
    col = mul(col, lerp(0.82, 1.06, edge));
    if moss && blotch(x, y, 12, 4) > 0.52 && rnd(x, y, 13) > 0.25 {
        col = shade4(0x3f6b2b, 0x4c7a31, 0x58883a, 0x63963f, rnd(x, y, 14));
    }
    col
}

fn t_planks(x: i32, y: i32, light: u32, mid: u32, dark: u32, seam: u32) -> C {
    let board = y / 4;
    if y % 4 == 3 {
        return hex(seam);
    }
    let joint = (rnd(board, 0, 15) * 16.0) as i32;
    if x == joint {
        return hex(seam);
    }
    let grain = rnd(x / 3, y, 16 + board as u32) * 0.6 + rnd(x, y, 17) * 0.4;
    if grain < 0.2 {
        hex(dark)
    } else if grain > 0.82 {
        hex(light)
    } else {
        hex(mid)
    }
}

fn t_bedrock(x: i32, y: i32) -> C {
    shade4(0x1f1f1f, 0x3a3a3a, 0x555555, 0x7a7a7a, rnd(x, y, 18) * 0.6 + blotch(x, y, 19, 4) * 0.4)
}

fn t_sand(x: i32, y: i32) -> C {
    shade4(0xcfc08e, 0xd8ca9a, 0xdfd3a6, 0xe7ddb5, rnd(x, y, 20) * 0.8 + blotch(x, y, 21, 8) * 0.2)
}

fn t_gravel(x: i32, y: i32) -> C {
    let c = cells(x, y, 22);
    if c[1] < 0.45 {
        return hex(0x4e4846);
    }
    let k = c[2];
    let col = if k < 0.25 {
        hex(0x9a9592)
    } else if k < 0.5 {
        hex(0x6a6463)
    } else if k < 0.65 {
        hex(0x8a7b6e)
    } else {
        hex(0x7f7a78)
    };
    mul(col, 0.88 + rnd(x, y, 23) * 0.22)
}

fn ore_spot(x: i32, y: i32) -> i32 {
    let centres = [(3, 3), (11, 2), (7, 8), (2, 12), (12, 12)];
    for (i, (cx, cy)) in centres.iter().enumerate() {
        let (dx, dy) = (x - cx, y - cy);
        let m = dx.abs() + dy.abs();
        if m == 0 {
            return 2;
        }
        if m == 1 && rnd(x, y, 24 + i as u32) > 0.2 {
            return 1;
        }
        if m == 2 && dx.abs() == 1 && rnd(x, y, 30) > 0.55 {
            return 1;
        }
    }
    0
}

fn t_ore(x: i32, y: i32, body: u32, hi: u32) -> C {
    match ore_spot(x, y) {
        2 => hex(hi),
        1 => hex(body),
        _ => t_stone(x, y),
    }
}

fn t_bark(x: i32, y: i32, a: u32, b: u32, c: u32, d: u32) -> C {
    shade4(a, b, c, d, rnd(x, y / 3, 31) * 0.6 + rnd(x, y, 32) * 0.4)
}

fn t_rings(x: i32, y: i32, bark: u32, inner: u32, ring: u32) -> C {
    let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
    if d > 6.9 {
        return hex(bark);
    }
    let r = (d / 2.2 + rnd(x, y, 33) * 0.15).fract();
    if r < 0.3 {
        hex(ring)
    } else {
        mul(hex(inner), 0.94 + rnd(x, y, 34) * 0.1)
    }
}

fn t_birch_bark(x: i32, y: i32) -> C {
    if rnd(x / 3, y, 35) > 0.78 && rnd(x, y, 36) > 0.25 {
        return hex(0x2e2c28);
    }
    shade4(0xc9c8c0, 0xd6d5cd, 0xe0dfd8, 0xeeede6, rnd(x, y, 37))
}

fn t_leaves(x: i32, y: i32, a: u32, b: u32, c: u32, hole: u32) -> C {
    let v = rnd(x, y, 38);
    if v < 0.16 {
        return hex(hole);
    }
    shade4(a, a, b, c, v * 0.8 + blotch(x, y, 39, 4) * 0.2)
}

fn t_glass(x: i32, y: i32) -> C {
    if x == 0 || y == 0 || x == 15 || y == 15 {
        return hex(0xf2fbff);
    }
    if (x - y == 2 && x > 2 && x < 7) || (x - y == -3 && x > 8 && x < 12) {
        return hex(0xffffff);
    }
    hex(0xa9cfdc)
}

fn t_sandstone(x: i32, y: i32, top: bool) -> C {
    if top {
        return shade4(0xd5c592, 0xdcce9e, 0xe1d4a6, 0xe6dbb1, rnd(x, y, 40) * 0.5 + 0.3);
    }
    if y < 3 {
        return shade4(0xdccf9f, 0xe2d6aa, 0xe7dcb3, 0xede3be, rnd(x, y, 41));
    }
    if y > 12 {
        return shade4(0xc6b682, 0xcdbd8b, 0xd3c493, 0xd9cb9c, rnd(x, y, 42));
    }
    let band = if (y + (rnd(x / 4, 0, 43) * 2.0) as i32) % 4 == 0 { 0.25 } else { 0.0 };
    shade4(0xcdbd8b, 0xd4c595, 0xdacc9e, 0xe0d3a8, rnd(x, y, 44) * 0.7 + band)
}

fn t_bricks(x: i32, y: i32) -> C {
    let row = y / 4;
    let off = if row % 2 == 1 { 4 } else { 0 };
    if y % 4 == 3 || (x + off) % 8 == 7 {
        return shade4(0x9b9389, 0xa79f95, 0xb0a99f, 0xbab3aa, rnd(x, y, 45));
    }
    shade4(0x8a4f3f, 0x96594a, 0xa16454, 0xab6d5c, rnd(x, y, 46) * 0.6 + rnd((x + off) / 8, row, 47) * 0.4)
}

fn t_stone_bricks(x: i32, y: i32) -> C {
    let row = y / 8;
    let off = if row % 2 == 1 { 8 } else { 0 };
    let lx = (x + off) % 16;
    let ly = y % 8;
    if ly == 7 || lx == 15 || lx == 7 {
        return hex(0x585858);
    }
    let c = shade4(0x707070, 0x7a7a7a, 0x838383, 0x8c8c8c, rnd(x, y, 48) * 0.7 + blotch(x, y, 49, 4) * 0.3);
    if ly == 0 || lx == 0 || lx == 8 {
        mul(c, 1.12)
    } else {
        c
    }
}

fn t_table_top(x: i32, y: i32) -> C {
    if x == 0 || y == 0 || x == 15 || y == 15 {
        return hex(0x5c3e1f);
    }
    if x == 5 || x == 10 || y == 5 || y == 10 {
        return hex(0x7a5a33);
    }
    t_planks(x, y, 0xc49c62, 0xb38d55, 0x9c7a47, 0x8c6c3e)
}

fn t_table_side(x: i32, y: i32, front: bool) -> C {
    if y < 3 {
        return if y == 2 { hex(0x5c3e1f) } else { hex(0x6e4b26) };
    }
    if x == 0 || x == 15 {
        return hex(0x5c3e1f);
    }
    if front {
        if (3..=6).contains(&x) && (5..=11).contains(&y) {
            return if (x + y) % 2 == 0 { hex(0x8a8a8a) } else { hex(0xb8b8b8) };
        }
        if (4..=5).contains(&x) && (12..=13).contains(&y) {
            return hex(0x5c3e1f);
        }
        if (9..=12).contains(&x) && (5..=6).contains(&y) {
            return hex(0x9a9a9a);
        }
        if (10..=11).contains(&x) && (7..=13).contains(&y) {
            return hex(0x6e4b26);
        }
    } else if (6..=12).contains(&y) && (x == 4 || x == 8 || x == 12) {
        return hex(0x6e4b26);
    }
    t_planks(x, y, 0xb08a52, 0xa07c48, 0x8c6c3e, 0x7d5f35)
}

fn t_snow(x: i32, y: i32) -> C {
    shade4(0xe3eef0, 0xebf4f6, 0xf3f9fa, 0xffffff, rnd(x, y, 50) * 0.7 + 0.2)
}

fn t_ice(x: i32, y: i32) -> C {
    if (x + y * 2) % 11 == 0 && rnd(x, y, 51) > 0.3 {
        return hex(0xd8e6ff);
    }
    shade4(0x7ea2f2, 0x86abf8, 0x8fb3fe, 0x99bbff, rnd(x, y, 52) * 0.5 + blotch(x, y, 53, 8) * 0.5)
}

fn t_cactus(x: i32, y: i32, top: bool) -> C {
    if top {
        let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
        if d < 2.0 {
            return hex(0x8bc34a);
        }
        return if (x + y) % 3 == 0 { hex(0x0d6a1b) } else { hex(0x147d24) };
    }
    if x % 4 == 1 && y % 3 == 1 {
        return hex(0xc8e6a0);
    }
    if x % 4 == 0 {
        return hex(0x0b5e18);
    }
    shade4(0x107421, 0x158028, 0x1a8a2e, 0x1f9434, rnd(x, y, 54))
}

fn t_clay(x: i32, y: i32) -> C {
    shade4(0x969ca6, 0x9ea4ae, 0xa5abb5, 0xaeb3bc, rnd(x, y, 55) * 0.6 + blotch(x, y, 56, 8) * 0.4)
}

fn t_obsidian(x: i32, y: i32) -> C {
    if rnd(x, y, 59) > 0.94 {
        return hex(0x4b3577);
    }
    shade4(0x0f0d16, 0x16121f, 0x1e1829, 0x2a2138, rnd(x, y, 57) * 0.5 + blotch(x, y, 58, 4) * 0.5)
}

fn t_bookshelf(x: i32, y: i32) -> C {
    if y == 0 || y == 15 || y == 7 || y == 8 {
        return t_planks(x, y, 0xc49c62, 0xb38d55, 0x9c7a47, 0x8c6c3e);
    }
    if x == 0 || x == 15 {
        return hex(0x6e4b26);
    }
    let shelf = if y > 7 { 1 } else { 0 };
    let spine = x / 2;
    let k = rnd(spine, shelf, 60);
    let top = if y > 7 { 9 } else { 1 } + (rnd(spine, shelf, 61) * 2.0) as i32;
    if y < top || k > 0.92 {
        return hex(0x3a2512);
    }
    let c = if k < 0.2 {
        hex(0x2f4f8a)
    } else if k < 0.4 {
        hex(0x3d6b35)
    } else if k < 0.55 {
        hex(0x8a6a3a)
    } else if k < 0.7 {
        hex(0x5a3d6b)
    } else {
        hex(0x7a2a24)
    };
    if x % 2 == 1 {
        mul(c, 0.8)
    } else {
        c
    }
}

fn t_metal(x: i32, y: i32, light: u32, mid: u32, dark: u32) -> C {
    if x == 0 || y == 0 {
        return hex(light);
    }
    if x == 15 || y == 15 {
        return hex(dark);
    }
    if (x == 1 && y < 15) || (y == 1 && x < 15) {
        return mix(hex(light), hex(mid), 0.5);
    }
    mul(hex(mid), 0.95 + rnd(x, y, 62) * 0.08)
}

fn t_tnt(x: i32, y: i32, face: i32) -> C {
    if face == 0 {
        if (5..=10).contains(&y) {
            let letters = [0xe97u32, 0x4d2, 0x4b2, 0x492];
            if (6..=9).contains(&y) && (2..=13).contains(&x) && (letters[(y - 6) as usize] >> (13 - x) as u32) & 1 == 1 {
                return hex(0x1a1a1a);
            }
            return hex(0xe8e8e8);
        }
        return if x % 4 == 0 { hex(0xa11c12) } else { hex(0xc9281a) };
    }
    if face == 1 && (x - 7).abs() <= 1 && (y - 7).abs() <= 1 {
        return if x == 7 && y == 7 { hex(0x2a2a2a) } else { hex(0x5b5b5b) };
    }
    let d = (((x % 8) as f32 - 3.5).powi(2) + ((y % 8) as f32 - 3.5).powi(2)).sqrt();
    if d > 3.2 {
        hex(0x9e1b10)
    } else {
        hex(0xc9281a)
    }
}

fn t_smooth_stone(x: i32, y: i32) -> C {
    if x == 0 || y == 0 || x == 15 || y == 15 {
        return hex(0x5f5f5f);
    }
    shade4(0x7c7c7c, 0x828282, 0x888888, 0x8e8e8e, rnd(x, y, 63))
}

fn t_furnace_front(x: i32, y: i32, lit: bool) -> C {
    if (4..=11).contains(&x) && (8..=13).contains(&y) {
        if lit {
            let heat = ((y - 8) as f32 / 5.0 + rnd(x, y, 64) * 0.35 * 0.5).clamp(0.0, 1.0);
            return mix(hex(0xb83a0a), hex(0xffd36a), heat);
        }
        return if y == 8 { hex(0x2a2a2a) } else { hex(0x1c1c1c) };
    }
    if (3..=5).contains(&y) && (3..=12).contains(&x) {
        return hex(0x4a4a4a);
    }
    mul(t_cobble(x, y, false), 1.05)
}

fn t_chest(x: i32, y: i32, which: i32) -> C {
    if x == 0 || x == 15 || y == 0 || y == 15 {
        return hex(0x3d2814);
    }
    if which == 0 {
        if y == 5 {
            return hex(0x3d2814);
        }
        if (7..=8).contains(&x) && (4..=7).contains(&y) {
            return if y == 7 { hex(0x8f8f8f) } else { hex(0xc8c8c8) };
        }
    }
    if which == 1 && y == 5 {
        return hex(0x3d2814);
    }
    t_planks(x, y, 0xb3772e, 0xa36a26, 0x8d5a1f, 0x7d4f1b)
}

fn t_water(x: i32, y: i32) -> C {
    let w = (x as f32 * 0.8).sin() + (y as f32 * 0.9).sin();
    shade4(0x2a4fb8, 0x3159c4, 0x3a64ce, 0x4a75da, 0.5 + 0.25 * w + rnd(x, y, 65) * 0.15)
}

fn t_lava(x: i32, y: i32) -> C {
    let flow = blotch(x, y & 15, 66, 4) * 0.6 + rnd(x, y, 67) * 0.25;
    hex(if flow < 0.3 {
        0xb7350a
    } else if flow < 0.55 {
        0xd65410
    } else if flow < 0.8 {
        0xf08a1c
    } else {
        0xffc848
    })
}

/// A model colour (128 and up).
pub fn model_color(id: u8, x: i32, y: i32) -> C {
    let n = 0.92 + rnd(x, y, 68 + id as u32) * 0.14;
    let base = match id {
        128 => 0x3e7d2c,
        129 => 0x4f9a36,
        130 => 0xffd83a,
        131 => 0xd8231f,
        132 => 0x4c6ee8,
        133 => 0x58963a,
        134 => 0x72b04a,
        135 => 0x8a6332,
        136 => 0x7d5a32,
        137 => return hex(0xffc63d),
        138 => return hex(0xfff1b0),
        139 => 0x3a1a12,
        140 => 0xe0a91c,
        141 => 0x2c46b0,
        142 => 0xf0a5a2,
        143 => 0xe88b8a,
        144 => 0x1c1c24,
        145 => 0x4d8c3a,
        146 => 0x2fa3a8,
        147 => 0x45428f,
        148 => 0x4fb83c,
        149 => 0x8fd67a,
        150 => 0x1d3a17,
        151 => 0xe8e8e8,
        152 => 0x6e4a3a,
        _ => return [1.0, 0.0, 1.0],
    };
    mul(hex(base), n)
}

/// A block texture's colour (linear) at texel (x, y), y 0 the top row.
pub fn texel(id: u8, x: i32, y: i32) -> C {
    match id {
        1 => t_stone(x, y),
        2 => t_dirt(x, y),
        3 => mulv(t_grass_top(x, y), grass_tint()),
        4 => {
            let c = t_grass_side(x, y, false);
            if y < hang(x) {
                mulv(c, grass_tint())
            } else {
                c
            }
        }
        5 => t_cobble(x, y, false),
        6 => t_planks(x, y, 0xc49c62, 0xb38d55, 0x9c7a47, 0x7d6038),
        7 => t_bedrock(x, y),
        8 => t_water(x, y),
        9 => t_lava(x, y),
        10 => t_sand(x, y),
        11 => t_gravel(x, y),
        12 => t_ore(x, y, 0xd9a91f, 0xfcee4b),
        13 => t_ore(x, y, 0xb4886b, 0xe4c0a2),
        14 => t_ore(x, y, 0x2a2a2a, 0x494949),
        15 => t_ore(x, y, 0x2bc3cb, 0x8ff8f8),
        16 => t_bark(x, y, 0x4a3a24, 0x5f4a2e, 0x6e5636, 0x846a45),
        17 => t_rings(x, y, 0x5a4529, 0xb3925c, 0x9a7a49),
        18 => mulv(t_leaves(x, y, 0x3c7424, 0x4b862d, 0x5c9a38, 0x24480f), grass_tint()),
        19 => t_birch_bark(x, y),
        20 => t_rings(x, y, 0xd6d5cd, 0xd2c08a, 0xb9a771),
        21 => t_leaves(x, y, 0x5b8a33, 0x6a9a3c, 0x7aaa47, 0x3a5c1d),
        22 => t_bark(x, y, 0x2e2113, 0x3d2c1a, 0x4b3721, 0x5c452b),
        23 => t_rings(x, y, 0x3a2918, 0x7a5c36, 0x5f472a),
        24 => t_leaves(x, y, 0x2b4a2b, 0x335634, 0x3c623c, 0x18301a),
        25 => t_glass(x, y),
        26 => t_sandstone(x, y, false),
        27 => t_sandstone(x, y, true),
        28 => t_bricks(x, y),
        29 => t_stone_bricks(x, y),
        30 => t_table_top(x, y),
        31 => t_table_side(x, y, false),
        32 => t_table_side(x, y, true),
        33 => t_snow(x, y),
        34 => t_grass_side(x, y, true),
        35 => t_ice(x, y),
        36 => t_cactus(x, y, false),
        37 => t_cactus(x, y, true),
        38 => t_clay(x, y),
        39 => t_obsidian(x, y),
        40 => t_bookshelf(x, y),
        41 => t_planks(x, y, 0xd7c48e, 0xc8b47b, 0xb39f69, 0x9a8858),
        42 => t_planks(x, y, 0x82603a, 0x735531, 0x634829, 0x4f3a21),
        43 => t_metal(x, y, 0xf4f4f4, 0xd8d8d8, 0xa8a8a8),
        44 => t_metal(x, y, 0xfff7a0, 0xf5d93c, 0xc79c16),
        45 => t_metal(x, y, 0xb8fff8, 0x5fe3da, 0x2aa59e),
        46 => t_metal(x, y, 0x3a3a3a, 0x1c1c1c, 0x0c0c0c),
        47 => t_cobble(x, y, true),
        48 => t_tnt(x, y, 0),
        49 => t_tnt(x, y, 1),
        50 => t_tnt(x, y, 2),
        51 => t_furnace_front(x, y, false),
        52 => t_furnace_front(x, y, true),
        53 => mul(t_cobble(x, y, false), 1.05),
        54 => t_smooth_stone(x, y),
        55 => t_chest(x, y, 0),
        56 => t_chest(x, y, 1),
        57 => t_chest(x, y, 2),
        128..=255 => model_color(id, x, y),
        _ => [1.0, 0.0, 1.0],
    }
}

fn to_srgb(c: C) -> [u8; 3] {
    let f = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8;
    [f(c[0]), f(c[1]), f(c[2])]
}

/// A texture as 16 x 16 sRGB pixels (row by row).
pub fn texture(id: u8) -> Vec<[u8; 3]> {
    (0..256).map(|i| to_srgb(texel(id, i % 16, i / 16))).collect()
}

// ---- Icons ------------------------------------------------------------------------------------------------------------------------------

/// An icon: `size` x `size` RGBA (straight alpha).
pub struct Icon {
    pub size: u32,
    pub px: Vec<u8>,
}

impl Icon {
    fn new(size: u32) -> Icon {
        Icon { size, px: vec![0; (size * size * 4) as usize] }
    }

    fn put(&mut self, x: i32, y: i32, c: [u8; 3]) {
        if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
            return;
        }
        let i = ((y as u32 * self.size + x as u32) * 4) as usize;
        self.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    }
}

/// The picture of an item, `size` pixels square.
pub fn icon(item: Item, size: u32) -> Icon {
    if let Some(b) = item.block() {
        return match b.info().shape {
            Shape::Plant(p) => model_icon(crate::mesh::plant_parts(p), size),
            Shape::Torch(_) => model_icon(crate::mesh::torch_parts(), size),
            _ => cube_icon(b, size),
        };
    }
    sprite_icon(item, size)
}

/// A block as an isometric cube: its top, its south face (lit) and its east face (in shade). A block point (x, y, z), each 0..16, lands at
/// ((x - z) ax, (x + z) ay - y up) from the middle of the icon.
fn cube_icon(b: Block, size: u32) -> Icon {
    let mut ic = Icon::new(size);
    let k = size as f32 / 32.0 * 0.96;
    let (ax, ay, up) = (0.866 * k, 0.5 * k, k);
    let mid = size as f32 * 0.5;
    let top_t = b.face_tex(2);
    // A facing block shows its front on the left.
    let south_t = if b.facing().is_some() { b.info().tex[3] } else { b.face_tex(4) };
    let east_t = b.face_tex(0);
    for py in 0..size as i32 {
        for pxl in 0..size as i32 {
            let (sx, sy) = (pxl as f32 + 0.5 - mid, py as f32 + 0.5 - mid);
            // The top (y = 16).
            let (a, c) = (sx / ax, (sy + 16.0 * up) / ay);
            let (x, z) = ((a + c) * 0.5, (c - a) * 0.5);
            if (0.0..16.0).contains(&x) && (0.0..16.0).contains(&z) {
                ic.put(pxl, py, to_srgb(texel(top_t, x as i32, z as i32)));
                continue;
            }
            // The south face (z = 16): x across, y up.
            let x = sx / ax + 16.0;
            let y = ((x + 16.0) * ay - sy) / up;
            if (0.0..16.0).contains(&x) && (0.0..16.0).contains(&y) {
                ic.put(pxl, py, to_srgb(mul(texel(south_t, x as i32, (15.999 - y) as i32), 0.8)));
                continue;
            }
            // The east face (x = 16): z back, y up.
            let z = 16.0 - sx / ax;
            let y = ((16.0 + z) * ay - sy) / up;
            if (0.0..16.0).contains(&z) && (0.0..16.0).contains(&y) {
                ic.put(pxl, py, to_srgb(mul(texel(east_t, (15.999 - z) as i32, (15.999 - y) as i32), 0.6)));
            }
        }
    }
    ic
}

/// A plant or a torch, from the front: its boxes flattened.
fn model_icon(parts: &[([u8; 3], [u8; 3], u8)], size: u32) -> Icon {
    let mut ic = Icon::new(size);
    let s = size as f32 / 16.0;
    // Back to front so nearer boxes cover farther ones.
    let mut order: Vec<&([u8; 3], [u8; 3], u8)> = parts.iter().collect();
    order.sort_by_key(|p| p.0[2]);
    for (lo, hi, t) in order {
        for y in lo[1]..hi[1] {
            for x in lo[0]..hi[0] {
                let c = to_srgb(texel(*t, x as i32, y as i32));
                let (x0, y0) = ((x as f32 * s) as i32, ((15 - y) as f32 * s) as i32);
                let (x1, y1) = (((x + 1) as f32 * s) as i32, ((16 - y) as f32 * s) as i32);
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        ic.put(xx, yy, c);
                    }
                }
            }
        }
    }
    ic
}

/// Item sprites: 16 rows of 16 characters ('.' is clear) and the colour of each character.
struct Sprite {
    rows: [&'static str; 16],
    colors: &'static [(char, u32)],
}

const PICKAXE: [&str; 16] = [
    "................",
    "....kkkkkk......",
    "...kMMMMMMk.....",
    "....kkkmMMMk....",
    "........kmMMk...",
    ".......k%kkmMk..",
    "......k%#k.kmMk.",
    ".....k%#k...kMk.",
    "....k%#k....kmk.",
    "...k%#k......k..",
    "..k%#k..........",
    ".k%#k...........",
    ".k#k............",
    "..k.............",
    "................",
    "................",
];

const AXE: [&str; 16] = [
    "................",
    "......kkk.......",
    ".....kMMMk......",
    "....kMMMMMk.....",
    "....kmMMMk%k....",
    ".....kmMk%#k....",
    "......kk%#k.....",
    ".....k%#k.......",
    "....k%#k........",
    "...k%#k.........",
    "..k%#k..........",
    ".k%#k...........",
    ".k#k............",
    "..k.............",
    "................",
    "................",
];

const SHOVEL: [&str; 16] = [
    "................",
    "...........kkk..",
    "..........kMMMk.",
    ".........kMMMMk.",
    ".........kmMMk..",
    "........k%kkk...",
    ".......k%#k.....",
    "......k%#k......",
    ".....k%#k.......",
    "....k%#k........",
    "...k%#k.........",
    "..k%#k..........",
    ".k%#k...........",
    ".k#k............",
    "..k.............",
    "................",
];

const SWORD: [&str; 16] = [
    "................",
    "...........kkk..",
    "..........kMhk..",
    ".........kMhMk..",
    "........kMhMk...",
    ".......kMhMk....",
    "...kk.kMhMk.....",
    "...kmkMhMk......",
    "....kmMMk.......",
    "....k#mk........",
    "...k#kkmk.......",
    "..k#k..kk.......",
    ".k#k............",
    ".kk.............",
    "................",
    "................",
];

const STICK: [&str; 16] = [
    "................",
    "................",
    "................",
    "...........kk...",
    "..........k%k...",
    ".........k%#k...",
    "........k%#k....",
    ".......k%#k.....",
    "......k%#k......",
    ".....k%#k.......",
    "....k%#k........",
    "...k%#k.........",
    "...k#k..........",
    "....k...........",
    "................",
    "................",
];

const LUMP: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    "......kkk.......",
    "....kkmmmkk.....",
    "...kmmhmmmmk....",
    "..kmhmmmmhmk....",
    "..kmmmmmmmmmk...",
    "..kmmhmmmmmmk...",
    "...kmmmmhmmk....",
    "....kkmmmkk.....",
    "......kkk.......",
    "................",
    "................",
    "................",
];

const INGOT: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    "................",
    "........kkkkkk..",
    ".....kkkhhhhhk..",
    "...kkhhhMMMMMk..",
    "..khhMMMMMMMmk..",
    "..kMMMMMMMmmk...",
    "..kmmmmmmmkk....",
    "..kkkkkkkk......",
    "................",
    "................",
    "................",
    "................",
];

const GEM: [&str; 16] = [
    "................",
    "................",
    "................",
    ".....kkkkkk.....",
    "....khhMMMMk....",
    "...khhMMMMMMk...",
    "..kkkkkkkkkkkk..",
    "..kMMMMMMMMmmk..",
    "...kMMMMMMmmk...",
    "....kMMMMmmk....",
    ".....kMMmmk.....",
    "......kmmk......",
    ".......kk.......",
    "................",
    "................",
    "................",
];

const APPLE: [&str; 16] = [
    "................",
    "................",
    ".......k........",
    ".......k.kk.....",
    "....kkkkkGGk....",
    "...kRRRkRkk.....",
    "..kRhRRRRRRk....",
    "..kRhRRRRRRk....",
    "..kRRRRRRRrk....",
    "..kRRRRRRRrk....",
    "...kRRRRRrk.....",
    "....kkkkkk......",
    "................",
    "................",
    "................",
    "................",
];

const MEAT: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    ".....kkkkk......",
    "...kkMMMMMkk....",
    "..kMMhhMMMMMk...",
    "..kMhMMMMMMMMk..",
    ".kMMMMMMMMmMMk..",
    ".kMMMMMMmmMMk...",
    ".kWWMMMMMMMk....",
    "..kWWkkkkkk.....",
    "...kk...........",
    "................",
    "................",
    "................",
];

const BREAD: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    "................",
    "......kkkkk.....",
    "....kkMhMhMkk...",
    "..kkMhMMhMMhMk..",
    ".kMMMMMMMMMMMMk.",
    ".kmMMMMMMMMMMmk.",
    "..kmmmmmmmmmmk..",
    "...kkkkkkkkkk...",
    "................",
    "................",
    "................",
    "................",
];

const FLINT_AND_STEEL: [&str; 16] = [
    "................",
    "................",
    "..kkk...........",
    ".kSSSk..........",
    ".kSkkSk.........",
    ".kSk.kSk........",
    "..kk..kSk.......",
    "......kSk.......",
    ".......kk.kkk...",
    "..........kFFk..",
    ".........kFFFk..",
    ".........kFfFk..",
    "..........kFk...",
    "...........k....",
    "................",
    "................",
];

fn tier_colors(tier: u8) -> &'static [(char, u32)] {
    match tier {
        1 => &[('k', 0x3a2a14), ('M', 0x9a7a45), ('m', 0x6b5028), ('h', 0xc4a066), ('%', 0x8a6a3a), ('#', 0x5a4022)],
        2 => &[('k', 0x2a2a2a), ('M', 0x8e8e8e), ('m', 0x6a6a6a), ('h', 0xb4b4b4), ('%', 0x8a6a3a), ('#', 0x5a4022)],
        3 => &[('k', 0x3c3c3c), ('M', 0xd8d8d8), ('m', 0xa8a8a8), ('h', 0xffffff), ('%', 0x8a6a3a), ('#', 0x5a4022)],
        _ => &[('k', 0x0c3e3a), ('M', 0x33ebcb), ('m', 0x1aa59a), ('h', 0xb0fff2), ('%', 0x8a6a3a), ('#', 0x5a4022)],
    }
}

fn sprite_of(item: Item) -> Option<Sprite> {
    if let Some(t) = item.tool() {
        let rows = match t.kind {
            crate::item::ToolKind::Pickaxe => PICKAXE,
            crate::item::ToolKind::Axe => AXE,
            crate::item::ToolKind::Shovel => SHOVEL,
            crate::item::ToolKind::Sword => SWORD,
        };
        return Some(Sprite { rows, colors: tier_colors(t.tier) });
    }
    let (rows, colors): ([&str; 16], &'static [(char, u32)]) = match item.0 {
        id::STICK => (STICK, &[('k', 0x2e2013), ('%', 0x8a6a3a), ('#', 0x5a4022)]),
        id::COAL => (LUMP, &[('k', 0x0c0c0c), ('m', 0x2a2a2a), ('h', 0x555555)]),
        id::FLINT => (LUMP, &[('k', 0x111111), ('m', 0x3a3838), ('h', 0x6e6a68)]),
        id::IRON_INGOT => (INGOT, &[('k', 0x4a4a4a), ('M', 0xd8d8d8), ('m', 0xa8a8a8), ('h', 0xffffff)]),
        id::GOLD_INGOT => (INGOT, &[('k', 0x6a4a08), ('M', 0xf5d93c), ('m', 0xc79c16), ('h', 0xfff7a0)]),
        id::DIAMOND => (GEM, &[('k', 0x0c3e3a), ('M', 0x4aeddc), ('m', 0x1aa59a), ('h', 0xd0fff8)]),
        id::APPLE => (APPLE, &[('k', 0x3a0a08), ('R', 0xd8231f), ('r', 0x9a1512), ('h', 0xff8a7a), ('G', 0x3e8a2e)]),
        id::PORKCHOP => (MEAT, &[('k', 0x5a2a2a), ('M', 0xf0a0a0), ('m', 0xd07878), ('h', 0xffd0d0), ('W', 0xf8f0e8)]),
        id::COOKED_PORKCHOP => (MEAT, &[('k', 0x3a1c0c), ('M', 0xc07a3a), ('m', 0x8a5226), ('h', 0xe0a868), ('W', 0xf0e0c8)]),
        id::ROTTEN_FLESH => (MEAT, &[('k', 0x2a3a1a), ('M', 0x8a6a3a), ('m', 0x5a7a2a), ('h', 0xb08a5a), ('W', 0x7a8a4a)]),
        id::BREAD => (BREAD, &[('k', 0x4a2a0a), ('M', 0xc08a3a), ('m', 0x8a5a1a), ('h', 0xe8c078)]),
        id::FLINT_AND_STEEL => (FLINT_AND_STEEL, &[('k', 0x222222), ('S', 0xb8b8b8), ('F', 0x4a4646), ('f', 0x8a8682)]),
        _ => return None,
    };
    Some(Sprite { rows, colors })
}

fn sprite_icon(item: Item, size: u32) -> Icon {
    let mut ic = Icon::new(size);
    let Some(sp) = sprite_of(item) else {
        // Unknown: a magenta square, so it cannot go unnoticed.
        for y in 0..size as i32 {
            for x in 0..size as i32 {
                ic.put(x, y, [255, 0, 255]);
            }
        }
        return ic;
    };
    let s = size as f32 / 16.0;
    for (y, row) in sp.rows.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            if ch == '.' {
                continue;
            }
            let c = sp.colors.iter().find(|(k, _)| *k == ch).map_or(0xff00ff, |(_, v)| *v);
            let rgb = [(c >> 16) as u8, (c >> 8) as u8, c as u8];
            let (x0, y0) = ((x as f32 * s) as i32, (y as f32 * s) as i32);
            let (x1, y1) = (((x + 1) as f32 * s) as i32, ((y + 1) as f32 * s) as i32);
            for yy in y0..y1 {
                for xx in x0..x1 {
                    ic.put(xx, yy, rgb);
                }
            }
        }
    }
    ic
}

/// Every item's icon on one sheet (for looking at them): rows of 12, `size` each.
pub fn sheet(size: u32) -> (u32, u32, Vec<u8>) {
    let items = Item::all();
    let cols = 12u32;
    let rows = (items.len() as u32).div_ceil(cols);
    let (w, h) = (cols * (size + 4), rows * (size + 4));
    let mut px = vec![0u8; (w * h * 4) as usize];
    for p in px.chunks_mut(4) {
        p.copy_from_slice(&[139, 139, 139, 255]);
    }
    for (i, it) in items.iter().enumerate() {
        let ic = icon(*it, size);
        let (ox, oy) = ((i as u32 % cols) * (size + 4) + 2, (i as u32 / cols) * (size + 4) + 2);
        for y in 0..size {
            for x in 0..size {
                let s = ((y * size + x) * 4) as usize;
                if ic.px[s + 3] > 0 {
                    let d = (((oy + y) * w + ox + x) * 4) as usize;
                    px[d..d + 4].copy_from_slice(&ic.px[s..s + 4]);
                }
            }
        }
    }
    (w, h, px)
}

/// Whether every texture number the blocks use has a picture (not the magenta of an unknown one).
pub fn every_texture_is_known() -> Result<(), u8> {
    for t in 1..=tex::LAST {
        if texel(t, 3, 3) == [1.0, 0.0, 1.0] {
            return Err(t);
        }
    }
    for t in 128..=tex::LAST_FLAT {
        if texel(t, 3, 3) == [1.0, 0.0, 1.0] {
            return Err(t);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_texture_and_every_item_has_a_picture() {
        assert_eq!(every_texture_is_known(), Ok(()));
        for it in Item::all() {
            let ic = icon(it, 32);
            let opaque = ic.px.chunks(4).filter(|p| p[3] > 0).count();
            assert!(opaque > 40, "{} has {opaque} pixels", it.name());
            assert!(!ic.px.chunks(4).any(|p| p == [255, 0, 255, 255]), "{} is magenta", it.name());
        }
    }

    #[test]
    fn the_hashes_match_the_shaders_numbers() {
        // Spot values computed by hand from the WGSL (u32 wrapping arithmetic): the twin must agree bit for bit.
        assert_eq!(hsh(0), 0);
        assert_eq!(hsh(1), {
            let mut x: u32 = 1;
            x ^= x >> 16;
            x = x.wrapping_mul(0x7feb352d);
            x ^= x >> 15;
            x = x.wrapping_mul(0x846ca68b);
            x ^ (x >> 16)
        });
        let r = rnd(3, 4, 1);
        assert!((0.0..=1.0).contains(&r));
    }
}
