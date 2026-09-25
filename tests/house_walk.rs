//! Walks real routes through `examples/house.json` with the game's own per-tick physics, so a
//! layout edit that seals a door, blocks a staircase, or leaves a floor unreachable fails here
//! instead of being discovered by a player.

use glam::Vec2;
use red_engine2::tools::walk::{format_walk, walk};
use red_engine2::tools::world::MapWorld;
use std::path::Path;

fn house() -> MapWorld {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/house.json");
    MapWorld::load(&path).unwrap_or_else(|e| panic!("house.json: {e:?}"))
}

fn route(points: &[(f32, f32)]) -> Vec<Vec2> {
    points.iter().map(|&(x, z)| Vec2::new(x, z)).collect()
}

fn assert_route(name: &str, points: &[(f32, f32)], final_y: f32) {
    let w = house();
    let path = route(points);
    let steps = walk(&w, w.spawn, &path);
    let report = format_walk(&steps, path.len());
    assert!(steps.len() == path.len() && steps.iter().all(|s| s.reached), "route '{name}' failed:\n{report}");
    let last = steps.last().unwrap();
    assert!((last.foot_y - final_y).abs() < 0.1, "route '{name}' should end at floor y={final_y}, got {:.2}:\n{report}", last.foot_y);
}

#[test]
fn front_door_to_every_ground_floor_room() {
    // Through the front door into the hall, then each room via its own doorway.
    assert_route("living room", &[(0.0, -8.0), (0.0, -1.0), (0.0, 0.8), (-0.5, 1.25), (-2.6, 1.25), (-4.3, 2.0)], 0.0);
    assert_route("kitchen", &[(0.0, -8.0), (0.0, 0.8), (0.6, 3.0), (2.6, 3.0), (4.0, 5.0)], 0.0);
    assert_route("dining room via the hall", &[(0.0, -8.0), (0.0, 0.8), (0.6, 8.0), (0.6, 9.0), (2.6, 9.0), (3.0, 10.5)], 0.0);
    assert_route("dining room via the kitchen arch", &[(0.0, -8.0), (0.0, 0.8), (0.6, 3.0), (2.6, 4.4), (4.25, 5.0), (4.25, 6.6), (6.2, 6.8), (6.2, 7.6)], 0.0);
    assert_route("study", &[(0.0, -8.0), (0.0, 0.8), (0.7, 8.0), (0.6, 9.5), (-2.6, 9.5), (-4.0, 9.0)], 0.0);
    assert_route("powder room", &[(0.0, -8.0), (0.0, 0.8), (0.6, 8.0), (2.6, 9.0), (5.75, 9.2), (5.75, 10.4)], 0.0);
}

#[test]
fn stairs_lead_up_to_every_bedroom_and_the_bathroom() {
    // Up the staircase: enter at its foot, walk its length, step off onto the landing.
    let up = [(0.0, -8.0), (0.0, 0.8), (-0.8, 2.0), (-0.8, 4.75), (-0.8, 7.6)];
    let mut master = up.to_vec();
    master.extend([(0.6, 8.0), (0.6, 4.0), (0.6, 1.25), (-1.0, 1.25), (-2.6, 1.25), (-4.0, 2.5)]);
    assert_route("master bedroom", &master, 3.0);
    let mut bed2 = up.to_vec();
    bed2.extend([(0.6, 8.0), (0.6, 4.0), (2.6, 4.0), (4.0, 3.0)]);
    assert_route("bedroom 2", &bed2, 3.0);
    let mut bed3 = up.to_vec();
    bed3.extend([(-0.8, 9.5), (-2.6, 9.5), (-4.0, 9.0)]);
    assert_route("bedroom 3", &bed3, 3.0);
    let mut bath = up.to_vec();
    bath.extend([(0.6, 8.0), (0.6, 9.5), (2.6, 9.5), (4.0, 9.0)]);
    assert_route("upstairs bathroom", &bath, 3.0);
}

#[test]
fn back_door_patio_and_shed() {
    assert_route("back yard and shed", &[(0.0, -8.0), (0.0, 0.8), (0.0, 11.0), (0.0, 13.0), (5.0, 16.0), (9.0, 18.0), (9.0, 21.0)], 0.0);
}

#[test]
fn the_player_cannot_walk_out_of_the_lot() {
    // Straight at the perimeter fence from inside: must be stopped by it, not pass through.
    let w = house();
    let steps = walk(&w, Vec2::new(0.0, -8.0), &route(&[(0.0, -14.0)]));
    assert!(!steps[0].reached && steps[0].pos.y > -10.0, "the front fence must stop the player: {:?}", steps[0]);
    let steps = walk(&w, Vec2::new(0.0, 20.0), &route(&[(0.0, 30.0)]));
    assert!(!steps[0].reached && steps[0].pos.y < 26.0, "the back fence must stop the player: {:?}", steps[0]);
}

#[test]
fn the_stairs_cannot_be_entered_from_the_side_or_the_tall_end() {
    let w = house();
    // From the hall lane straight west into the stair flank.
    let steps = walk(&w, Vec2::new(0.6, 4.0), &route(&[(-0.8, 4.0)]));
    assert!(steps[0].foot_y < 0.3 || !steps[0].reached, "walked into the stair volume from the side: {:?}", steps[0]);
    // From the south hall straight north into the tall end of the stairs.
    let steps = walk(&w, Vec2::new(-0.8, 9.0), &route(&[(-0.8, 6.0)]));
    assert!(steps[0].foot_y < 0.3 && !steps[0].reached, "walked under the stairs from the tall end: {:?}", steps[0]);
}
