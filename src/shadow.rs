//! Cascaded sun shadows: where each shadow map sits in the atlas, what it covers, and the matrix that draws into it. Pure arithmetic, tested without a GPU.
//!
//! One map cannot be both sharp under the player's feet and long enough to reach the trees at the edge of an open world. So a world whose shadow follows the camera
//! gets three maps in one atlas texture, each a square box under the camera:
//!
//! | cascade | covers | draws | texel (radius 60) |
//! |---|---|---|---|
//! | 0 near | `radius / 6` | everything that casts, flowers and grass included | 2 cm |
//! | 1 mid | `radius` | trees, shrubs, objects | 6 cm |
//! | 2 far | `radius * 4` | trees, shrubs, objects | 47 cm |
//!
//! A pixel uses the smallest cascade that holds it and blends into the next one toward its edge, and the last cascade fades out to "lit", so no seam or pop shows. A
//! scene whose shadow does not follow the camera (a room, a yard) keeps its single map, laid out exactly as it always was.

use glam::{Mat4, Vec3};

/// The most cascades.
pub const MAX_CASCADES: usize = 3;
/// The atlas texture all cascades are drawn into, in pixels.
pub const ATLAS: (u32, u32) = (3072, 2048);
/// The edge of the main (mid or only) cascade, in pixels.
pub const MAIN_SIZE: u32 = 2048;

/// One shadow map: a rectangle of the atlas and the box of the world it sees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cascade {
    /// World to the light's clip space.
    pub view_proj: Mat4,
    /// `[x, y, width, height]` in atlas pixels.
    pub rect: [u32; 4],
    /// Half the width of the box on the ground, metres.
    pub radius: f32,
    /// Whether flowers and grass are drawn into this one (only the nearest).
    pub flora: bool,
}

impl Cascade {
    /// Metres on the ground per shadow-map pixel.
    pub fn texel(&self) -> f32 {
        2.0 * self.radius / self.rect[2] as f32
    }

    /// Depth units (0 to 1 across the light's near-to-far range) per metre along the light.
    pub fn depth_per_metre(&self) -> f32 {
        1.0 / (self.radius * 3.5)
    }

    /// The rectangle as atlas texture coordinates: offset and scale.
    pub fn uv_rect(&self) -> [f32; 4] {
        let (w, h) = (ATLAS.0 as f32, ATLAS.1 as f32);
        [self.rect[0] as f32 / w, self.rect[1] as f32 / h, self.rect[2] as f32 / w, self.rect[3] as f32 / h]
    }
}

/// The matrix of a map for a light shining along `d`, a box of half-width `radius` centred on `center` (or under the camera, snapped to whole texels so the shadows do not
/// crawl as it moves, when `follow`).
pub fn matrix(d: Vec3, radius: f32, center: Vec3, follow: bool, camera: Vec3, size_px: u32) -> Mat4 {
    let r = radius.max(0.5);
    let mut center = center;
    if follow {
        let texel = (2.0 * r) / size_px as f32;
        center = Vec3::new((camera.x / texel).round() * texel, center.y, (camera.z / texel).round() * texel);
    }
    let light_pos = center - d * (r * 1.6);
    let up = if d.y.abs() > 0.98 { Vec3::Z } else { Vec3::Y };
    let view_l = glam::camera::rh::view::look_at_mat4(light_pos, center, up);
    let proj_l = glam::camera::rh::proj::directx::orthographic(-r, r, -r, r, 0.05, r * 3.5);
    proj_l * view_l
}

