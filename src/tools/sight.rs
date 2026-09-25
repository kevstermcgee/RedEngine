//! `red_engine2 ray`: line of sight between two points, against the real shapes of the map (not bounding boxes).
//!
//! For a hide-and-seek game "can the seeker see the hiding spot from the spawn?" is a design question worth a
//! command: it answers with the first thing in the way (id, kind, distance, point) or "clear". It uses the same
//! exact-shape ray test as the bat and revolver ([`crate::hit`]), so the answer matches what the game will do.

use super::world::MapWorld;
use crate::hit::{collect_hit_shapes_where, raycast_shapes};
use glam::Vec3;

/// The result of a line-of-sight query.
#[derive(Debug, Clone)]
pub struct Sight {
    /// Distance between the two points, m.
    pub length: f32,
    /// The first object in the way, if any: `(id, kind, distance along the ray, world point)`.
    pub blocker: Option<(String, String, f32, Vec3)>,
}

impl Sight {
    /// Whether nothing stands between the points.
    pub fn clear(&self) -> bool {
        self.blocker.is_none()
    }

    /// One readable line.
    pub fn render(&self, from: Vec3, to: Vec3) -> String {
        match &self.blocker {
            None => format!(
                "clear: nothing between ({:.2}, {:.2}, {:.2}) and ({:.2}, {:.2}, {:.2}) ({:.2} m)",
                from.x, from.y, from.z, to.x, to.y, to.z, self.length
            ),
            Some((id, kind, d, p)) => format!("blocked by '{id}' [{kind}] at {d:.2} m of {:.2} m, point ({:.2}, {:.2}, {:.2})", self.length, p.x, p.y, p.z),
        }
    }
}

/// Casts a ray from `from` to `to`. `skip` names objects to ignore (e.g. the thing standing at one end, or a floor plane).
pub fn cast(world: &MapWorld, from: Vec3, to: Vec3, skip: &[String]) -> Sight {
    let d = to - from;
    let length = d.length();
    if length < 1e-4 {
        return Sight { length, blocker: None };
    }
    let dir = d / length;
    let shapes = collect_hit_shapes_where(&world.scene, |i| !skip.iter().any(|s| world.scene.objects[i].id == *s));
    let blocker = raycast_shapes(from, dir, length, &shapes).map(|h| {
        let o = &world.scene.objects[h.object_index];
        let kind = world.items.iter().find(|it| it.top_id == o.id).map(|it| it.kind.label()).unwrap_or_else(|| "object".into());
        (o.id.clone(), kind, h.distance, from + dir * h.distance)
    });
    Sight { length, blocker }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn world() -> MapWorld {
        MapWorld::from_text(
            r##"{"camera":{"position":[0,1.7,-6]},"objects":[
              {"id":"floor","type":"plane","size":[20,20],"position":[0,0.01,0]},
              {"id":"wall","type":"wall","from":[-4,0],"to":[4,0],"height":2.8,"openings":[{"at":4.0,"width":1.0}]}]}"##,
            Path::new("t.json"),
        )
        .unwrap()
    }

    #[test]
    fn a_wall_blocks_sight_and_a_doorway_does_not() {
        let w = world();
        let eye = Vec3::new(-2.0, 1.6, -5.0);
        let s = cast(&w, eye, Vec3::new(-2.0, 1.6, 5.0), &[]);
        assert!(!s.clear() && s.blocker.as_ref().unwrap().0 == "wall", "{s:?}");
        assert!((s.blocker.as_ref().unwrap().2 - 4.9).abs() < 0.2, "{s:?}");
        // Straight through the doorway (centre at x = 0) is clear.
        assert!(cast(&w, Vec3::new(0.0, 1.6, -5.0), Vec3::new(0.0, 1.6, 5.0), &[]).clear());
        // Ignoring the wall makes the blocked line clear.
        assert!(cast(&w, eye, Vec3::new(-2.0, 1.6, 5.0), &["wall".to_string()]).clear());
        assert!(s.render(eye, Vec3::new(-2.0, 1.6, 5.0)).starts_with("blocked by 'wall'"));
    }
}
