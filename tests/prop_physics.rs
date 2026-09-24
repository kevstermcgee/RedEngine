//! The four real maps under the prop physics: loose props exist, and an idle map stays put (nothing
//! explodes, sinks or drifts when the physics world is built and stepped with nobody touching it).

use glam::Vec3;
use red_engine2::physics::PropWorld;
use std::path::Path;
use std::time::Instant;

fn load(name: &str) -> red_engine2::schema::Scene {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(name);
    red_engine2::load_scene(&path).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

#[test]
fn every_map_has_loose_props_and_an_idle_map_stays_put() {
    for (name, min_props) in [("house.json", 15), ("school.json", 40), ("office.json", 20), ("store.json", 30)] {
        let mut scene = load(name);
        let mut w = PropWorld::new(&scene, None);
        let n = w.props().len();
        eprintln!("{name}: {n} loose props of {} objects", scene.objects.len());
        assert!(n >= min_props, "{name}: only {n} loose props");
        eprintln!("{name}: awake right after build: {}", w.awake_count());
        // The load-time settle must not rearrange the map: every prop stays within a few cm of where it was authored.
        let mut worst = (0.0f32, String::new());
        for i in 0..n {
            let o = &scene.objects[w.props()[i].object_index];
            let d = (w.prop_pose(i).w_axis.truncate() - o.position.sample(0.0)).length();
            if d > worst.0 {
                worst = (d, o.id.clone());
            }
        }
        eprintln!("{name}: worst settle displacement {:.3} m ({})", worst.0, worst.1);
        assert!(worst.0 < 0.25, "{name}: {} moved {} m while settling at load", worst.1, worst.0);
        let before: Vec<Vec3> = (0..n).map(|i| w.prop_pose(i).w_axis.truncate()).collect();
        // The player stands far away so nothing is touched.
        w.set_player(Vec3::new(500.0, 0.0, 500.0), 0.35, 1.75);
        let t = Instant::now();
        for k in 0..180 {
            w.step();
            if k < 3 { eprintln!("{name}: awake after step {k}: {}", w.awake_count()); }
        }
        let ms = t.elapsed().as_secs_f32() * 1000.0 / 180.0;
        w.sync_scene(&mut scene);
        for i in 0..n {
            let d = (w.prop_pose(i).w_axis.truncate() - before[i]).length();
            let ok = d < 0.02;
            if !ok { let g = scene.objects[w.props()[i].object_index].id.clone(); eprintln!("drifted: {g} {d} shape {:?}", w.props()[i].shape); }
            assert!(d < 0.02, "{name}: prop {} ({}) drifted {d} m by itself", i, scene.objects[w.props()[i].object_index].id);
        }
        let awake: Vec<&str> = (0..n).filter(|&i| !w.is_asleep(i)).map(|i| scene.objects[w.props()[i].object_index].id.as_str()).collect();
        assert_eq!(w.awake_count(), 0, "{name}: awake: {awake:?}");
        assert_eq!(w.awake_count(), 0, "{name}: an idle map should be asleep");
        assert!(ms < 4.0, "{name}: {ms:.2} ms per physics step is too slow for a 60 fps budget");
    }
}

#[test]
fn lifting_a_side_table_drops_the_globe_that_was_on_it_and_everything_comes_to_rest() {
    let mut scene = load("school.json");
    let mut w = PropWorld::new(&scene, None);
    let idx = |w: &PropWorld, scene: &red_engine2::schema::Scene, id: &str| {
        w.props().iter().position(|p| scene.objects[p.object_index].id == id).unwrap_or_else(|| panic!("{id} is not loose"))
    };
    let (table, globe) = (idx(&w, &scene, "nook_side"), idx(&w, &scene, "globe_lib"));
    let y0 = w.prop_pose(globe).w_axis.y;
    assert!(y0 > 0.5, "the globe starts on the table ({y0})");
    w.set_player(Vec3::new(21.6, 0.0, 6.0), 0.35, 1.75);
    w.pick_up(table);
    w.set_held_pose(glam::Mat4::from_translation(Vec3::new(21.6, 1.2, 5.0)));
    let mut worst_ms = 0.0f32;
    for _ in 0..240 {
        let t = Instant::now();
        w.step();
        worst_ms = worst_ms.max(t.elapsed().as_secs_f32() * 1000.0);
    }
    let gy = w.prop_pose(globe).w_axis.y;
    assert!(gy < 0.4 && gy < y0 - 0.2, "the globe fell to the floor when its table was lifted away (y = {gy})");
    // Put the table down on the same spot: nothing may end up inside the floor or fly off.
    w.drop_held(Vec3::ZERO);
    for _ in 0..600 {
        w.step();
    }
    w.sync_scene(&mut scene);
    for i in 0..w.props().len() {
        let p = w.prop_pose(i).w_axis;
        assert!(p.is_finite() && p.y > -0.2 && p.y < 3.0, "{} ended up at {p:?}", scene.objects[w.props()[i].object_index].id);
    }
    assert_eq!(w.awake_count(), 0, "everything is asleep again after a few seconds");
    assert!(worst_ms < 8.0, "worst step {worst_ms:.2} ms");
}
