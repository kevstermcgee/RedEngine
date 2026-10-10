//! Split-screen foundations: where each local player's view goes on the window, how wide it should see, and which device drives which player.
//!
//! All pure arithmetic (nothing here touches a GPU, a window or a pad), so the rules a couch co-op game depends on are tested as numbers:
//! the viewports tile the window with no overlap and no gaps larger than the gutter, every viewport is the *same size* (so one renderer sized for one view
//! serves them all), a narrow view sees a little wider than a full-screen one so the picture is not a tunnel, and a pad that is missing or unplugged is a named
//! problem rather than a silent one. Plan and measurements: `docs/analysis/2026-10-04-split-screen-survey-and-plan.md`.

/// The most local players on one screen.
pub const MAX_LOCAL_PLAYERS: usize = 4;

/// What a guest (every local player after the first) cannot do, as `(topic, sentence)`. **One definition**: `describe multiplayer`, the notes `re2 --players N` prints, the
/// scripted `interact` diagnostic and the test that pins them all read this list, so the limits cannot be stated in one place and forgotten in another. A game that needs
/// one of these needs a different design (or one process per person, which is online play).
pub const GUEST_LIMITS: &[(&str, &str)] = &[
    (
        "props",
        "only player 1 can pick up, carry, throw or drop loose props (objects that are `movable`); guests walk, jump, swing and shoot, and rules see every player's body",
    ),
    ("flashlight", "the carried flashlight (`flashlight: true`) is player 1's"),
    ("online", "`--players N` is offline only: online play and the kart racer run one player per process (one process per person, hosted and joined as usual)"),
];

/// The limits in [`GUEST_LIMITS`] that this scene actually runs into, as sentences for someone about to play it with `--players` 2 to 4 (empty when it runs into none):
/// a game built around carrying a crate gets told before its second player finds out.
pub fn guest_limits_for(scene: &crate::schema::Scene) -> Vec<String> {
    let said = |topic: &str| GUEST_LIMITS.iter().find(|(t, _)| *t == topic).map_or("", |(_, s)| *s);
    let mut out = Vec::new();
    let props = crate::physics::loose_props(scene, None).len();
    if props > 0 {
        out.push(format!("this scene has {props} loose prop(s), and {}", said("props")));
    }
    if scene.flashlight {
        out.push(format!("this scene gives the player a flashlight, and {}", said("flashlight")));
    }
    out
}

/// A rectangle of the window, in pixels (origin top left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// Whether two rectangles share any pixel.
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    /// Width over height.
    pub fn aspect(&self) -> f32 {
        self.w as f32 / self.h.max(1) as f32
    }

    /// Whether a window point is inside.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// How the viewports sit on the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// One rectangle per player, in player order.
    pub views: Vec<Rect>,
    /// The size every view has (a viewport texture of this size serves any of them).
    pub size: (u32, u32),
    /// The columns and rows of the grid the views sit in.
    pub grid: (u32, u32),
}

/// Lays out `players` (1 to 4) on a `w` x `h` window with `gutter` pixels between views.
///
/// One player gets the whole window. Two sit side by side on a wide window and one above the other on a tall one. Three and four use a 2x2 grid (with three, the
/// fourth place is left empty rather than giving one player a different shape: every view is the same size, so nobody sees more or less than another).
/// Sizes are rounded down to even numbers so a half-resolution pass divides cleanly.
pub fn layout(players: usize, w: u32, h: u32, gutter: u32) -> Layout {
    let n = players.clamp(1, MAX_LOCAL_PLAYERS);
    let (cols, rows) = match n {
        1 => (1, 1),
        2 if w as f32 >= h as f32 * 1.2 => (2, 1),
        2 => (1, 2),
        _ => (2, 2),
    };
    let gx = gutter * (cols - 1);
    let gy = gutter * (rows - 1);
    let vw = (w.saturating_sub(gx) / cols) & !1;
    let vh = (h.saturating_sub(gy) / rows) & !1;
    // Centre the block so the rounding slack is split between the edges.
    let used_w = vw * cols + gx;
    let used_h = vh * rows + gy;
    let (ox, oy) = ((w - used_w.min(w)) / 2, (h - used_h.min(h)) / 2);
    let views = (0..n as u32).map(|i| Rect { x: ox + (i % cols) * (vw + gutter), y: oy + (i / cols) * (vh + gutter), w: vw, h: vh }).collect();
    Layout { views, size: (vw, vh), grid: (cols, rows) }
}

