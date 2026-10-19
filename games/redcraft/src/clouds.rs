//! The clouds: flat blocky slabs high over the world (12 m cells, 4 m thick, at y 132), drifting slowly west to east, drawn see-through in white
//! and lit by the sun like everything else. Which cells hold cloud is a noise of the cell, so the sky is the same wherever the player goes.

use crate::noise::Noise;
use glam::Vec3;
use red_engine2::mesh::{Mesh, Vertex};

/// A cell's side and the slab's height, metres; how high the clouds float; how fast they drift (m/s, toward +X).
pub const CELL: f32 = 12.0;
pub const THICK: f32 = 4.0;
pub const HEIGHT: f32 = 132.0;
pub const DRIFT: f32 = 0.9;
/// Cells from the player to the edge of the cloud field.
pub const RADIUS: i32 = 20;

/// Whether cloud cell (i, j) holds cloud.
pub fn cloudy(noise: &Noise, i: i32, j: i32) -> bool {
    noise.fbm2(i as f64, j as f64, 7.0, 3) > 0.08
}

/// The cloud field's mesh around cell (ci, cj) (cloud space), in metres from the corner of cell (ci - RADIUS, cj - RADIUS).
pub fn mesh(noise: &Noise, ci: i32, cj: i32) -> Mesh {
    let mut m = Mesh::default();
    let n = 2 * RADIUS;
    let grid: Vec<bool> = (0..n * n).map(|k| cloudy(noise, ci - RADIUS + k % n, cj - RADIUS + k / n)).collect();
    let at = |i: i32, j: i32| (0..n).contains(&i) && (0..n).contains(&j) && grid[(j * n + i) as usize];
    let white = [1.0, 1.0, 1.0];
    let quad = |m: &mut Mesh, d: usize, lo: Vec3, hi: Vec3| {
        let dir = crate::block::DIRS[d];
        let base = m.vertices.len() as u32;
        for c in crate::mesh::FACES[d] {
            let p = lo + (hi - lo) * Vec3::from(c);
            m.vertices.push(Vertex { pos: p.to_array(), normal: [dir[0] as f32, dir[1] as f32, dir[2] as f32], color: white, sway: 0.0 });
        }
        m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    // Tops and bottoms as few large rectangles (see-through quads that meet show a seam where they overlap).
    let mut left = grid.clone();
    for j in 0..n {
        for i in 0..n {
            if !left[(j * n + i) as usize] {
                continue;
            }
            let mut w = 1;
            while i + w < n && left[(j * n + i + w) as usize] {
                w += 1;
            }
            let mut h = 1;
            while j + h < n && (0..w).all(|k| left[((j + h) * n + i + k) as usize]) {
                h += 1;
            }
            for jj in j..j + h {
                for ii in i..i + w {
                    left[(jj * n + ii) as usize] = false;
                }
            }
            let lo = Vec3::new(i as f32 * CELL, 0.0, j as f32 * CELL);
            let hi = Vec3::new((i + w) as f32 * CELL, THICK, (j + h) as f32 * CELL);
            quad(&mut m, 2, lo, hi);
            quad(&mut m, 3, lo, hi);
        }
    }
    // Sides only where cloud meets clear sky.
    for j in 0..n {
        for i in 0..n {
            if !at(i, j) {
                continue;
            }
            let lo = Vec3::new(i as f32 * CELL, 0.0, j as f32 * CELL);
            let hi = lo + Vec3::new(CELL, THICK, CELL);
            for d in [0usize, 1, 4, 5] {
                let dir = crate::block::DIRS[d];
                if !at(i + dir[0], j + dir[2]) {
                    quad(&mut m, d, lo, hi);
                }
            }
        }
    }
    m
}

/// Where the cloud field around `eye` is at scene time `t`: the cell the mesh is built around, and the world position of its corner.
pub fn place(eye: Vec3, t: f32) -> ((i32, i32), Vec3) {
    let drift = t * DRIFT;
    let (ci, cj) = (((eye.x - drift) / CELL).floor() as i32, (eye.z / CELL).floor() as i32);
    let corner = Vec3::new((ci - RADIUS) as f32 * CELL + drift, HEIGHT, (cj - RADIUS) as f32 * CELL);
    ((ci, cj), corner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn some_of_the_sky_is_cloudy_and_the_field_follows_the_player() {
        let noise = Noise::new(77);
        let cover = (0..4000).filter(|k| cloudy(&noise, k % 63, k / 63)).count() as f32 / 4000.0;
        assert!(cover > 0.2 && cover < 0.6, "{cover}");
        let m = mesh(&noise, 0, 0);
        assert!(!m.indices.is_empty());
        let ((a, _), c0) = place(Vec3::new(5.0, 70.0, 5.0), 0.0);
        let ((b, _), c1) = place(Vec3::new(5.0, 70.0, 5.0), 100.0);
        assert!((7..=8).contains(&(a - b)), "100 s of drift moves the field 90 m: {}", a - b);
        assert!((c1.x - c0.x).abs() < CELL, "the field stays around the player");
    }
}
