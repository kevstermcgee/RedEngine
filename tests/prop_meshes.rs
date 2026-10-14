//! The prop catalogue's meshes as the renderer builds them (moved from `props`' own tests when `props` moved into the core: the mesh builder is the
//! renderer's, which needs `gfx`).
#![cfg(feature = "gfx")]

use red_engine2::mesh::Mesh;
use red_engine2::props::*;
use red_engine2::render::build_prim_mesh;

fn assert_valid(m: &Mesh) {
    assert!(!m.vertices.is_empty());
    assert!(!m.indices.is_empty());
    assert_eq!(m.indices.len() % 3, 0);
    for &i in &m.indices {
        assert!((i as usize) < m.vertices.len());
    }
}

#[test]
fn every_prop_has_parts_with_valid_meshes() {
    for kind in PropKind::ALL {
        let parts = prop_parts(kind);
        assert!(!parts.is_empty(), "{:?} has no parts", kind);
        for part in &parts {
            assert_valid(&build_prim_mesh(&part.shape));
            assert!((part.metallic_delta + part.roughness_delta).is_finite());
        }
    }
}

#[test]
fn collision_boxes_are_sane() {
    for kind in PropKind::ALL {
        if let Some((min, max)) = collision_box(kind) {
            assert!(min.x < max.x && min.y < max.y && min.z < max.z, "{:?} has a degenerate collider", kind);
        }
    }
    assert!(collision_box(PropKind::FlowerPatch).is_none(), "flowers must be walk-through");
    let (min, max) = collision_box(PropKind::TreeOak).unwrap();
    assert!(max.x - min.x < 0.6, "only a tree's trunk should block, not its canopy");
}

#[test]
fn tables_leave_room_underneath_for_a_small_body_but_are_solid_for_a_tall_one() {
    for kind in [PropKind::DiningTable, PropKind::Desk, PropKind::CoffeeTable, PropKind::Bench, PropKind::PicnicTable] {
        let boxes = game_collision_boxes(kind);
        assert!(boxes.len() > 1, "{kind:?} blocks part by part");
        // Something reaches the floor (a leg) and something is a raised top with clear space below it.
        assert!(boxes.iter().any(|(lo, _)| lo.y < 0.05), "{kind:?} has legs");
        let lowest_top = boxes.iter().filter(|(lo, _)| lo.y > 0.05).map(|(lo, _)| lo.y).fold(f32::INFINITY, f32::min);
        assert!(lowest_top > 0.26, "{kind:?}: the underside is {lowest_top} m up, room for a 0.25 m rat");
    }
    assert_eq!(game_collision_boxes(PropKind::Crate).len(), 1);
    assert!(game_collision_boxes(PropKind::Rug).is_empty());
}

#[test]
fn props_rest_on_the_floor() {
    // Every prop's lowest point sits at y=0 (so `position.y` is the floor it stands on).
    let bad: Vec<String> = PropKind::ALL
        .iter()
        .filter_map(|&kind| {
            let (min, _) = local_bounds(kind);
            (!(min.y > -0.05 && min.y < 0.06)).then(|| format!("{}(min y={:.3})", kind.name(), min.y))
        })
        .collect();
    assert!(bad.is_empty(), "props whose origin is not at their base: {}", bad.join(", "));
}

#[test]
fn every_kind_round_trips_through_its_name() {
    for kind in PropKind::ALL {
        assert_eq!(PropKind::from_name(kind.name()), Some(kind));
    }
}