/// The vertical field of view (degrees) for a view of `view_aspect` when a full window of `window_aspect` would use `base_vfov_deg`.
///
/// A narrow view cut from a wide window would see a tunnel if the vertical field were kept: so the horizontal field shrinks only by the square root of the
/// aspect ratio, and the vertical field grows to make up for it. It never goes below the base or above 100 degrees, and a full-size view is unchanged.
pub fn vertical_fov(base_vfov_deg: f32, window_aspect: f32, view_aspect: f32) -> f32 {
    if view_aspect >= window_aspect * 0.999 {
        return base_vfov_deg;
    }
    let tan_h0 = (base_vfov_deg.to_radians() / 2.0).tan() * window_aspect;
    let tan_h = tan_h0 * (view_aspect / window_aspect).sqrt();
    let v = 2.0 * (tan_h / view_aspect).atan();
    v.to_degrees().clamp(base_vfov_deg, 100.0)
}

/// How far the streamed world reaches (metres) with `players` views on the screen: every view draws it, so a crowded screen sees a little less far (its views are
/// also smaller, so the difference is hard to spot) and a whole frame of four views stays near the triangle budget of one big one.
pub fn view_distance(players: usize) -> f32 {
    match players {
        0 | 1 => 270.0,
        2 => 230.0,
        _ => 190.0,
    }
}

/// What drives a local player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// The keyboard and mouse.
    KeyboardMouse,
    /// A gamepad, by the order they were connected in (0 is the first).
    Pad(usize),
}

/// Why players cannot be given devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortfall {
    /// Gamepads needed in addition to those connected.
    pub missing: usize,
}

impl std::fmt::Display for Shortfall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} more gamepad{} needed: connect {} and start again (player 1 uses the keyboard and mouse, every other player a gamepad)",
            self.missing,
            if self.missing == 1 { "" } else { "s" },
            if self.missing == 1 { "it" } else { "them" }
        )
    }
}

/// Gives each of `players` a device with `pads` gamepads connected. Player 1 takes the keyboard and mouse; every other player takes the next pad. With
/// `pads_only` player 1 takes a pad too (a couch with no keyboard). Too few pads is a [`Shortfall`] that says how many.
pub fn assign(players: usize, pads: usize, pads_only: bool) -> Result<Vec<Device>, Shortfall> {
    let n = players.clamp(1, MAX_LOCAL_PLAYERS);
    let pads_needed = if pads_only { n } else { n - 1 };
    if pads < pads_needed {
        return Err(Shortfall { missing: pads_needed - pads });
    }
    let mut next = 0;
    Ok((0..n)
        .map(|i| {
            if i == 0 && !pads_only {
                Device::KeyboardMouse
            } else {
                next += 1;
                Device::Pad(next - 1)
            }
        })
        .collect())
}

/// The players whose pad was unplugged: when pad `lost` disconnects, the game pauses and names them.
pub fn owners_of(devices: &[Device], lost: usize) -> Vec<usize> {
    devices.iter().enumerate().filter(|(_, d)| **d == Device::Pad(lost)).map(|(i, _)| i).collect()
}

/// How loud a guest's own sound (steps, jumps, swings) is next to the first player's: the speakers are shared, so everyone hears everyone, but four sets of footsteps at
/// full level are noise.
pub const GUEST_SOUND: f32 = 0.55;

