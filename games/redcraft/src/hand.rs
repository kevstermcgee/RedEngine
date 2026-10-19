//! What the player holds, drawn at the lower right of the view as the original does: a block as a small cube in its own textures, a plant or a
//! torch as its little model, any other item as its pixel sprite pressed into a slab one pixel thick. The meshes are lit with the light where the
//! player stands (the game rebuilds them when that changes), and the whole swings down and back when the player mines or places.

use crate::block::Shape;
use crate::item::Item;
use crate::mesh::{boxes, FACES};
use glam::{Mat4, Vec3};
use red_engine2::mesh::{Mesh, Vertex};

/// The mesh of an item held in hand, in a unit cell (0..1), and whether it is painted by the block shader (its vertex colours are texture numbers
/// and light) or carries plain colours.
pub fn item_mesh(item: Item, sky: f32, glow: f32) -> (Mesh, bool) {
    let mut m = Mesh::default();
    if let Some(b) = item.block() {
        match b.info().shape {
            Shape::Plant(p) => boxes(&mut m, crate::mesh::plant_parts(p), Mat4::IDENTITY, sky, glow, 0.0),
            Shape::Torch(_) => boxes(&mut m, crate::mesh::torch_parts(), Mat4::IDENTITY, sky, glow, 0.0),
            _ => {
                for (d, f) in FACES.iter().enumerate() {
                    let n = crate::block::DIRS[d];
                    let t = b.face_tex(d);
                    let base = m.vertices.len() as u32;
                    for c in f {
                        m.vertices.push(Vertex { pos: *c, normal: [n[0] as f32, n[1] as f32, n[2] as f32], color: [t as f32 / 255.0, sky, glow], sway: 0.0 });
                    }
                    m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
                }
            }
        }
        return (m, true);
    }
    // A sprite pressed into a slab: one small box per pixel, faces between pixels left out.
    let ic = crate::icons::icon(item, 16);
    let opaque = |x: i32, y: i32| (0..16).contains(&x) && (0..16).contains(&y) && ic.px[((y * 16 + x) * 4 + 3) as usize] > 0;
    let light = 0.25 + 0.75 * sky.max(glow);
    for y in 0..16 {
        for x in 0..16 {
            if !opaque(x, y) {
                continue;
            }
            let i = ((y * 16 + x) * 4) as usize;
            let srgb = [ic.px[i], ic.px[i + 1], ic.px[i + 2]];
            let lin = srgb.map(|c| (c as f32 / 255.0).powf(2.2) * light);
            let (x0, y0) = (x as f32 / 16.0, 1.0 - (y + 1) as f32 / 16.0);
            for (d, f) in FACES.iter().enumerate() {
                let n = crate::block::DIRS[d];
                // Neighbours in the picture hide the faces between them; the front and back always show.
                if n[2] == 0 && opaque(x + n[0], y - n[1]) {
                    continue;
                }
                let base = m.vertices.len() as u32;
                for c in f {
                    let p = [x0 + c[0] / 16.0, y0 + c[1] / 16.0, 0.47 + c[2] / 16.0];
                    m.vertices.push(Vertex { pos: p, normal: [n[0] as f32, n[1] as f32, n[2] as f32], color: lin, sway: 0.0 });
                }
                m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
    }
    (m, false)
}

/// Where the held item is drawn: `eye` and the view's yaw and pitch, `swing` 1 just after a blow falling to 0, `bob` the walk's phase, `block`
/// whether it is a block (held turned to show its corner) or a flat item (held up like a tool).
pub fn hand_transform(eye: Vec3, yaw: f32, pitch: f32, swing: f32, bob: f32, block: bool) -> Mat4 {
    let fwd = crate::tool::look_dir(yaw, pitch);
    let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let up = right.cross(fwd).normalize();
    let view = Mat4::from_cols(right.extend(0.0), up.extend(0.0), (-fwd).extend(0.0), eye.extend(1.0));
    let s = (1.0 - swing).clamp(0.0, 1.0);
    let arc = (s * std::f32::consts::PI).sin() * if swing > 0.0 { 1.0 } else { 0.0 };
    let walk = Vec3::new((bob).sin() * 0.008, -(bob * 2.0).sin().abs() * 0.008, 0.0);
    let offset = Vec3::new(0.2 - arc * 0.07, -0.19 - arc * 0.05, -0.32 + arc * 0.04) + walk;
    if block {
        view * Mat4::from_translation(offset)
            * Mat4::from_rotation_x(-arc * 0.9)
            * Mat4::from_rotation_y(0.78)
            * Mat4::from_rotation_x(0.12)
            * Mat4::from_scale(Vec3::splat(0.16))
            * Mat4::from_translation(Vec3::splat(-0.5))
    } else {
        view * Mat4::from_translation(offset + Vec3::new(0.01, 0.03, 0.0))
            * Mat4::from_rotation_x(-arc * 1.1)
            * Mat4::from_rotation_y(-1.2)
            * Mat4::from_rotation_z(0.35)
            * Mat4::from_scale(Vec3::splat(0.26))
            * Mat4::from_translation(Vec3::new(-0.5, -0.5, -0.5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;
    use crate::item::id;

    #[test]
    fn a_block_is_a_painted_cube_and_a_tool_a_plain_slab() {
        let (m, painted) = item_mesh(Item::of(Block::GRASS), 1.0, 0.0);
        assert!(painted && m.vertices.len() == 24);
        let (m, painted) = item_mesh(Item(id::DIAMOND_PICKAXE), 1.0, 0.0);
        assert!(!painted && m.vertices.len() > 100 && m.vertices.len() % 4 == 0);
        let (m, _) = item_mesh(Item::of(Block::TORCH), 0.5, 1.0);
        assert!(!m.vertices.is_empty());
    }

    #[test]
    fn the_hand_sits_in_front_of_the_eye_to_the_right_and_below() {
        let eye = Vec3::new(10.0, 70.0, 10.0);
        let t = hand_transform(eye, 0.0, 0.0, 0.0, 0.0, true);
        let c = t.transform_point3(Vec3::splat(0.5));
        assert!(c.z < eye.z - 0.2 && c.x > eye.x + 0.1 && c.y < eye.y - 0.1, "{c}");
    }
}