/// The cascades for a light shining along `d`. A light that follows the camera gets three, around the authored `radius`; any other gets its one map.
pub fn cascades(d: Vec3, radius: f32, center: Vec3, follow: bool, camera: Vec3) -> Vec<Cascade> {
    if !follow {
        return vec![Cascade {
            view_proj: matrix(d, radius, center, false, camera, MAIN_SIZE),
            rect: [0, 0, MAIN_SIZE, MAIN_SIZE],
            radius: radius.max(0.5),
            flora: false,
        }];
    }
    let mid = radius.max(8.0);
    let near = (mid / 6.0).max(6.0);
    let far = (mid * 4.0).min(320.0).max(mid * 1.5);
    let make = |r: f32, rect: [u32; 4], flora: bool| Cascade { view_proj: matrix(d, r, center, true, camera, rect[2]), rect, radius: r, flora };
    vec![make(near, [MAIN_SIZE, 0, 1024, 1024], true), make(mid, [0, 0, MAIN_SIZE, MAIN_SIZE], false), make(far, [MAIN_SIZE, 1024, 1024, 1024], false)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun() -> Vec3 {
        Vec3::new(-0.4, -1.0, -0.3).normalize()
    }

    #[test]
    fn a_fixed_light_keeps_its_single_map_and_a_following_one_gets_three_nested_maps() {
        let one = cascades(sun(), 15.0, Vec3::ZERO, false, Vec3::ZERO);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].rect, [0, 0, MAIN_SIZE, MAIN_SIZE]);
        let three = cascades(sun(), 60.0, Vec3::ZERO, true, Vec3::new(5.0, 0.0, 9.0));
        assert_eq!(three.len(), 3);
        assert!(three[0].radius < three[1].radius && three[1].radius < three[2].radius);
        assert!(three[0].flora && !three[1].flora && !three[2].flora, "only the nearest map carries the small plants");
        assert_eq!(three[1].radius, 60.0, "the authored radius is the middle map");
    }

    #[test]
    fn the_maps_sit_inside_the_atlas_and_never_overlap() {
        let c = cascades(sun(), 60.0, Vec3::ZERO, true, Vec3::ZERO);
        for a in &c {
            assert!(a.rect[0] + a.rect[2] <= ATLAS.0 && a.rect[1] + a.rect[3] <= ATLAS.1, "{:?}", a.rect);
        }
        for (i, a) in c.iter().enumerate() {
            for b in &c[i + 1..] {
                let apart = a.rect[0] + a.rect[2] <= b.rect[0]
                    || b.rect[0] + b.rect[2] <= a.rect[0]
                    || a.rect[1] + a.rect[3] <= b.rect[1]
                    || b.rect[1] + b.rect[3] <= a.rect[1];
                assert!(apart, "{:?} overlaps {:?}", a.rect, b.rect);
            }
        }
    }

    #[test]
    fn each_following_map_moves_in_whole_texels_so_shadows_do_not_crawl() {
        let at = |x: f32| cascades(sun(), 60.0, Vec3::ZERO, true, Vec3::new(x, 0.0, 0.0));
        for k in 0..3 {
            // A step of less than half a texel changes nothing at all; a step of several texels moves the map.
            let t = at(0.0)[k].texel();
            assert_eq!(at(0.0)[k].view_proj, at(t * 0.2)[k].view_proj, "cascade {k} moved by a fraction of a texel");
            assert_ne!(at(0.0)[k].view_proj, at(t * 3.0)[k].view_proj, "cascade {k} did not follow the camera");
        }
    }

    #[test]
    fn a_point_near_the_camera_is_inside_every_map_and_the_sharpest_is_the_nearest() {
        let cam = Vec3::new(30.0, 0.0, -20.0);
        let c = cascades(sun(), 60.0, Vec3::ZERO, true, cam);
        for a in &c {
            let p = a.view_proj * (cam + Vec3::new(1.0, 0.0, 1.0)).extend(1.0);
            let ndc = p.truncate() / p.w;
            assert!(ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0 && (0.0..=1.0).contains(&ndc.z), "{ndc:?}");
        }
        assert!(c[0].texel() < c[1].texel() && c[1].texel() < c[2].texel());
        assert!(c[0].texel() < 0.04, "the near map resolves a few centimetres: {}", c[0].texel());
    }

    #[test]
    fn uv_rects_tile_the_atlas_in_texture_coordinates() {
        let c = cascades(sun(), 60.0, Vec3::ZERO, true, Vec3::ZERO);
        let r = c[1].uv_rect();
        assert_eq!(r, [0.0, 0.0, MAIN_SIZE as f32 / ATLAS.0 as f32, 1.0]);
        let n = c[0].uv_rect();
        assert!((n[0] - MAIN_SIZE as f32 / ATLAS.0 as f32).abs() < 1e-6 && n[1] == 0.0);
    }
}