/// How a sound made by a guest at `source` is heard: `(gain scale, pan)`. It is placed against the *other* players (the loudest of them hears it), so a guest running off
/// across the map fades and leans to their side; the first player's sounds are centred and full as ever.
pub fn guest_mix(others: &[crate::mixer::Listener], source: [f32; 3]) -> (f32, f32) {
    let (gain, pan) = crate::mixer::hear(others, source);
    (GUEST_SOUND + (1.0 - GUEST_SOUND) * gain, pan * 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guest_far_from_the_others_is_quieter_and_leans_to_their_side_but_never_inaudible() {
        use crate::mixer::Listener;
        let others = [Listener { pos: [0.0, 1.6, 0.0], yaw: 0.0 }];
        let (near, _) = guest_mix(&others, [0.0, 1.6, -1.0]);
        let (far, pan) = guest_mix(&others, [60.0, 1.6, 0.0]);
        assert!(near > far && far >= GUEST_SOUND && near <= 1.0, "{near} {far}");
        assert!(pan > 0.2 && pan <= 0.5, "to the right, and softened: {pan}");
        assert_eq!(guest_mix(&[], [3.0, 0.0, 3.0]), (1.0, 0.0), "nobody else to hear them: the sound is as it was");
    }

    #[test]
    fn views_tile_the_window_without_overlap_and_are_all_the_same_size() {
        for (w, h) in [(1920, 1080), (1280, 720), (1366, 768), (800, 600), (1080, 1920), (3840, 2160), (1001, 701)] {
            for n in 1..=4 {
                for gutter in [0, 4, 8] {
                    let l = layout(n, w, h, gutter);
                    assert_eq!(l.views.len(), n, "{n} players on {w}x{h}");
                    for (i, a) in l.views.iter().enumerate() {
                        assert_eq!((a.w, a.h), l.size, "all views are one size");
                        assert!(a.w % 2 == 0 && a.h % 2 == 0, "even sizes divide for half-resolution passes: {a:?}");
                        assert!(a.x + a.w <= w && a.y + a.h <= h, "{a:?} is inside {w}x{h}");
                        for b in &l.views[i + 1..] {
                            assert!(!a.overlaps(b), "{a:?} overlaps {b:?}");
                        }
                    }
                    // Nothing is wasted: the views cover nearly the whole window (all but rounding, gutters and, with three, the empty place).
                    let covered: u64 = l.views.iter().map(|r| r.w as u64 * r.h as u64).sum();
                    let places = (l.grid.0 * l.grid.1) as u64;
                    let wasted = (w as u64 * h as u64) as f64 - covered as f64 * places as f64 / n as f64;
                    assert!(wasted / ((w as u64 * h as u64) as f64) < 0.04 + gutter as f64 * 0.004, "{n} on {w}x{h} gutter {gutter} wastes {wasted}");
                }
            }
        }
    }

    #[test]
    fn the_shapes_are_the_familiar_ones() {
        let one = layout(1, 1920, 1080, 8);
        assert_eq!(one.views, vec![Rect { x: 0, y: 0, w: 1920, h: 1080 }]);
        let two = layout(2, 1920, 1080, 8);
        assert_eq!(two.grid, (2, 1));
        assert!(two.views[0].x < two.views[1].x && two.views[0].y == two.views[1].y, "side by side on a wide window");
        let tall = layout(2, 1080, 1920, 8);
        assert_eq!(tall.grid, (1, 2));
        assert!(tall.views[0].y < tall.views[1].y, "stacked on a tall one");
        let four = layout(4, 1920, 1080, 8);
        assert_eq!(four.grid, (2, 2));
        assert_eq!((four.views[0].x, four.views[0].y), (four.views[2].x, four.views[1].y), "player 1 is top left, 2 top right, 3 bottom left, 4 bottom right");
        assert!(
            four.views[1].x > four.views[0].x && four.views[2].y > four.views[0].y && four.views[3].x == four.views[1].x && four.views[3].y == four.views[2].y
        );
        let three = layout(3, 1920, 1080, 8);
        assert_eq!((three.views.len(), three.size), (3, four.size), "three players get the same views as four, with a place left empty");
        assert_eq!(layout(9, 1920, 1080, 0).views.len(), 4, "never more than four");
        assert_eq!(layout(0, 1920, 1080, 0).views.len(), 1);
    }

    #[test]
    fn a_narrow_view_sees_a_little_wider_but_never_absurdly() {
        let base = 70.0;
        let window = 16.0 / 9.0;
        assert_eq!(vertical_fov(base, window, window), base, "a full view is unchanged");
        let side_by_side = layout(2, 1920, 1080, 0).views[0].aspect();
        let narrow = vertical_fov(base, window, side_by_side);
        assert!(narrow > base && narrow < 100.0, "{narrow}");
        // The horizontal field shrinks, but by the square root of the aspect ratio, not all the way.
        let horizontal = |v: f32, a: f32| 2.0 * ((v.to_radians() / 2.0).tan() * a).atan().to_degrees();
        let (h_full, h_narrow) = (horizontal(base, window), horizontal(narrow, side_by_side));
        assert!(h_narrow < h_full && h_narrow > h_full * 0.75, "{h_full} -> {h_narrow}");
        // The narrower the view the wider the vertical field, up to the cap.
        let mut last = base;
        for a in [1.6, 1.2, 0.9, 0.6, 0.4] {
            let v = vertical_fov(base, window, a);
            assert!(v >= last && v <= 100.0, "{a}: {v}");
            last = v;
        }
        // A view wider than the window (an ultrawide split) is not squeezed.
        assert_eq!(vertical_fov(base, window, 3.0), base);
    }

    #[test]
    fn devices_go_keyboard_first_then_pads_in_order() {
        use Device::*;
        assert_eq!(assign(1, 0, false).unwrap(), vec![KeyboardMouse]);
        assert_eq!(assign(2, 1, false).unwrap(), vec![KeyboardMouse, Pad(0)]);
        assert_eq!(assign(4, 5, false).unwrap(), vec![KeyboardMouse, Pad(0), Pad(1), Pad(2)]);
        assert_eq!(assign(2, 2, true).unwrap(), vec![Pad(0), Pad(1)], "a couch with no keyboard");
        assert_eq!(assign(9, 9, false).unwrap().len(), 4);
    }

    #[test]
    fn too_few_pads_is_a_clear_shortfall_and_a_lost_pad_names_its_owner() {
        let e = assign(4, 1, false).unwrap_err();
        assert_eq!(e, Shortfall { missing: 2 });
        assert!(e.to_string().contains("2 more gamepads needed") && e.to_string().contains("keyboard"), "{e}");
        assert_eq!(assign(2, 0, false).unwrap_err().to_string().split(':').next().unwrap(), "1 more gamepad needed");
        assert_eq!(assign(2, 1, true).unwrap_err(), Shortfall { missing: 1 });
        let devices = assign(3, 2, false).unwrap();
        assert_eq!(owners_of(&devices, 1), vec![2]);
        assert!(owners_of(&devices, 7).is_empty());
    }

    fn scene(extra: &str) -> crate::schema::Scene {
        let text = format!(
            r#"{{"camera": {{"position": [0, 2, 5], "target": [0, 1, 0]}}, "objects": [{{"id": "floor", "type": "plane", "size": [20, 20]}}{extra}]}}"#
        );
        crate::schema::parse_scene(&text).unwrap_or_else(|e| panic!("{e:?}"))
    }

    #[test]
    fn a_scene_is_told_which_guest_limits_it_runs_into() {
        assert!(guest_limits_for(&scene("")).is_empty(), "nothing to carry, nothing to warn about");
        let crate_scene = scene(r#", {"id": "box1", "type": "prop", "prop": "crate", "position": [0, 0, 0]}"#);
        let notes = guest_limits_for(&crate_scene);
        assert!(notes.len() == 1 && notes[0].contains("loose prop") && notes[0].contains("only player 1"), "{notes:?}");
        let mut lit = scene("");
        lit.flashlight = true;
        assert!(guest_limits_for(&lit).iter().any(|n| n.contains("flashlight") && n.contains("player 1")));
    }

    #[test]
    fn every_limit_names_who_can_and_what() {
        for (topic, sentence) in GUEST_LIMITS {
            assert!(!topic.is_empty() && sentence.len() > 30, "{topic}: {sentence}");
        }
        assert!(GUEST_LIMITS.iter().any(|(t, s)| *t == "props" && s.contains("only player 1")), "the prop limit says player 1 only");
    }
}
